import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/rust/api/poll.dart' as rust;

/// One participant's link into a poll: the server, the poll and the secret
/// invite token. The same link the web app uses.
class Invite {
  Invite({required this.server, required this.poll, required this.token, this.title});

  final String server;
  final String poll;
  final String token;
  String? title;

  String get key => '$server|$poll|$token';

  Uri get webUrl => Uri.parse('$server/p/$poll').replace(queryParameters: {'t': token});

  Uri get socketUrl {
    final base = Uri.parse(server);
    return base.replace(
      scheme: base.scheme == 'https' ? 'wss' : 'ws',
      path: '/ws/$poll',
      queryParameters: {'t': token},
    );
  }

  Uri get checkUrl =>
      Uri.parse('$server/api/polls/$poll/check').replace(queryParameters: {'t': token});

  /// Accepts the web link (`https://host/p/<poll>?t=<token>`) or the app's
  /// own (`whisperpoll://p/<poll>?t=<token>&server=<https://host>`).
  static Invite? parse(String text) {
    final uri = Uri.tryParse(text.trim());
    if (uri == null) return null;
    final token = uri.queryParameters['t'];
    if (token == null || token.isEmpty) return null;
    if (uri.scheme == 'whisperpoll') {
      final server = uri.queryParameters['server'];
      final poll = uri.pathSegments.firstOrNull;
      if (uri.host != 'p' || server == null || poll == null) return null;
      return Invite(server: server, poll: poll, token: token);
    }
    if (uri.scheme != 'https' && uri.scheme != 'http') return null;
    final segments = uri.pathSegments;
    if (segments.length != 2 || segments[0] != 'p') return null;
    return Invite(server: uri.origin, poll: segments[1], token: token);
  }

  Map<String, dynamic> toJson() =>
      {'server': server, 'poll': poll, 'token': token, 'title': title};

  static Invite fromJson(Map<String, dynamic> j) => Invite(
        server: j['server'] as String,
        poll: j['poll'] as String,
        token: j['token'] as String,
        title: j['title'] as String?,
      );
}

/// The polls this device has joined, newest first.
class Invites {
  static const _key = 'invites';

  static Future<List<Invite>> all() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getStringList(_key) ?? [];
    return [for (final j in raw) Invite.fromJson(jsonDecode(j) as Map<String, dynamic>)];
  }

  static Future<void> save(Invite invite) async {
    final list = await all();
    list.removeWhere((i) => i.key == invite.key);
    list.insert(0, invite);
    await _write(list);
  }

  static Future<void> remove(Invite invite) async {
    final list = await all();
    list.removeWhere((i) => i.key == invite.key);
    await _write(list);
  }

  static Future<List<Invite>> forPoll(String poll) async =>
      [for (final i in await all()) if (i.poll == poll) i];

  static Future<void> _write(List<Invite> list) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setStringList(_key, [for (final i in list) jsonEncode(i.toJson())]);
  }
}

/// This device's secrets and memories per invite: the trustee key (in the
/// Keychain), ballot receipts and the last tiers voted.
class Keys {
  static const _secure = FlutterSecureStorage(
    // Background wake-ups can happen while the phone is locked.
    iOptions: IOSOptions(accessibility: KeychainAccessibility.first_unlock_this_device),
  );

  static final _identities = <String, Future<String>>{};

  /// This invite's trustee key, created on first use. Loaded once per run:
  /// two connections starting together must not each create a key.
  static Future<String> identity(Invite invite) =>
      _identities.putIfAbsent(invite.key, () => _loadIdentity(invite));

  static Future<String> _loadIdentity(Invite invite) async {
    final key = 'identity:${invite.key}';
    final existing = await _secure.read(key: key);
    if (existing != null) return existing;
    final fresh = rust.newIdentity();
    await _secure.write(key: key, value: fresh);
    return fresh;
  }

  static Future<void> saveReceipt(Invite invite, int round, String hex) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('receipt:${invite.key}:$round', hex);
  }

  /// Receipts for rounds 1..[rounds], `null` where none was cast.
  static Future<List<String?>> receipts(Invite invite, int rounds) async {
    final prefs = await SharedPreferences.getInstance();
    return [for (var r = 1; r <= rounds; r++) prefs.getString('receipt:${invite.key}:$r')];
  }

  static Future<void> saveChoices(Invite invite, List<int?> choices) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('choices:${invite.key}', jsonEncode(choices));
  }

  static Future<List<int?>?> choices(Invite invite) async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString('choices:${invite.key}');
    return raw == null ? null : (jsonDecode(raw) as List).cast<int?>();
  }
}
