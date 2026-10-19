import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:path_provider/path_provider.dart';
import 'package:share_plus/share_plus.dart';

import 'calendar.dart';
import 'connection.dart';
import 'model.dart';
import 'src/rust/api/poll.dart' as rust;
import 'storage.dart';

/// A slot (`2026-10-30T18:00+02:00/120`) in this phone's time zone:
/// `Fri 30 Oct, 18:00–20:00`.
String prettySlot(String slot) {
  final times = rust.slotTimes(slot: slot);
  if (times == null) return slot;
  final start = DateTime.fromMillisecondsSinceEpoch(times.startMillis);
  final end = start.add(Duration(minutes: times.minutes));
  final hm = DateFormat('HH:mm');
  return '${DateFormat('EEE d MMM').format(start)}, ${hm.format(start)}–${hm.format(end)}';
}

/// `90` → `1 h 30 min`.
String prettyDuration(int minutes) => switch ((minutes ~/ 60, minutes % 60)) {
      (0, final m) => '$m min',
      (final h, 0) => '$h h',
      (final h, final m) => '$h h $m min',
    };

/// Durations offered when suggesting a slot, in minutes.
const durations = [30, 60, 90, 120, 150, 180, 240, 300, 360, 480];

/// One poll, mirroring the web page: invite & times, vote, result.
class PollScreen extends StatefulWidget {
  const PollScreen({super.key, required this.invite});

  final Invite invite;

  @override
  State<PollScreen> createState() => _PollScreenState();
}

class _PollScreenState extends State<PollScreen> {
  late final PollConnection _conn = PollConnection(widget.invite);
  late final AppLifecycleListener _lifecycle = AppLifecycleListener(
    onHide: _conn.pause,
    onShow: _conn.resume,
  );

  /// Finished rounds, verified on this phone. Recomputed when one finishes.
  List<rust.OutcomeOut> _outcomes = [];
  int _outcomesFor = -1;

  @override
  void initState() {
    super.initState();
    _conn.addListener(_changed);
    _conn.start();
    _lifecycle;
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    _conn.removeListener(_changed);
    _conn.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    setState(() {});
    final view = _conn.view;
    if (view == null) return;
    final tallied = view.rounds.where((r) => r.hasTally).length;
    if (tallied != _outcomesFor) {
      _outcomesFor = tallied;
      _refreshOutcomes(view);
    }
  }

  Future<void> _refreshOutcomes(PollView view) async {
    final receipts = await Keys.receipts(widget.invite, view.rounds.length);
    final outcomes = await rust.outcomes(viewJson: view.raw, receipts: receipts);
    if (mounted) setState(() => _outcomes = outcomes);
  }

  @override
  Widget build(BuildContext context) {
    final view = _conn.view;
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(
        title: Text(view?.title ?? widget.invite.title ?? 'Poll'),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 16),
            child: Row(children: [
              Icon(Icons.circle, size: 10, color: _conn.connected ? Colors.green : theme.disabledColor),
              const SizedBox(width: 6),
              Text(_conn.gone
                  ? 'Offline'
                  : view == null
                      ? 'Connecting'
                      : "You're ${view.mine.name}"),
            ]),
          ),
        ],
      ),
      body: ListenableBuilder(
        listenable: _conn,
        builder: (context, _) => _body(context, view),
      ),
    );
  }

  Widget _body(BuildContext context, PollView? view) {
    if (view == null) {
      if (_conn.gone) {
        return const _Message(
          title: 'This poll no longer exists',
          body: "The server doesn't know this poll or invite link. Polls are only kept in memory, "
              'so they are lost whenever the server restarts or is updated.',
        );
      }
      return const Center(child: CircularProgressIndicator());
    }
    final s = _Ctx(conn: _conn, view: view, invite: widget.invite);
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        if (_conn.gone)
          const _Banner(
            "The server no longer has this poll (it restarted or was updated), so this page can't "
            'change anymore. What you see is the last state.',
          ),
        if (_conn.error != null)
          _Banner(_conn.error!, onDismiss: _conn.clearError),
        _Steps(view.phase),
        const SizedBox(height: 8),
        ...switch (view.phase) {
          Phase.inviting => _inviting(s),
          Phase.gathering => _gathering(s),
          Phase.keyCeremony => [_KeyCeremony(s)],
          Phase.voting => [..._finishedRounds(s, open: false), _Voting(s, key: ValueKey(view.rounds.length))],
          Phase.tallying => [..._finishedRounds(s, open: false), _Tallying(s)],
          Phase.done => [_ResultSummary(s, _outcomes), ..._finishedRounds(s, open: _outcomes.length == 1)],
          Phase.failed => [_Message(title: 'Something went wrong', body: view.failure ?? '')],
        },
        _People(s),
      ],
    );
  }

  static const _keyHint = _Hint(
    "Your part of the poll's key is kept on this phone. Opening your link somewhere else moves "
    "it there, which is fine until voting opens. After that, this phone is the one that can "
    'help reveal the result.',
  );

  /// The first step: the creator invites people, then moves everyone on to
  /// suggesting times. Invitees who open their link meanwhile join and wait.
  List<Widget> _inviting(_Ctx s) {
    final creator = s.view.participants.firstWhere((p) => p.creator).name;
    if (!s.view.isCreator) {
      return [
        _Card(
          title: '$creator is inviting people',
          child: const Text('Next, everyone suggests times that could work. This screen moves on by itself.'),
        ),
        _keyHint,
      ];
    }
    final joined = s.view.participants.where((p) => !p.creator && p.registered).length;
    final opened = switch (joined) {
      0 => 'Nobody has opened their link yet',
      1 => '1 person has opened their link',
      _ => '$joined people have opened their links',
    };
    return [
      _Invites(s),
      _Card(
        title: 'Done inviting?',
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('$opened. Next, everyone suggests times that could work; you can still invite people then.'),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: () => s.conn.send('FinishInviting'),
              child: const Text('Next: suggest times'),
            ),
          ],
        ),
      ),
      _keyHint,
    ];
  }

  List<Widget> _gathering(_Ctx s) => [
        _Dates(s),
        _keyHint,
        if (s.view.isCreator) ...[
          _Start(s),
          // Someone usually gets forgotten.
          Card(
            margin: const EdgeInsets.only(bottom: 12),
            child: ExpansionTile(
              title: const Text('Invite more people'),
              childrenPadding: EdgeInsets.zero,
              children: [_Invites(s)],
            ),
          ),
        ] else
          _Card(child: Text(
            'Waiting for ${s.view.participants.firstWhere((p) => p.creator).name} to lock the times '
            'and open voting.',
          )),
      ];

  List<Widget> _finishedRounds(_Ctx s, {required bool open}) => [
        for (final (i, o) in _outcomes.indexed) _RoundDetails(s, o, round: i + 1, open: open),
      ];
}

/// What every section needs.
class _Ctx {
  _Ctx({required this.conn, required this.view, required this.invite});
  final PollConnection conn;
  final PollView view;
  final Invite invite;
}

class _Card extends StatelessWidget {
  const _Card({required this.child, this.title});
  final Widget child;
  final String? title;

  @override
  Widget build(BuildContext context) => Card(
        margin: const EdgeInsets.only(bottom: 12),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (title != null) ...[
                Text(title!, style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 12),
              ],
              child,
            ],
          ),
        ),
      );
}

class _Hint extends StatelessWidget {
  const _Hint(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 0, 4, 12),
      child: Text(
        text,
        style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }
}

class _Banner extends StatelessWidget {
  const _Banner(this.text, {this.onDismiss});
  final String text;
  final VoidCallback? onDismiss;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Card(
      color: scheme.errorContainer,
      margin: const EdgeInsets.only(bottom: 12),
      child: ListTile(
        title: Text(text, style: TextStyle(color: scheme.onErrorContainer)),
        trailing: onDismiss == null
            ? null
            : IconButton(icon: const Icon(Icons.close), onPressed: onDismiss),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.title, required this.body});
  final String title;
  final String body;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.all(16),
        child: _Card(title: title, child: Text(body)),
      );
}

class _Steps extends StatelessWidget {
  const _Steps(this.phase);
  final Phase phase;

  @override
  Widget build(BuildContext context) {
    final current = switch (phase) {
      Phase.inviting => 0,
      Phase.gathering => 1,
      // Creating the key is part of opening the vote, not a step of its own.
      Phase.keyCeremony || Phase.voting => 2,
      _ => 3,
    };
    final scheme = Theme.of(context).colorScheme;
    return Row(
      children: [
        for (final (i, label) in ['Invite', 'Times', 'Vote', 'Result'].indexed)
          Expanded(
            child: Container(
              padding: const EdgeInsets.only(bottom: 8),
              margin: const EdgeInsets.only(right: 6),
              decoration: BoxDecoration(
                border: Border(
                  bottom: BorderSide(
                    width: 3,
                    color: i == current ? scheme.primary : scheme.outlineVariant,
                  ),
                ),
              ),
              child: Text(
                '${i + 1}  $label',
                style: TextStyle(
                  fontWeight: i == current ? FontWeight.w600 : FontWeight.normal,
                  color: i == current ? null : scheme.onSurfaceVariant,
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// Suggesting a slot: a date, a start time and a duration. The time and
/// duration stay after adding (and are remembered for next time), so several
/// days only need their dates.
class _Dates extends StatefulWidget {
  const _Dates(this.s);
  final _Ctx s;

  @override
  State<_Dates> createState() => _DatesState();
}

class _DatesState extends State<_Dates> {
  static final _compact = OutlinedButton.styleFrom(
    padding: const EdgeInsets.symmetric(horizontal: 10),
    visualDensity: VisualDensity.compact,
  );

  DateTime? _date;
  TimeOfDay _time = const TimeOfDay(hour: 18, minute: 0);
  int _minutes = 120;

  @override
  void initState() {
    super.initState();
    SlotDefaults.load().then((d) {
      if (!mounted) return;
      setState(() {
        _time = d.$1;
        _minutes = d.$2;
      });
    });
  }

  Future<void> _pickDate() async {
    final now = DateTime.now();
    final picked = await showDatePicker(
      context: context,
      initialDate: _date ?? now,
      firstDate: DateTime(now.year, now.month, now.day),
      lastDate: now.add(const Duration(days: 730)),
    );
    if (picked != null) setState(() => _date = picked);
  }

  Future<void> _pickTime() async {
    final picked = await showTimePicker(context: context, initialTime: _time);
    if (picked != null) setState(() => _time = picked);
  }

  void _add() {
    final date = _date;
    if (date == null) return;
    final local = DateTime(date.year, date.month, date.day, _time.hour, _time.minute);
    final slot = rust.makeSlot(
      date: DateFormat('yyyy-MM-dd').format(local),
      time: DateFormat('HH:mm').format(local),
      offsetMinutes: local.timeZoneOffset.inMinutes,
      minutes: _minutes,
    );
    if (slot == null) return;
    widget.s.conn.send({
      'SuggestSlot': {'slot': slot}
    });
    SlotDefaults.save(_time, _minutes);
    setState(() => _date = null);
  }

  @override
  Widget build(BuildContext context) {
    final slots = widget.s.view.slots;
    final date = _date;
    return _Card(
      title: 'Suggest times',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text('Anyone can add a time. The list is locked when voting opens.'),
          const SizedBox(height: 8),
          if (slots.isEmpty) const Text('No times yet.'),
          for (final (slot, by) in slots)
            ListTile(
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: Text(prettySlot(slot), style: const TextStyle(fontWeight: FontWeight.w600)),
              trailing: Text('by $by'),
            ),
          const SizedBox(height: 8),
          // Date, start time and duration on one row.
          Row(
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  style: _compact,
                  onPressed: _pickDate,
                  icon: const Icon(Icons.calendar_today, size: 16),
                  label: Text(
                    date == null ? 'Date' : DateFormat('EEE d MMM').format(date),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
              const SizedBox(width: 6),
              OutlinedButton.icon(
                style: _compact,
                onPressed: _pickTime,
                icon: const Icon(Icons.schedule, size: 16),
                label: Text(_time.format(context)),
              ),
              const SizedBox(width: 6),
              PopupMenuButton<int>(
                tooltip: 'Duration',
                initialValue: _minutes,
                onSelected: (m) => setState(() => _minutes = m),
                itemBuilder: (_) => [
                  for (final m in durations) PopupMenuItem(value: m, child: Text(prettyDuration(m))),
                ],
                // Looks like the other two; the menu button handles the tap.
                child: IgnorePointer(
                  child: OutlinedButton.icon(
                    style: _compact,
                    onPressed: () {},
                    icon: const Icon(Icons.timelapse, size: 16),
                    label: Text(prettyDuration(_minutes)),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          FilledButton.tonalIcon(
            onPressed: date == null ? null : _add,
            icon: const Icon(Icons.add),
            label: const Text('Add time'),
          ),
        ],
      ),
    );
  }
}

class _Invites extends StatefulWidget {
  const _Invites(this.s);
  final _Ctx s;

  @override
  State<_Invites> createState() => _InvitesState();
}

class _InvitesState extends State<_Invites> {
  final _name = TextEditingController();

  void _invite() {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    widget.s.conn.send({
      'Invite': {'name': name}
    });
    _name.clear();
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.s;
    return _Card(
      title: 'Invite people',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text('Each person gets their own link. Send it to them privately.'),
          const SizedBox(height: 12),
          Row(children: [
            Expanded(
              child: TextField(
                controller: _name,
                decoration: const InputDecoration(hintText: 'Their name', border: OutlineInputBorder()),
                onSubmitted: (_) => _invite(),
              ),
            ),
            const SizedBox(width: 8),
            FilledButton.tonal(onPressed: _invite, child: const Text('Create link')),
          ]),
          for (final (name, token) in s.view.invites)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(name),
              trailing: IconButton(
                icon: const Icon(Icons.ios_share),
                tooltip: 'Share link',
                onPressed: () {
                  final url = Invite(server: s.invite.server, poll: s.view.id, token: token).webUrl;
                  SharePlus.instance.share(ShareParams(
                    text: 'Help pick a time for ${s.view.title}: $url',
                    subject: s.view.title,
                  ));
                },
              ),
            ),
        ],
      ),
    );
  }
}

class _Start extends StatefulWidget {
  const _Start(this.s);
  final _Ctx s;

  @override
  State<_Start> createState() => _StartState();
}

class _StartState extends State<_Start> {
  int? _threshold;
  int _days = 1;

  @override
  Widget build(BuildContext context) {
    final view = widget.s.view;
    final n = view.participants.where((p) => p.registered).length;
    final t = (_threshold != null && _threshold! >= 1 && _threshold! <= n) ? _threshold! : n ~/ 2 + 1;
    final dates = view.slots.length;
    final days = _days.clamp(1, dates < 1 ? 1 : dates);
    final hint = n <= 1
        ? "Only you have joined so far, so you'd hold the whole key. Wait for others to open their links."
        : t == n
            ? 'All $n must come back to reveal the result. Nobody short of everyone could read a ballot.'
            : 'Any $t of $n can reveal the result, so it survives ${n - t} no-show${n - t == 1 ? '' : 's'}. '
                'But $t working together could also read individual ballots.';
    return _Card(
      title: 'Lock times & open voting',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Everyone who has opened their link ($n so far) becomes a trustee: their phones and '
            "browsers create the poll's key together, so no single person can unlock a ballot. That "
            'needs their app or page open for a moment after you lock the times.',
          ),
          const SizedBox(height: 12),
          Row(children: [
            const Expanded(child: Text('Trustees needed to reveal the result')),
            DropdownButton<int>(
              value: t,
              items: [
                for (var i = 1; i <= (n < 1 ? 1 : n); i++)
                  DropdownMenuItem(value: i, child: Text('$i of $n')),
              ],
              onChanged: (v) => setState(() => _threshold = v),
            ),
          ]),
          _Hint(hint),
          Row(children: [
            const Expanded(child: Text('Meetings to pick')),
            DropdownButton<int>(
              value: days,
              items: [
                for (var d = 1; d <= (dates < 1 ? 1 : dates); d++)
                  DropdownMenuItem(value: d, child: Text(d == 1 ? '1 meeting' : '$d meetings')),
              ],
              onChanged: (v) => setState(() => _days = v ?? 1),
            ),
          ]),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: dates == 0 || n == 0
                ? null
                : () => widget.s.conn.send({
                      'StartCeremony': {'threshold': t, 'days': days}
                    }),
            child: const Text('Lock times & open voting'),
          ),
        ],
      ),
    );
  }
}

/// Creating the key normally takes a split second, so this only appears if
/// it's slow — which means a trustee's device isn't connected — and says who
/// everyone is waiting for.
class _KeyCeremony extends StatefulWidget {
  const _KeyCeremony(this.s);
  final _Ctx s;

  @override
  State<_KeyCeremony> createState() => _KeyCeremonyState();
}

class _KeyCeremonyState extends State<_KeyCeremony> {
  bool _slow = false;
  late final Timer _timer = Timer(const Duration(milliseconds: 1500), () {
    if (mounted) setState(() => _slow = true);
  });

  @override
  void initState() {
    super.initState();
    _timer;
  }

  @override
  void dispose() {
    _timer.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (!_slow) return const SizedBox.shrink();
    final view = widget.s.view;
    final c = view.ceremony!;
    final waiting = [
      for (final (j, p) in c.trustees.indexed)
        if (c.allDealt ? !c.checked[j] : !c.dealtBy.contains(j + 1)) p,
    ];
    return _Card(
      title: 'Preparing the ballot box',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            "Before anyone votes, everyone's devices create the poll's key together, so no single "
            "person can unlock a ballot. It takes a moment once everyone's app or page is open.",
          ),
          for (final p in waiting)
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(Icons.circle, size: 10, color: view.participants[p].online ? Colors.green : null),
              title: Text(view.participants[p].name),
              subtitle: Text(p == view.me
                  ? 'you — keep the app open'
                  : view.participants[p].online
                      ? 'setting up…'
                      : 'needs to open their link'),
            ),
          const _Hint('Voting opens by itself as soon as everyone has done their part.'),
        ],
      ),
    );
  }
}

class _Voting extends StatefulWidget {
  const _Voting(this.s, {super.key});
  final _Ctx s;

  @override
  State<_Voting> createState() => _VotingState();
}

class _VotingState extends State<_Voting> {
  late List<int?> _choices = List.filled(widget.s.view.options.length, null);
  bool _editing = false;
  bool _busy = false;
  String? _receipt;

  int get _round => widget.s.view.rounds.length;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final view = widget.s.view;
    final remembered = await Keys.choices(widget.s.invite);
    final receipts = await Keys.receipts(widget.s.invite, _round);
    if (!mounted) return;
    setState(() {
      // Later rounds start from your last vote; picked times can't win again.
      if (_round > 1 && remembered != null && remembered.length == view.options.length) {
        _choices = [for (final (i, c) in remembered.indexed) view.picked.contains(i) ? null : c];
      }
      _receipt = receipts.reversed.whereType<String>().firstOrNull;
    });
  }

  Future<void> _cast() async {
    setState(() => _busy = true);
    try {
      final out = await rust.castBallot(viewJson: widget.s.view.raw, choices: _choices);
      await Keys.saveReceipt(widget.s.invite, _round, out.receiptHex);
      await Keys.saveChoices(widget.s.invite, _choices);
      widget.s.conn.sendJson(out.msgJson);
      setState(() {
        _receipt = out.receiptHex;
        _editing = false;
      });
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('$e')));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final view = widget.s.view;
    final mine = view.mine;
    final voters = view.participants;
    final ready = voters.where((p) => p.voted).length;
    final taken = [for (final o in view.picked) prettySlot(view.options[o])].join(', ');
    final Widget body;
    if (_editing || (!mine.hasBallot && !mine.voted)) {
      body = _ballot(view);
    } else if (mine.voted) {
      body = _Card(
        title: view.days > 1 ? "You're set for meeting $_round" : 'Your vote is in',
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('It was encrypted on this phone before it was sent. '
                'Receipt: ${_receipt?.substring(0, 16) ?? '—'}'),
            const _Hint('You can still change it until voting closes.'),
            OutlinedButton(
              onPressed: () => setState(() => _editing = true),
              child: const Text('Change my vote'),
            ),
          ],
        ),
      );
    } else {
      body = _Card(
        title: 'Meeting $_round of ${view.days}',
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Your vote from the last round still counts. Keep it, or update it now that $taken is taken.'),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: () => widget.s.conn.send('KeepVote'),
              child: const Text('Keep my votes'),
            ),
            const SizedBox(height: 8),
            OutlinedButton(
              onPressed: () => setState(() => _editing = true),
              child: const Text('Update my votes'),
            ),
          ],
        ),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        body,
        _Card(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text('Ready: $ready of ${voters.length}'),
              const SizedBox(height: 8),
              LinearProgressIndicator(value: voters.isEmpty ? 0 : ready / voters.length),
              _Hint(_round > 1
                  ? 'Voting closes by itself once everyone has kept or updated their vote.'
                  : 'Voting closes by itself once everyone has voted.'),
              if (view.isCreator)
                OutlinedButton(
                  onPressed: () => widget.s.conn.send('CloseVoting'),
                  child: const Text('Close voting now'),
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _ballot(PollView view) {
    return _Card(
      title: view.days > 1
          ? 'Meeting $_round of ${view.days}: how well does each time work for you?'
          : 'How well does each time work for you?',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            "Your choices are encrypted on this phone, with proofs that they're well-formed. "
            'Nobody — not the server, not any single trustee — can read them.',
          ),
          for (final (i, date) in view.options.indexed)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(prettySlot(date), style: const TextStyle(fontWeight: FontWeight.w600)),
                  const SizedBox(height: 6),
                  if (view.picked.contains(i))
                    Text('Picked for meeting ${view.picked.indexOf(i) + 1}')
                  else
                    SegmentedButton<int>(
                      showSelectedIcon: false,
                      segments: [
                        for (final (t, label) in tiers.indexed)
                          ButtonSegment(value: t, label: Text(label)),
                        const ButtonSegment(value: -1, label: Text("Can't")),
                      ],
                      selected: {_choices[i] ?? -1},
                      onSelectionChanged: (v) => setState(() {
                        final t = v.first;
                        _choices[i] = t < 0 ? null : t;
                      }),
                    ),
                ],
              ),
            ),
          const SizedBox(height: 16),
          FilledButton(
            onPressed: _busy ? null : _cast,
            child: const Text('Cast encrypted vote'),
          ),
          if (_editing)
            TextButton(
              onPressed: () => setState(() => _editing = false),
              child: const Text('Cancel'),
            ),
        ],
      ),
    );
  }
}

class _Tallying extends StatelessWidget {
  const _Tallying(this.s);
  final _Ctx s;

  @override
  Widget build(BuildContext context) {
    final view = s.view;
    final done = view.rounds.last.decryptedBy.length;
    final needed = view.ceremony?.threshold ?? 0;
    return _Card(
      title: view.days > 1 ? 'Revealing meeting ${view.rounds.length} of ${view.days}' : 'Revealing the result',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            "Trustees' devices each decrypt a share of the total — never an individual ballot — and "
            'prove they did it honestly.',
          ),
          const SizedBox(height: 12),
          Text('Trustees: $done of $needed needed'),
          const SizedBox(height: 8),
          LinearProgressIndicator(value: needed == 0 ? 0 : done / needed),
        ],
      ),
    );
  }
}

class _ResultSummary extends StatefulWidget {
  const _ResultSummary(this.s, this.outcomes);
  final _Ctx s;
  final List<rust.OutcomeOut> outcomes;

  @override
  State<_ResultSummary> createState() => _ResultSummaryState();
}

class _ResultSummaryState extends State<_ResultSummary> {
  /// Picked slots already added to this phone's calendar.
  List<String> _added = [];
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    DeviceCalendar.added(widget.s.invite).then((d) {
      if (mounted) setState(() => _added = d);
    });
  }

  List<rust.OutcomeOut> get _picked => [for (final o in widget.outcomes) if (o.winnerSlot != null) o];

  String _note(rust.OutcomeOut o) =>
      '${o.winnerAvailable} of ${o.ballots} can make it. Picked with whisperpoll.';

  Future<void> _addToCalendar() async {
    final picked = _picked;
    final messenger = ScaffoldMessenger.of(context);
    setState(() => _busy = true);
    try {
      final count = await DeviceCalendar.add(widget.s.invite, [
        for (final o in picked)
          SlotEvent(slot: o.winnerSlot!, title: widget.s.view.title, notes: _note(o)),
      ]);
      setState(() => _added = [for (final o in picked) o.winnerSlot!]);
      messenger.showSnackBar(SnackBar(
        content: Text(count == 1 ? 'Added to your calendar.' : 'Added $count meetings to your calendar.'),
        action: SnackBarAction(label: 'Open', onPressed: () => DeviceCalendar.open(picked.first.winnerSlot!)),
      ));
    } on PlatformException catch (e) {
      messenger.showSnackBar(SnackBar(
        content: Text(e.code == 'denied'
            ? 'Calendar access is off. Allow it in Settings, or share the .ics file instead.'
            : "Couldn't add to your calendar: ${e.message}"),
      ));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// The same times as an .ics file, for other calendars or other people.
  Future<void> _shareIcs() async {
    final view = widget.s.view;
    final picked = _picked;
    final stamp = DateFormat("yyyyMMdd'T'HHmmss'Z'").format(DateTime.now().toUtc());
    final ics = rust.calendarIcs(
      title: view.title,
      pollId: view.id,
      stamp: stamp,
      slots: [for (final o in picked) o.winnerSlot!],
      notes: [for (final o in picked) _note(o)],
    );
    final dir = await getTemporaryDirectory();
    final slug = view.title.toLowerCase().replaceAll(RegExp(r'[^a-z0-9]+'), '-').replaceAll(RegExp(r'^-|-$'), '');
    final file = File('${dir.path}/${slug.isEmpty ? 'whisperpoll' : slug}.ics');
    await file.writeAsString(ics);
    await SharePlus.instance.share(ShareParams(
      files: [XFile(file.path, mimeType: 'text/calendar')],
      subject: view.title,
    ));
  }

  @override
  Widget build(BuildContext context) {
    final outcomes = widget.outcomes;
    if (outcomes.isEmpty) return const Center(child: CircularProgressIndicator());
    final theme = Theme.of(context);
    final picked = _picked;
    final days = widget.s.view.days;
    final inCalendar = picked.isNotEmpty && picked.every((o) => _added.contains(o.winnerSlot));
    return Card(
      color: theme.colorScheme.primaryContainer,
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (days == 1 && picked.isNotEmpty) ...[
              const Text('Best time'),
              Text(prettySlot(picked.first.winnerSlot!), style: theme.textTheme.headlineSmall),
              Text('${picked.first.winnerAvailable} of ${picked.first.ballots} can make it'),
            ] else if (picked.isEmpty) ...[
              Text('No time works', style: theme.textTheme.headlineSmall),
              const Text('Nobody can make any of the suggested times.'),
            ] else ...[
              Text('Found ${picked.length} of $days meetings'),
              for (final (i, o) in outcomes.indexed)
                Text(o.winnerSlot == null
                    ? '${i + 1}. No remaining time works for anyone, so the search stopped here.'
                    : '${i + 1}. ${prettySlot(o.winnerSlot!)} — ${o.winnerAvailable} of ${o.ballots} can make it'),
            ],
            if (picked.isNotEmpty) ...[
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  if (inCalendar)
                    FilledButton.tonalIcon(
                      onPressed: () => DeviceCalendar.open(picked.first.winnerSlot!),
                      icon: const Icon(Icons.event_available),
                      label: const Text('In your calendar'),
                    )
                  else
                    FilledButton.icon(
                      onPressed: _busy ? null : _addToCalendar,
                      icon: const Icon(Icons.event),
                      label: Text(picked.length == 1 ? 'Add to calendar' : 'Add all ${picked.length} to calendar'),
                    ),
                  TextButton.icon(
                    onPressed: _shareIcs,
                    icon: const Icon(Icons.ios_share, size: 18),
                    label: const Text('Share .ics'),
                  ),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _RoundDetails extends StatelessWidget {
  const _RoundDetails(this.s, this.o, {required this.round, required this.open});
  final _Ctx s;
  final rust.OutcomeOut o;
  final int round;
  final bool open;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final checksOut = o.verificationError == null && o.winnerChecksOut;
    final title = o.winnerSlot == null
        ? 'Meeting $round: no time found'
        : s.view.days == 1
            ? 'Result: ${prettySlot(o.winnerSlot!)}'
            : 'Meeting $round: ${prettySlot(o.winnerSlot!)}';
    final ok = TextStyle(color: Colors.green.shade600);
    final bad = TextStyle(color: theme.colorScheme.error);
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: ExpansionTile(
        initiallyExpanded: open,
        title: Text(title),
        trailing: Text(checksOut ? '✓ verified' : '✗ check failed', style: checksOut ? ok : bad),
        childrenPadding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
        expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: DataTable(
              columnSpacing: 16,
              columns: [
                const DataColumn(label: Text('')),
                for (final t in tiers) DataColumn(label: Text(t), numeric: true),
                const DataColumn(label: Text('Can come'), numeric: true),
                const DataColumn(label: Text('Score'), numeric: true),
              ],
              rows: [
                for (final row in o.rows)
                  DataRow(cells: [
                    DataCell(Text(
                      prettySlot(row.slot) + (row.pickedEarlier ? ' (picked)' : ''),
                      style: row.pickedEarlier ? TextStyle(color: theme.disabledColor) : null,
                    )),
                    for (final c in row.counts) DataCell(Text('$c')),
                    DataCell(Text('${row.available}')),
                    DataCell(Text('${row.score}')),
                  ]),
              ],
            ),
          ),
          const _Hint('Times are ranked by how many can come, then by preference (3 / 2 / 1 points).'),
          if (o.verificationError == null)
            Text(
              '✓ Checked on this phone: all ${o.ballots} ballots are valid, and the totals are exactly '
              'what the decryption proofs from ${o.decryptedBy.join(' and ')} show.',
              style: ok,
            )
          else
            Text('✗ Verification failed: ${o.verificationError}', style: bad),
          if (!o.winnerChecksOut)
            Text("✗ The server's pick for this round doesn't match these totals!", style: bad),
          const SizedBox(height: 4),
          switch (o.mine) {
            true => Text('✓ Your ballot is on the board and was counted.', style: ok),
            false => Text('✗ Your ballot is missing from the board!', style: bad),
            null => const Text("You didn't vote from this phone."),
          },
        ],
      ),
    );
  }
}

class _People extends StatelessWidget {
  const _People(this.s);
  final _Ctx s;

  String _status(int i, Participant p) {
    final view = s.view;
    final c = view.ceremony;
    final j = c?.trustees.indexOf(i) ?? -1;
    switch (view.phase) {
      case Phase.inviting || Phase.gathering:
        return p.registered ? 'joined' : 'not joined yet';
      case Phase.keyCeremony when j >= 0:
        if (c!.checked[j]) return 'ready';
        if (c.dealtBy.contains(j + 1) && !c.allDealt) return 'waiting for others';
        return p.online ? 'setting up…' : 'needs to open their link';
      case Phase.voting:
        return p.voted ? 'voted' : p.hasBallot ? 'deciding to keep or update' : 'not voted';
      case Phase.tallying || Phase.done when j >= 0:
        if (view.rounds.last.decryptedBy.contains(j + 1)) return 'decrypted';
        return view.phase == Phase.tallying ? 'decrypting…' : '';
      default:
        return j < 0 ? 'not a trustee' : '';
    }
  }

  @override
  Widget build(BuildContext context) {
    final view = s.view;
    return _Card(
      title: 'People',
      child: Column(
        children: [
          for (final (i, p) in view.participants.indexed)
            ListTile(
              contentPadding: EdgeInsets.zero,
              dense: true,
              leading: Icon(Icons.circle, size: 10, color: p.online ? Colors.green : Theme.of(context).disabledColor),
              title: Text('${p.name}${i == view.me ? ' (you)' : ''}${p.creator ? ' · creator' : ''}'),
              subtitle: Text(_status(i, p)),
              onLongPress: () => Clipboard.setData(ClipboardData(text: p.name)),
            ),
        ],
      ),
    );
  }
}
