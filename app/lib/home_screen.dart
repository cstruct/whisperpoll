import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

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
  List<Invite> _invites = [];
  final _link = TextEditingController();
  final _title = TextEditingController();
  final _name = TextEditingController();
  final _server = TextEditingController(text: defaultServer);
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    final invites = await Invites.all();
    setState(() {
      _invites = invites;
      _server.text = prefs.getString('server') ?? defaultServer;
    });
  }

  Future<void> _open(Invite invite) async {
    await widget.onOpen(invite);
    await _load();
  }

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
    return Scaffold(
      appBar: AppBar(title: const Text('whisperpoll')),
      body: ListView(
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
          if (_invites.isNotEmpty)
            Card(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const ListTile(title: Text('Your polls')),
                  for (final invite in _invites)
                    Dismissible(
                      key: ValueKey(invite.key),
                      direction: DismissDirection.endToStart,
                      background: Container(
                        color: theme.colorScheme.errorContainer,
                        alignment: Alignment.centerRight,
                        padding: const EdgeInsets.only(right: 16),
                        child: const Icon(Icons.delete_outline),
                      ),
                      onDismissed: (_) async {
                        await Invites.remove(invite);
                        await _load();
                      },
                      child: ListTile(
                        title: Text(invite.title ?? 'Poll ${invite.poll}'),
                        subtitle: Text(Uri.parse(invite.server).host),
                        trailing: const Icon(Icons.chevron_right),
                        onTap: () => _open(invite),
                      ),
                    ),
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
    );
  }
}
