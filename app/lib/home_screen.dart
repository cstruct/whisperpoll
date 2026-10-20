import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import 'history.dart';
import 'join.dart';
import 'maxi.dart';
import 'muzak.dart';
import 'scan.dart';
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
    final open = JoinLink.parse(_link.text);
    if (invite == null && open != null) {
      _link.clear();
      setState(() => _error = null);
      final joined = await joinWithLink(context, open);
      if (joined != null) await _open(joined);
      return;
    }
    if (invite == null) {
      setState(() => _error = "That doesn't look like a whisperpoll invite link.");
      return;
    }
    _link.clear();
    setState(() => _error = null);
    await _open(invite);
  }

  /// Scans a QR code and joins with it like a pasted link.
  Future<void> _scan() async {
    final link = await ScanScreen.open(context);
    if (link == null || !mounted) return;
    _link.text = link;
    await _join();
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
    final polls = _polls;
    final running = [for (final p in polls ?? <(Invite, PollStatus)>[]) if (p.$2.running) p];
    return Scaffold(
      appBar: AppBar(
        title: const Text('~*~ whisperpoll ~*~'),
        actions: const [MuzakButton(), SizedBox(width: 8)],
      ),
      body: Starfield(
        child: Column(children: [
          const Ticker('Votes nobody can read ★ not even the server ★ a result everyone can check'),
          Expanded(
            child: RefreshIndicator(
              onRefresh: _load,
              child: ListView(
                padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
                children: [
                  Text('Find a time that works', style: Maxi.chroma(30)),
                  const SizedBox(height: 10),
                  const Text(
                    'With votes nobody can read, not even the server, and a result everyone can check.',
                    style: TextStyle(fontSize: 16, fontWeight: FontWeight.w700),
                  ),
                  const SizedBox(height: 20),
                  if (_error != null)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 16, right: 4),
                      child: Sticker(
                        color: Maxi.orange,
                        child: Text(_error!, style: const TextStyle(color: Maxi.ink, fontWeight: FontWeight.w700)),
                      ),
                    ),
                  if (polls != null && polls.isNotEmpty)
                    ZigCard(
                      title: 'Running polls',
                      icon: const SpinningStar(size: 24),
                      trailing: TextButton(onPressed: _openHistory, child: const Text('All your polls')),
                      padding: const EdgeInsets.fromLTRB(10, 26, 10, 22),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          if (running.isEmpty)
                            const Padding(
                              padding: EdgeInsets.symmetric(horizontal: 8),
                              child: Text('Nothing running right now.'),
                            ),
                          for (final (invite, status) in running)
                            PollTile(invite: invite, status: status, onOpen: () => _open(invite)),
                        ],
                      ),
                    ),
                  ZigCard(
                    title: 'Join with an invite link',
                    color: Maxi.yellow,
                    icon: const Animated(Motion.bob, child: Icon(Icons.mail_outline, size: 28)),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        TextField(
                          controller: _link,
                          decoration: const InputDecoration(hintText: 'https://…/p/…?t=…'),
                          style: Maxi.pixel(22, Maxi.ink),
                          keyboardType: TextInputType.url,
                          onSubmitted: (_) => _join(),
                        ),
                        const SizedBox(height: 14),
                        Pop(color: Maxi.cyan, child: FilledButton(onPressed: _join, child: const Text('Join'))),
                        const SizedBox(height: 14),
                        Pop(
                          color: Maxi.ink,
                          child: OutlinedButton.icon(
                            onPressed: _scan,
                            icon: const Animated(Motion.pulse, child: Icon(Icons.qr_code_scanner)),
                            label: const Text('Scan QR code'),
                          ),
                        ),
                      ],
                    ),
                  ),
                  ZigCard(
                    title: 'Start a poll',
                    color: Maxi.pink,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        TextField(
                          controller: _title,
                          style: Maxi.pixel(22, Maxi.ink),
                          decoration: const InputDecoration(
                            labelText: 'What are you planning?',
                            hintText: 'Team dinner',
                          ),
                        ),
                        const SizedBox(height: 12),
                        TextField(
                          controller: _name,
                          style: Maxi.pixel(22, Maxi.ink),
                          decoration: const InputDecoration(labelText: 'Your name'),
                        ),
                        ExpansionTile(
                          tilePadding: EdgeInsets.zero,
                          title: Text('Server', style: Maxi.display(12, Maxi.ink)),
                          children: [
                            TextField(
                              controller: _server,
                              style: Maxi.pixel(22, Maxi.ink),
                              keyboardType: TextInputType.url,
                            ),
                          ],
                        ),
                        Pop(
                          color: Maxi.cyan,
                          child: FilledButton(
                            onPressed: _busy ? null : _create,
                            child: const Text('Create poll'),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const Badges(),
                ],
              ),
            ),
          ),
        ]),
      ),
    );
  }
}
