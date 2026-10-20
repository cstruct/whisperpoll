import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import 'maxi.dart';
import 'storage.dart';

/// Scans a whisperpoll QR code — an open invite (`/j/…`) or a personal link
/// (`/p/…`) — and returns its text. Other codes get a nudge, not a result.
class ScanScreen extends StatefulWidget {
  const ScanScreen({super.key});

  /// Opens the scanner; the link it read, or null.
  static Future<String?> open(BuildContext context) =>
      Navigator.of(context).push<String>(MaterialPageRoute(builder: (_) => const ScanScreen()));

  @override
  State<ScanScreen> createState() => _ScanScreenState();
}

class _ScanScreenState extends State<ScanScreen> {
  final _controller = MobileScannerController(
    formats: const [BarcodeFormat.qrCode],
    detectionSpeed: DetectionSpeed.noDuplicates,
  );
  bool _done = false;
  String? _ignored;

  static bool _ours(String text) => JoinLink.parse(text) != null || Invite.parse(text) != null;

  void _detected(BarcodeCapture capture) {
    if (_done) return;
    for (final code in capture.barcodes) {
      final text = code.rawValue;
      if (text == null) continue;
      if (_ours(text)) {
        _done = true;
        Navigator.of(context).pop(text);
        return;
      }
      if (_ignored != text) setState(() => _ignored = text);
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('~*~ scan ~*~')),
      body: Starfield(
        child: Column(children: [
          const Ticker('Point at a whisperpoll QR code ★ the app does the rest'),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(24, 28, 24, 24),
              children: [
                Text('Scan the invite', style: Maxi.chroma(28)),
                const SizedBox(height: 22),
                _Viewfinder(
                  child: MobileScanner(
                    controller: _controller,
                    onDetect: _detected,
                    errorBuilder: (context, error) => _CameraProblem(error),
                    placeholderBuilder: (context) => Center(
                      child: Text('Starting the camera…', style: Maxi.pixel(24, Maxi.lime)),
                    ),
                  ),
                ),
                const SizedBox(height: 26),
                if (_ignored != null)
                  const Sticker(
                    color: Maxi.orange,
                    tilt: -1,
                    child: Text(
                      "That's not a whisperpoll code. Look for the one with the star in the middle.",
                      style: TextStyle(color: Maxi.ink, fontWeight: FontWeight.w700),
                    ),
                  )
                else
                  const Text(
                    'Hold the QR code inside the frame. Invites with a star in the middle let you '
                    'pick your own name.',
                    style: TextStyle(fontWeight: FontWeight.w700),
                  ),
                const Badges(),
              ],
            ),
          ),
        ]),
      ),
    );
  }
}

/// A square camera view in a tilted sticker frame, with a spinning star.
class _Viewfinder extends StatelessWidget {
  const _Viewfinder({required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => Transform.rotate(
        angle: -0.025,
        child: Stack(clipBehavior: Clip.none, children: [
          Container(
            decoration: BoxDecoration(
              color: Maxi.ink,
              border: Border.all(color: Maxi.ink, width: 4),
              boxShadow: Maxi.hard(Maxi.pink, 8),
            ),
            child: AspectRatio(aspectRatio: 1, child: ClipRect(child: child)),
          ),
          const Positioned(
            top: -14,
            left: -10,
            child: Animated(Motion.blink, child: Tag('Scanning!!', textColor: Maxi.lime, tilt: -8)),
          ),
          const Positioned(top: -18, right: -16, child: SpinningStar(size: 38)),
        ]),
      );
}

/// What to do when there's no camera picture: no permission, no camera.
class _CameraProblem extends StatelessWidget {
  const _CameraProblem(this.error);
  final MobileScannerException error;

  @override
  Widget build(BuildContext context) {
    final (title, body) = switch (error.errorCode) {
      MobileScannerErrorCode.permissionDenied => (
          'No camera access',
          'Allow camera access for whisperpoll in Settings to scan invites. '
              'Or paste the invite link on the start screen instead.',
        ),
      MobileScannerErrorCode.unsupported => (
          'No camera here',
          "This device can't scan. Paste the invite link on the start screen instead.",
        ),
      _ => (
          "The camera didn't start",
          'Try again, or paste the invite link on the start screen instead.',
        ),
    };
    return ColoredBox(
      color: Maxi.night,
      child: Center(
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: ZigCard(
            color: Maxi.orange,
            title: title,
            icon: const Animated(Motion.wiggle, child: Icon(Icons.no_photography_outlined, size: 26)),
            child: Text(body),
          ),
        ),
      ),
    );
  }
}
