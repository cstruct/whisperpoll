import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'connection.dart';
import 'storage.dart';

/// Push notifications, via the native `PushBridge` (ios/Runner): the device
/// token, background wake-ups and taps.
class PushService {
  static const _channel = MethodChannel('whisperpoll/push');

  static String? token;

  /// The poll id of a tapped notification, for the app to open.
  static final opened = ValueNotifier<String?>(null);

  static Future<void> init() async {
    _channel.setMethodCallHandler((call) async {
      switch (call.method) {
        case 'token':
          token = call.arguments as String;
          PollConnection.registerDeviceEverywhere(token!);
          await _registerWithAllPolls();
          return null;
        case 'wake':
          await _wake(call.arguments as String);
          return true;
        case 'open':
          opened.value = call.arguments as String;
          return null;
      }
      return null;
    });
    await _channel.invokeMethod('ready');
  }

  /// Asks to show notifications. Background wake-ups work without it.
  static Future<void> requestPermission() async {
    try {
      await _channel.invokeMethod('requestPermission');
    } on PlatformException catch (_) {}
  }

  /// Woken in the background for a poll: connect, let the automatic steps
  /// run (deal, check or decrypt), then disconnect.
  static Future<void> _wake(String poll) async {
    for (final invite in await Invites.forPoll(poll)) {
      final connection = PollConnection(invite);
      await connection.start();
      await connection.settle();
      await connection.close();
    }
    await _channel.invokeMethod('clearWake', poll);
  }

  /// Tells every joined poll's server where to push, so pushes reach polls
  /// that aren't open on this device.
  static Future<void> _registerWithAllPolls() async {
    for (final invite in await Invites.all()) {
      // Open ones register the device themselves.
      if (PollConnection.isLive(invite)) continue;
      final connection = PollConnection(invite);
      await connection.start();
      await connection.settle(timeout: const Duration(seconds: 5));
      await connection.close();
    }
  }
}
