import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'model.dart';
import 'push.dart';
import 'src/rust/api/poll.dart' as rust;
import 'storage.dart';

const _firstRetry = Duration(milliseconds: 1500);
const _maxRetry = Duration(seconds: 30);

/// A live connection to one poll, like the web page's: it reconnects with
/// backoff, notices when the server no longer has the poll, and does this
/// participant's trustee steps on its own as states arrive.
class PollConnection extends ChangeNotifier {
  PollConnection(this.invite);

  final Invite invite;
  PollView? view;
  String? error;
  bool connected = false;

  /// The server doesn't know this poll (or invite) any more.
  bool gone = false;

  WebSocketChannel? _socket;
  String? _identity;
  final _sent = <String>{};
  var _retry = _firstRetry;
  Timer? _retryTimer;
  bool _closed = false;
  bool _stepping = false;
  bool _stepAgain = false;
  DateTime _lastActivity = DateTime.now();

  static final _live = <PollConnection>{};

  static bool isLive(Invite invite) => _live.any((c) => c.invite.key == invite.key);

  /// Sends the device's push token to every open connection.
  static void registerDeviceEverywhere(String token) {
    for (final c in _live) {
      c._registerDevice(token);
    }
  }

  Future<void> start() async {
    _live.add(this);
    _identity = await Keys.identity(invite);
    await _open();
  }

  Future<void> _open() async {
    if (_closed || _paused) return;
    final socket = WebSocketChannel.connect(invite.socketUrl);
    try {
      await socket.ready;
    } catch (_) {
      await _neverOpened();
      return;
    }
    if (_closed) {
      await socket.sink.close();
      return;
    }
    _socket = socket;
    connected = true;
    _retry = _firstRetry;
    _sent.clear();
    _touch();
    notifyListeners();
    final token = PushService.token;
    if (token != null) _registerDevice(token);
    socket.stream.listen(
      _onMessage,
      onDone: () {
        if (_socket != socket) return; // Closed on purpose by pause().
        connected = false;
        _socket = null;
        notifyListeners();
        _scheduleRetry(_firstRetry);
      },
      onError: (_) {},
    );
  }

  /// A connection that never opened: the server may be down, or may have
  /// forgotten the poll. Socket errors hide the status, so ask separately.
  Future<void> _neverOpened() async {
    try {
      final response = await http.get(invite.checkUrl).timeout(const Duration(seconds: 10));
      if (response.statusCode == 404) {
        gone = true;
        notifyListeners();
        return;
      }
    } catch (_) {}
    _scheduleRetry(_retry);
    _retry = Duration(milliseconds: min(_retry.inMilliseconds * 2, _maxRetry.inMilliseconds));
  }

  void _scheduleRetry(Duration delay) {
    if (_closed || _paused) return;
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, _open);
  }

  void _onMessage(dynamic data) {
    _touch();
    final msg = jsonDecode(data as String);
    if (msg is Map && msg['State'] != null) {
      final state = msg['State'] as Map<String, dynamic>;
      view = PollView(state, jsonEncode(state));
      if (invite.title != view!.title) {
        invite.title = view!.title;
        Invites.save(invite);
      }
      notifyListeners();
      _step();
    } else if (msg is Map && msg['Error'] != null) {
      error = msg['Error'] as String;
      notifyListeners();
    }
  }

  /// Runs the shared automatic steps (in Rust) on the latest state.
  Future<void> _step() async {
    if (_stepping) {
      _stepAgain = true;
      return;
    }
    _stepping = true;
    try {
      do {
        _stepAgain = false;
        final v = view;
        if (v == null || _identity == null) break;
        final step = await rust.nextStep(
          viewJson: v.raw,
          identityHex: _identity!,
          sent: _sent.toList(),
        );
        if (step != null) {
          _sent.add(step.key);
          sendJson(step.msgJson);
          _touch();
        }
      } while (_stepAgain);
    } catch (e) {
      error = '$e';
      notifyListeners();
    } finally {
      _stepping = false;
    }
  }

  void _touch() => _lastActivity = DateTime.now();

  void _registerDevice(String token) => send({
        'RegisterDevice': {'platform': 'Apns', 'token': token}
      });

  /// Sends a message given as JSON-ready Dart values.
  void send(Object message) => sendJson(jsonEncode(message));

  void sendJson(String json) => _socket?.sink.add(json);

  void clearError() {
    error = null;
    notifyListeners();
  }

  /// For background wake-ups: resolves once this device has nothing left to
  /// do (no new state or step for a moment), or after [timeout].
  Future<void> settle({Duration timeout = const Duration(seconds: 20)}) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline) && !gone) {
      await Future<void>.delayed(const Duration(milliseconds: 300));
      final quiet = DateTime.now().difference(_lastActivity) > const Duration(seconds: 2);
      if (view != null && !_stepping && quiet) return;
    }
  }

  bool _paused = false;

  /// The app went to the background: disconnect, so the server sees this
  /// person as offline and pushes to them when needed. (iOS suspends the app
  /// without closing its socket otherwise.) The last state stays on screen.
  Future<void> pause() async {
    if (_paused || _closed) return;
    _paused = true;
    _retryTimer?.cancel();
    final socket = _socket;
    _socket = null;
    connected = false;
    notifyListeners();
    await socket?.sink.close();
  }

  Future<void> resume() async {
    if (!_paused || _closed) return;
    _paused = false;
    await _open();
  }

  Future<void> close() async {
    _closed = true;
    _live.remove(this);
    _retryTimer?.cancel();
    await _socket?.sink.close();
  }

  @override
  void dispose() {
    close();
    super.dispose();
  }
}
