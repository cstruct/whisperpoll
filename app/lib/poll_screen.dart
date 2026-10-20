import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:path_provider/path_provider.dart';
import 'package:share_plus/share_plus.dart';

import 'calendar.dart';
import 'connection.dart';
import 'maxi.dart';
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
    return Scaffold(
      appBar: AppBar(
        title: const FittedBox(fit: BoxFit.scaleDown, child: Text('~*~ whisperpoll ~*~')),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 16),
            child: Sticker(
              tilt: 3,
              shadow: 3,
              border: 2,
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                StatusDot(online: _conn.connected, onColor: const Color(0xFF1FBF3A)),
                const SizedBox(width: 6),
                Text(
                  _conn.gone
                      ? 'Offline'
                      : view == null
                          ? 'Connecting'
                          : "You're ${view.mine.name}",
                  style: Maxi.display(11, Maxi.ink),
                ),
              ]),
            ),
          ),
        ],
      ),
      body: Starfield(
        child: ListenableBuilder(
          listenable: _conn,
          builder: (context, _) => _body(context, view),
        ),
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
      return Center(child: Text('Connecting…', style: Maxi.pixel(28, Maxi.lime)));
    }
    final s = _Ctx(conn: _conn, view: view, invite: widget.invite);
    final ticker = switch (view.phase) {
      Phase.inviting => 'Everyone gets their own link ★ send it privately ★ next up: suggest times',
      Phase.gathering => 'Anyone can add a time ★ the list locks when voting opens',
      Phase.keyCeremony => 'Building a key nobody holds alone ★ hang tight',
      Phase.voting => 'Your ballot is encrypted before it leaves ★ nobody can read it ★ not even the server',
      Phase.tallying => 'Only the total gets decrypted ★ almost there',
      Phase.done => 'We have a result ★ checked on this phone ★ proofs included',
      Phase.failed => 'Something went wrong',
    };
    return Column(children: [
      Ticker(ticker),
      Expanded(child: _list(s, view)),
    ]);
  }

  Widget _list(_Ctx s, PollView view) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
      children: [
        Text(view.title, style: Maxi.chroma(30)),
        const SizedBox(height: 18),
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
        const SizedBox(height: 6),
        _People(s),
        const Badges(),
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
        color: Maxi.yellow,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('$opened. Next, everyone suggests times that could work; you can still invite people then.'),
            const SizedBox(height: 14),
            Pop(
              child: FilledButton(
                onPressed: () => s.conn.send('FinishInviting'),
                child: const _Onward('Next: suggest times'),
              ),
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
          Padding(
            padding: const EdgeInsets.only(bottom: 18, right: 6),
            child: Sticker(
              padding: EdgeInsets.zero,
              shadow: 6,
              child: Theme(
                data: Maxi.onLight(Theme.of(context)),
                child: ExpansionTile(
                  title: Text('Invite more people', style: Maxi.display(15, Maxi.ink)),
                  childrenPadding: const EdgeInsets.fromLTRB(8, 0, 8, 0),
                  children: [_Invites(s)],
                ),
              ),
            ),
          ),
        ] else
          _Card(color: Maxi.yellow, child: Text(
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
  const _Card({required this.child, this.title, this.color = Maxi.cyan, this.icon});
  final Widget child;
  final String? title;
  final Color color;
  final Widget? icon;

  @override
  Widget build(BuildContext context) => ZigCard(title: title, icon: icon, color: color, child: child);
}

/// A button label with an arrow that nudges forward.
class _Onward extends StatelessWidget {
  const _Onward(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Flexible(child: Text(label, textAlign: TextAlign.center)),
          const SizedBox(width: 8),
          const Animated(Motion.nudge, child: Icon(Icons.arrow_forward, size: 20)),
        ],
      );
}

class _Hint extends StatelessWidget {
  const _Hint(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme.onSurfaceVariant;
    return Padding(
      padding: const EdgeInsets.fromLTRB(2, 2, 2, 16),
      child: Container(
        padding: const EdgeInsets.only(left: 10),
        decoration: BoxDecoration(border: Border(left: BorderSide(color: color, width: 3))),
        child: Text(text, style: TextStyle(color: color, fontSize: 13, fontWeight: FontWeight.w500)),
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
    return Padding(
      padding: const EdgeInsets.only(bottom: 16, right: 4),
      child: Sticker(
        color: Maxi.orange,
        child: Row(children: [
          Expanded(
            child: Text(text, style: const TextStyle(color: Maxi.ink, fontWeight: FontWeight.w700)),
          ),
          if (onDismiss != null)
            IconButton(icon: const Icon(Icons.close, color: Maxi.ink), tooltip: 'Dismiss', onPressed: onDismiss),
        ]),
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
        child: _Card(title: title, color: Maxi.orange, child: Text(body)),
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
    return Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 14),
      child: Row(
        children: [
          for (final (i, label) in ['Invite', 'Times', 'Vote', 'Result'].indexed)
            Expanded(
              child: Padding(
                padding: EdgeInsets.only(right: i < 3 ? 8 : 0),
                child: Semantics(
                  selected: i == current,
                  child: _StepTab(i + 1, label, done: i < current, now: i == current),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _StepTab extends StatelessWidget {
  const _StepTab(this.n, this.label, {required this.done, required this.now});
  final int n;
  final String label;
  final bool done;
  final bool now;

  @override
  Widget build(BuildContext context) {
    final fg = done || now ? Maxi.ink : Maxi.cyan;
    final tab = Container(
      height: 60,
      width: double.infinity,
      decoration: BoxDecoration(
        color: now ? Maxi.lime : done ? Maxi.pink : Colors.transparent,
        border: Border.all(color: done || now ? Maxi.ink : Maxi.cyan, width: 3),
        boxShadow: done || now ? Maxi.hard(Maxi.ink, now ? 4 : 3) : null,
      ),
      child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
        Text('$n', style: Maxi.display(22, fg)),
        Text(label.toUpperCase(), style: Maxi.display(9, fg)),
      ]),
    );
    if (!now) return tab;
    return Transform.rotate(
      angle: -0.07,
      child: Transform.scale(
        scale: 1.06,
        child: Stack(clipBehavior: Clip.none, children: [
          tab,
          const Positioned(top: -14, right: -10, child: SpinningStar()),
        ]),
      ),
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
    const ticketColors = [Maxi.lime, Maxi.yellow, Maxi.paper];
    const ticketTilts = [-1.0, 0.8, -0.4];
    return _Card(
      title: 'Suggest times',
      icon: const Animated(Motion.wiggle, child: Icon(Icons.schedule, size: 30)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text('Anyone can add a time. The list is locked when voting opens.'),
          const SizedBox(height: 12),
          if (slots.isEmpty) const Text('No times yet.'),
          for (final (i, (slot, by)) in slots.indexed)
            Padding(
              padding: const EdgeInsets.only(bottom: 10, right: 4),
              child: Sticker(
                color: ticketColors[i % 3],
                tilt: ticketTilts[i % 3],
                padding: const EdgeInsets.fromLTRB(14, 10, 12, 10),
                child: Row(children: [
                  Expanded(child: Text(prettySlot(slot), style: const TextStyle(fontWeight: FontWeight.w700))),
                  Text('by $by', style: const TextStyle(fontSize: 13)),
                ]),
              ),
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
          const SizedBox(height: 4),
          Pop(
            child: FilledButton.icon(
              onPressed: date == null ? null : _add,
              icon: const Icon(Icons.add),
              label: const Text('Add time'),
            ),
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
      icon: const Animated(Motion.bob, child: Icon(Icons.mail_outline, size: 30)),
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
            Pop(color: Maxi.ink, child: FilledButton(onPressed: _invite, child: const Text('Create link'))),
          ]),
          for (final (name, token) in s.view.invites)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(name, style: Maxi.display(15, Maxi.ink)),
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
      color: Maxi.pink,
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
          Pop(
            color: Maxi.cyan,
            child: FilledButton(
              onPressed: dates == 0 || n == 0
                  ? null
                  : () => widget.s.conn.send({
                        'StartCeremony': {'threshold': t, 'days': days}
                      }),
              child: const _Onward('Lock times & open voting'),
            ),
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
      icon: const Animated(Motion.spin, child: Icon(Icons.key, size: 28)),
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
              leading: StatusDot(online: view.participants[p].online),
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
        color: Maxi.lime,
        icon: const Animated(Motion.bob, child: Icon(Icons.check_box_outlined, size: 30)),
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
          color: Maxi.yellow,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(children: [
                const Expanded(child: Text('Ballots in', style: TextStyle(fontWeight: FontWeight.w700))),
                Lcd('$ready/${voters.length}'),
              ]),
              const SizedBox(height: 10),
              _Meter(voters.isEmpty ? 0 : ready / voters.length),
              const SizedBox(height: 10),
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
      icon: const Animated(Motion.pulse, child: Icon(Icons.lock, size: 30)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            "Your choices are encrypted on this phone, with proofs that they're well-formed. "
            'Nobody — not the server, not any single trustee — can read them.',
          ),
          for (final (i, date) in view.options.indexed)
            Padding(
              padding: const EdgeInsets.only(top: 14, right: 4),
              child: Opacity(
                opacity: view.picked.contains(i) ? 0.55 : 1,
                child: Sticker(
                  padding: const EdgeInsets.all(12),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(prettySlot(date).toUpperCase(), style: Maxi.display(14, Maxi.ink)),
                      const SizedBox(height: 10),
                      if (view.picked.contains(i))
                        Text('Picked for meeting ${view.picked.indexOf(i) + 1}')
                      else
                        _TierPicker(
                          value: _choices[i],
                          onChanged: (t) => setState(() => _choices[i] = t),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          const SizedBox(height: 18),
          Pop(
            child: FilledButton(
              onPressed: _busy ? null : _cast,
              child: const Row(mainAxisSize: MainAxisSize.min, children: [
                Animated(Motion.wiggle, child: Icon(Icons.lock_outline, size: 20)),
                SizedBox(width: 8),
                Text('Cast encrypted vote'),
              ]),
            ),
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

/// Prefer / OK / If needed / Can't, one color each; the pick tilts.
class _TierPicker extends StatelessWidget {
  const _TierPicker({required this.value, required this.onChanged});
  final int? value;
  final ValueChanged<int?> onChanged;

  static const _colors = [Maxi.lime, Maxi.pink, Maxi.yellow];

  @override
  Widget build(BuildContext context) {
    final options = [for (final (t, label) in tiers.indexed) (t as int?, label), (null, "Can't")];
    return Row(children: [
      for (final (k, (t, label)) in options.indexed) ...[
        if (k > 0) const SizedBox(width: 6),
        Expanded(
          child: Semantics(
            button: true,
            selected: value == t,
            child: GestureDetector(
              onTap: () => onChanged(t),
              child: Transform.rotate(
                angle: value == t ? -0.05 : 0,
                child: Container(
                  height: 46,
                  alignment: Alignment.center,
                  padding: const EdgeInsets.symmetric(horizontal: 3),
                  decoration: BoxDecoration(
                    color: value == t ? (t == null ? Maxi.orange : _colors[t]) : Maxi.paper,
                    border: Border.all(color: Maxi.ink, width: 3),
                    boxShadow: value == t ? Maxi.hard(Maxi.ink, 3) : null,
                  ),
                  child: FittedBox(
                    fit: BoxFit.scaleDown,
                    child: Text(
                      label,
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        color: Maxi.ink,
                        fontWeight: FontWeight.w700,
                        fontSize: 14,
                        decoration: value == t && t == null ? TextDecoration.lineThrough : null,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    ]);
  }
}

/// A striped progress bar in a black frame.
class _Meter extends StatelessWidget {
  const _Meter(this.value);
  final double value;

  @override
  Widget build(BuildContext context) => Container(
        height: 22,
        decoration: BoxDecoration(color: Maxi.paper, border: Border.all(color: Maxi.ink, width: 3)),
        child: FractionallySizedBox(
          alignment: Alignment.centerLeft,
          widthFactor: value.clamp(0, 1),
          child: const ColoredBox(color: Maxi.pink),
        ),
      );
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
      icon: const Animated(Motion.spin, child: Icon(Icons.key, size: 28)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            "Trustees' devices each decrypt a share of the total — never an individual ballot — and "
            'prove they did it honestly.',
          ),
          const SizedBox(height: 12),
          Row(children: [
            const Expanded(child: Text('Trustees done', style: TextStyle(fontWeight: FontWeight.w700))),
            Lcd('$done/$needed'),
          ]),
          const SizedBox(height: 10),
          _Meter(needed == 0 ? 0 : done / needed),
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
    if (outcomes.isEmpty) {
      return Center(child: Text('Checking the result…', style: Maxi.pixel(26, Maxi.lime)));
    }
    final picked = _picked;
    final days = widget.s.view.days;
    final inCalendar = picked.isNotEmpty && picked.every((o) => _added.contains(o.winnerSlot));
    final big = Maxi.display(24, Maxi.ink);
    return ZigCard(
      color: Maxi.lime,
      child: Stack(clipBehavior: Clip.none, children: [
        if (picked.isNotEmpty) const Positioned(top: -8, right: -4, child: Starburst(size: 76)),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (days == 1 && picked.isNotEmpty) ...[
              const Tag('Best time!!', textColor: Maxi.lime, tilt: -4),
              const SizedBox(height: 10),
              Padding(
                padding: const EdgeInsets.only(right: 80),
                child: Text(prettySlot(picked.first.winnerSlot!).toUpperCase(), style: big),
              ),
              const SizedBox(height: 4),
              Text('${picked.first.winnerAvailable} of ${picked.first.ballots} can make it',
                  style: const TextStyle(fontWeight: FontWeight.w700)),
            ] else if (picked.isEmpty) ...[
              Text('No time works', style: big),
              const Text('Nobody can make any of the suggested times.'),
            ] else ...[
              const Tag('Results!!', textColor: Maxi.lime, tilt: -4),
              const SizedBox(height: 10),
              Padding(
                padding: const EdgeInsets.only(right: 80),
                child: Text('Found ${picked.length} of $days meetings', style: big),
              ),
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
                    Pop(
                      color: Maxi.ink,
                      child: OutlinedButton.icon(
                        onPressed: () => DeviceCalendar.open(picked.first.winnerSlot!),
                        icon: const Icon(Icons.event_available),
                        label: const Text('In your calendar'),
                      ),
                    )
                  else
                    Pop(
                      child: FilledButton.icon(
                        onPressed: _busy ? null : _addToCalendar,
                        icon: const Animated(Motion.bob, child: Icon(Icons.event)),
                        label: Text(picked.length == 1 ? 'Add to calendar' : 'Add all ${picked.length} to calendar'),
                      ),
                    ),
                  Pop(
                    color: Maxi.ink,
                    child: OutlinedButton.icon(
                      onPressed: _shareIcs,
                      icon: const Icon(Icons.ios_share, size: 18),
                      label: const Text('Share .ics'),
                    ),
                  ),
                ],
              ),
            ],
          ],
        ),
      ]),
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
    const ok = TextStyle(color: Maxi.ok, fontWeight: FontWeight.w700);
    const bad = TextStyle(color: Maxi.bad, fontWeight: FontWeight.w700);
    return Padding(
      padding: const EdgeInsets.only(bottom: 18, right: 6),
      child: Sticker(
        padding: EdgeInsets.zero,
        shadow: 6,
        child: Theme(
          data: Maxi.onLight(theme),
          child: ExpansionTile(
        initiallyExpanded: open,
        title: Text(title, style: Maxi.display(14, Maxi.ink)),
        trailing: checksOut
            ? const Row(mainAxisSize: MainAxisSize.min, children: [
                Animated(Motion.spin, child: Icon(Icons.verified, color: Maxi.ok)),
                SizedBox(width: 4),
                Text('verified', style: ok),
              ])
            : const Text('✗ check failed', style: bad),
        childrenPadding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
        expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: DataTable(
              headingTextStyle: Maxi.display(10, Maxi.ink),
              dataTextStyle: const TextStyle(color: Maxi.ink, fontWeight: FontWeight.w600),
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
            true => const Text('✓ Your ballot is on the board and was counted.', style: ok),
            false => const Text('✗ Your ballot is missing from the board!', style: bad),
            null => const Text("You didn't vote from this phone."),
          },
        ],
          ),
        ),
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
    final online = view.participants.where((p) => p.online).length;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8, right: 6),
      child: Sticker(
        color: Maxi.pink,
        tilt: -1,
        shadow: 6,
        padding: const EdgeInsets.fromLTRB(16, 14, 16, 8),
        child: Theme(
          data: Maxi.onLight(Theme.of(context)),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(children: [
                Expanded(child: Text('People', style: Maxi.display(18, Maxi.ink))),
                Text('online ', style: Maxi.pixel(20, Maxi.ink)),
                Lcd(online.toString().padLeft(3, '0')),
              ]),
              const SizedBox(height: 4),
              for (final (i, p) in view.participants.indexed)
                Container(
                  decoration: const BoxDecoration(border: Border(top: BorderSide(color: Maxi.ink, width: 2))),
                  child: ListTile(
                    contentPadding: EdgeInsets.zero,
                    dense: true,
                    leading: StatusDot(online: p.online),
                    minLeadingWidth: 14,
                    title: Row(children: [
                      Flexible(
                        child: Text(
                          '${p.name}${i == view.me ? ' (you)' : ''}',
                          style: const TextStyle(fontWeight: FontWeight.w700, fontSize: 15),
                        ),
                      ),
                      if (p.creator) ...[const SizedBox(width: 6), const Tag('Creator')],
                    ]),
                    subtitle: Text(_status(i, p), style: const TextStyle(color: Maxi.ink)),
                    onLongPress: () => Clipboard.setData(ClipboardData(text: p.name)),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
