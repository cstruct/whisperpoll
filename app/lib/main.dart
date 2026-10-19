import 'dart:async';

import 'package:app_links/app_links.dart';
import 'package:flutter/material.dart';

import 'home_screen.dart';
import 'poll_screen.dart';
import 'push.dart';
import 'src/rust/frb_generated.dart';
import 'storage.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  // Before anything else: a background wake-up may be why the app launched.
  await PushService.init();
  runApp(const WhisperpollApp());
}

class WhisperpollApp extends StatefulWidget {
  const WhisperpollApp({super.key});

  @override
  State<WhisperpollApp> createState() => _WhisperpollAppState();
}

class _WhisperpollAppState extends State<WhisperpollApp> {
  final _navigator = GlobalKey<NavigatorState>();
  StreamSubscription<Uri>? _links;

  @override
  void initState() {
    super.initState();
    // Invite links opened from elsewhere (whisperpoll://p/…).
    _links = AppLinks().uriLinkStream.listen((uri) => _openLink(uri.toString()));
    PushService.opened.addListener(_openFromNotification);
  }

  @override
  void dispose() {
    _links?.cancel();
    PushService.opened.removeListener(_openFromNotification);
    super.dispose();
  }

  Future<void> _openLink(String link) async {
    final invite = Invite.parse(link);
    if (invite != null) await openInvite(invite);
  }

  Future<void> _openFromNotification() async {
    final poll = PushService.opened.value;
    if (poll == null) return;
    PushService.opened.value = null;
    final invites = await Invites.forPoll(poll);
    if (invites.isNotEmpty) await openInvite(invites.first);
  }

  Future<void> openInvite(Invite invite) async {
    await Invites.save(invite);
    await PushService.requestPermission();
    await _navigator.currentState?.push(
      MaterialPageRoute<void>(builder: (_) => PollScreen(invite: invite)),
    );
  }

  @override
  Widget build(BuildContext context) {
    const seed = Color(0xFF3B5BDB);
    return MaterialApp(
      title: 'whisperpoll',
      navigatorKey: _navigator,
      theme: ThemeData(colorSchemeSeed: seed, useMaterial3: true),
      darkTheme: ThemeData(
        colorSchemeSeed: seed,
        brightness: Brightness.dark,
        useMaterial3: true,
      ),
      home: HomeScreen(onOpen: openInvite),
    );
  }
}
