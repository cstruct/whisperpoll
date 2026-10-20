import 'dart:io';
import 'dart:math';

import 'package:audioplayers/audioplayers.dart';
import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'maxi.dart';
import 'src/rust/api/muzak.dart' as rust;

/// Waiting music: procedurally composed lounge muzak (the same Rust composer
/// as the web client), a new piece after each one ends. It only plays while
/// the app is on screen, respects the silent switch, mixes with other audio,
/// and remembers being muted.
class Muzak extends ChangeNotifier {
  Muzak._();

  static final instance = Muzak._();
  static const _prefKey = 'muzak-muted';

  final _player = AudioPlayer(playerId: 'muzak');
  final _random = Random();
  AppLifecycleListener? _lifecycle;

  bool muted = false;

  /// The piece playing now, if any.
  String? title;
  bool _started = false;
  bool _foreground = false;
  int _file = 0;

  Future<void> init() async {
    final prefs = await SharedPreferences.getInstance();
    muted = prefs.getBool(_prefKey) ?? false;
    await _player.setAudioContext(AudioContext(
      // Ambient: quiet when the phone is on silent, and mixes with whatever
      // else is playing instead of stopping it.
      iOS: AudioContextIOS(category: AVAudioSessionCategory.ambient),
      android: const AudioContextAndroid(audioFocus: AndroidAudioFocus.none),
    ));
    await _player.setVolume(0.6);
    _player.onPlayerComplete.listen((_) => _next());
    _lifecycle = AppLifecycleListener(onShow: _shown, onHide: _hidden);
    // The app may have been woken in the background by a push: only play
    // once it's actually on screen.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (WidgetsBinding.instance.lifecycleState == AppLifecycleState.resumed) _shown();
    });
    notifyListeners();
  }

  void _shown() {
    _foreground = true;
    if (muted) return;
    _started ? _player.resume() : _next();
  }

  void _hidden() {
    _foreground = false;
    _player.pause();
  }

  Future<void> _next() async {
    if (muted || !_foreground) return;
    _started = true;
    final piece = await rust.composeMuzak(seed: _random.nextInt(1 << 32));
    // Two files in turn, so the one playing is never overwritten.
    final dir = await getTemporaryDirectory();
    final file = File('${dir.path}/muzak-${_file++ % 2}.wav');
    await file.writeAsBytes(piece.wav, flush: true);
    if (muted || !_foreground) return;
    await _player.play(DeviceFileSource(file.path, mimeType: 'audio/wav'));
    title = piece.title;
    notifyListeners();
  }

  Future<void> toggle() async {
    muted = !muted;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_prefKey, muted);
    if (muted) {
      await _player.pause();
    } else if (_started && _player.state == PlayerState.paused) {
      await _player.resume();
    } else {
      await _next();
    }
  }

  @override
  void dispose() {
    _lifecycle?.dispose();
    _player.dispose();
    super.dispose();
  }
}

/// The app bar's mute button: a record that spins while the muzak plays.
class MuzakButton extends StatelessWidget {
  const MuzakButton({super.key});

  @override
  Widget build(BuildContext context) {
    final muzak = Muzak.instance;
    return ListenableBuilder(
      listenable: muzak,
      builder: (context, _) {
        final playing = !muzak.muted && muzak.title != null;
        final label = muzak.muted
            ? 'Muzak is off. Turn it on'
            : muzak.title == null
                ? 'Mute the muzak'
                : 'Now playing: ${muzak.title}. Mute the muzak';
        final record = Icon(
          muzak.muted ? Icons.music_off : Icons.album,
          color: muzak.muted ? Maxi.lilac : Maxi.lime,
          size: 26,
        );
        return IconButton(
          tooltip: label,
          onPressed: muzak.toggle,
          icon: Semantics(
            label: label,
            toggled: !muzak.muted,
            child: playing ? Animated(Motion.spin, period: const Duration(seconds: 3), child: record) : record,
          ),
        );
      },
    );
  }
}
