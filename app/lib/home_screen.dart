import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'history.dart';
import 'storage.dart';

const defaultServer = 'https://whisperpoll.fly.dev';

/// Joined polls, joining by link, and starting a poll.
class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key, required this.onOpen});

  final Future<void> Function(Invite) onOpen;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  /// Joined polls with their status; `null` until first loaded.
  List<(Invite, PollStatus)>? _polls;
  final _link = TextEditingController();
  final _title = TextEditingController();
  final _name = TextEditingController();
  final _server = TextEditingController(text: defaultServer);
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    SharedPreferences.getInstance().then((prefs) {
      if (mounted) _server.text = prefs.getString('server') ?? defaultServer;
    });
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

  Future<void> _openHistory() async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(builder: (_) => HistoryScreen(onOpen: widget.onOpen)),
    );
  }

  Future<void> _open(Invite invite) => widget.onOpen(invite);

  Future<void> _join() async {
    final invite = Invite.parse(_link.text);
    if (invite == null) {
      setState(() => _error = "That doesn't look like a whisperpoll invite link.");
      return;
    }
    _link.clear();
    setState(() => _error = null);
    await _open(invite);
  }

  Future<void> _create() async {
    final server = _server.text.trim().replaceAll(RegExp(r'/+$'), '');
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final response = await http.post(
        Uri.parse('$server/api/polls'),
        headers: {'content-type': 'application/json'},
        body: jsonEncode({'title': _title.text, 'name': _name.text}),
      );
      if (response.statusCode != 200) throw response.body;
      final created = jsonDecode(response.body) as Map<String, dynamic>;
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString('server', server);
      _title.clear();
      _name.clear();
      await _open(Invite(
        server: server,
        poll: created['id'] as String,
        token: created['token'] as String,
      ));
    } catch (e) {
      setState(() => _error = "Couldn't create the poll: $e");
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final polls = _polls;
    final running = [for (final p in polls ?? <(Invite, PollStatus)>[]) if (p.$2.running) p];
    return Scaffold(
      appBar: AppBar(title: const Text('whisperpoll')),
      body: RefreshIndicator(
        onRefresh: _load,
        child: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            Text(
              'Find a day that works — with votes nobody can read, not even the server, '
              'and a result everyone can check.',
              style: theme.textTheme.bodyLarge?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            const SizedBox(height: 16),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
              ),
            if (polls != null && polls.isNotEmpty)
              Card(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    ListTile(
                      title: const Text('Running polls'),
                      trailing: TextButton(
                        onPressed: _openHistory,
                        child: const Text('All your polls'),
                      ),
                    ),
                    if (running.isEmpty)
                      Padding(
                        padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
                        child: Text(
                          'Nothing running right now.',
                          style: TextStyle(color: theme.colorScheme.onSurfaceVariant),
                        ),
                      ),
                    for (final (invite, status) in running)
                      PollTile(invite: invite, status: status, onOpen: () => _open(invite)),
                  ],
                ),
              ),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text('Join with an invite link', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _link,
                      decoration: const InputDecoration(
                        hintText: 'https://…/p/…?t=…',
                        border: OutlineInputBorder(),
                      ),
                      keyboardType: TextInputType.url,
                      onSubmitted: (_) => _join(),
                    ),
                    const SizedBox(height: 12),
                    FilledButton.tonal(onPressed: _join, child: const Text('Join')),
                  ],
                ),
              ),
            ),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text('Start a poll', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _title,
                      decoration: const InputDecoration(
                        labelText: 'What are you planning?',
                        hintText: 'Team dinner',
                        border: OutlineInputBorder(),
                      ),
                    ),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _name,
                      decoration: const InputDecoration(
                        labelText: 'Your name',
                        border: OutlineInputBorder(),
                      ),
                    ),
                    ExpansionTile(
                      tilePadding: EdgeInsets.zero,
                      title: const Text('Server'),
                      children: [
                        TextField(
                          controller: _server,
                          keyboardType: TextInputType.url,
                          decoration: const InputDecoration(border: OutlineInputBorder()),
                        ),
                      ],
                    ),
                    FilledButton(
                      onPressed: _busy ? null : _create,
                      child: const Text('Create poll'),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
