import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;

import 'model.dart';
import 'poll_screen.dart' show prettySlot;
import 'storage.dart';

/// The server's `PollSummary`: a poll at a glance for one participant.
class PollSummary {
  PollSummary(Map<String, dynamic> j)
    : title = j['title'] as String,
      you = j['you'] as String,
      phase = PollView.parsePhase(j['phase']),
      failure = PollView.parseFailure(j['phase']),
      picked = (j['picked'] as List).cast<String>(),
      yourTurn = j['your_turn'] as bool,
      participants = j['participants'] as int;

  final String title;
  final String you;
  final Phase phase;
  final String? failure;
  final List<String> picked;
  final bool yourTurn;
  final int participants;

  bool get running => phase != Phase.done && phase != Phase.failed;
}

sealed class PollStatus {
  const PollStatus();

  /// Worth showing among the running polls: in progress, or unknown.
  bool get running => switch (this) {
    Live(:final summary) => summary.running,
    Unreachable() => true,
    Loading() || Expired() => false,
  };
}

class Loading extends PollStatus {
  const Loading();
}

class Live extends PollStatus {
  const Live(this.summary);
  final PollSummary summary;
}

/// The server no longer has it: a week without changes, or a restart.
class Expired extends PollStatus {
  const Expired();
}

class Unreachable extends PollStatus {
  const Unreachable();
}

Future<PollStatus> fetchStatus(Invite invite) async {
  try {
    final response = await http
        .get(invite.summaryUrl)
        .timeout(const Duration(seconds: 10));
    if (response.statusCode == 404) return const Expired();
    if (response.statusCode != 200) return const Unreachable();
    return Live(PollSummary(jsonDecode(response.body) as Map<String, dynamic>));
  } catch (_) {
    return const Unreachable();
  }
}

/// Every joined poll with its status, fetched together.
Future<List<(Invite, PollStatus)>> loadStatuses() async {
  final invites = await Invites.all();
  final statuses = await Future.wait(invites.map(fetchStatus));
  return [for (var i = 0; i < invites.length; i++) (invites[i], statuses[i])];
}

String phaseText(Phase phase) => switch (phase) {
  Phase.inviting => 'Inviting people',
  Phase.gathering => 'Suggesting times',
  Phase.keyCeremony => 'Creating the shared key',
  Phase.voting => 'Voting',
  Phase.tallying => 'Counting votes',
  Phase.done => 'Done',
  Phase.failed => 'Failed',
};

/// One poll in a list: title, status, picked times; tap to open.
class PollTile extends StatelessWidget {
  const PollTile({
    super.key,
    required this.invite,
    required this.status,
    required this.onOpen,
  });

  final Invite invite;
  final PollStatus status;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.colorScheme.onSurfaceVariant);
    final title = switch (status) {
      Live(:final summary) => summary.title,
      _ => invite.title ?? 'Poll ${invite.poll}',
    };
    final Widget subtitle = switch (status) {
      Loading() => const Text('…'),
      Unreachable() => Text("Couldn't reach ${Uri.parse(invite.server).host}"),
      Expired() => const Text(
        'Expired — polls are kept for a week after their last change',
      ),
      Live(:final summary) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            [
              summary.you,
              phaseText(summary.phase),
              summary.participants == 1
                  ? 'just you'
                  : '${summary.participants} people',
            ].join(' · '),
          ),
          if (summary.failure != null) Text(summary.failure!),
          for (final slot in summary.picked)
            Text(
              '✓ ${prettySlot(slot)}',
              style: TextStyle(color: theme.colorScheme.onSurface),
            ),
        ],
      ),
    };
    final yourTurn = status is Live && (status as Live).summary.yourTurn;
    return ListTile(
      enabled: status is! Expired,
      title: Text(title),
      subtitle: DefaultTextStyle.merge(style: muted, child: subtitle),
      trailing: yourTurn
          ? Chip(
              label: const Text('Your turn'),
              visualDensity: VisualDensity.compact,
              backgroundColor: theme.colorScheme.primaryContainer,
              side: BorderSide.none,
            )
          : const Icon(Icons.chevron_right),
      onTap: onOpen,
    );
  }
}

/// All joined polls, finished and expired ones too. Swipe to remove.
class HistoryScreen extends StatefulWidget {
  const HistoryScreen({super.key, required this.onOpen});

  final Future<void> Function(Invite) onOpen;

  @override
  State<HistoryScreen> createState() => _HistoryScreenState();
}

class _HistoryScreenState extends State<HistoryScreen> {
  List<(Invite, PollStatus)>? _polls;

  @override
  void initState() {
    super.initState();
    _load();
    Invites.changed.addListener(_load);
  }

  @override
  void dispose() {
    Invites.changed.removeListener(_load);
    super.dispose();
  }

  Future<void> _load() async {
    final polls = await loadStatuses();
    if (mounted) setState(() => _polls = polls);
  }

  Future<void> _remove(Invite invite) async {
    // Out of the tree right away, as Dismissible requires.
    setState(() => _polls?.removeWhere((p) => p.$1.key == invite.key));
    await Invites.remove(invite);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final polls = _polls;
    return Scaffold(
      appBar: AppBar(title: const Text('Your polls')),
      body: polls == null
          ? const Center(child: CircularProgressIndicator())
          : RefreshIndicator(
              onRefresh: _load,
              child: ListView(
                padding: const EdgeInsets.symmetric(vertical: 8),
                children: [
                  Padding(
                    padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
                    child: Text(
                      polls.isEmpty
                          ? 'No polls yet. Polls you create or join show up here.'
                          : 'The server keeps a poll for a week after its last change. '
                                'Swipe to remove one from this list.',
                      style: TextStyle(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ),
                  for (final (invite, status) in polls)
                    Dismissible(
                      key: ValueKey(invite.key),
                      direction: DismissDirection.endToStart,
                      background: Container(
                        color: theme.colorScheme.errorContainer,
                        alignment: Alignment.centerRight,
                        padding: const EdgeInsets.only(right: 16),
                        child: const Icon(Icons.delete_outline),
                      ),
                      onDismissed: (_) => _remove(invite),
                      child: PollTile(
                        invite: invite,
                        status: status,
                        onOpen: () => widget.onOpen(invite),
                      ),
                    ),
                ],
              ),
            ),
    );
  }
}
