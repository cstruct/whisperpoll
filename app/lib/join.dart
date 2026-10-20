import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:http/http.dart' as http;
import 'package:share_plus/share_plus.dart';

import 'maxi.dart';
import 'src/rust/api/qr.dart' as rust;
import 'storage.dart';

/// The open invite's QR code, stylized like the web client's: dark modules
/// in three tones, exact finder patterns, and a star sticker in the middle
/// (the code has room to spare for it).
class QrSticker extends StatelessWidget {
  const QrSticker(this.link, {super.key, this.size = 240});
  final String link;
  final double size;

  @override
  Widget build(BuildContext context) {
    final art = rust.qrArt(text: link);
    if (art == null) return const SizedBox.shrink();
    return Semantics(
      label: 'QR code that opens the invite',
      image: true,
      child: Transform.rotate(
        angle: -0.035,
        child: Stack(clipBehavior: Clip.none, children: [
          Container(
            padding: const EdgeInsets.fromLTRB(10, 10, 10, 8),
            decoration: BoxDecoration(
              color: Maxi.paper,
              border: Border.all(color: Maxi.ink, width: 3),
              boxShadow: Maxi.hard(Maxi.pink, 6),
            ),
            child: Column(mainAxisSize: MainAxisSize.min, children: [
              SizedBox.square(dimension: size, child: CustomPaint(painter: _QrPainter(art))),
              const SizedBox(height: 6),
              SizedBox(
                width: size,
                child: Text(
                  'Anyone who scans this picks their own name and joins.',
                  style: Maxi.pixel(18, Maxi.ink),
                ),
              ),
            ]),
          ),
          const Positioned(
            top: -14,
            left: -10,
            child: Animated(Motion.blink, child: Tag('Scan me!!', textColor: Maxi.lime, tilt: -8)),
          ),
          const Positioned(top: -16, right: -14, child: SpinningStar(size: 34)),
        ]),
      ),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.art);
  final rust.QrArt art;

  static const _tones = [Color(0xFF000000), Color(0xFF1A0B3B), Color(0xFF3B0F73)];

  @override
  void paint(Canvas canvas, Size size) {
    final edge = (art.size + 2 * art.quiet).toDouble();
    final m = size.width / edge;
    Rect at(num x, num y, num w, num h) =>
        Rect.fromLTWH((x + art.quiet) * m, (y + art.quiet) * m, w * m, h * m);
    canvas.drawRect(Offset.zero & size, Paint()..color = Maxi.paper);
    final tones = [for (final c in _tones) Paint()..color = c];
    for (final p in art.pieces) {
      switch (p.kind) {
        case 0:
          canvas.drawRect(at(p.x, p.y, 7, 7), Paint()..color = Maxi.ink);
          canvas.drawRect(at(p.x + 1, p.y + 1, 5, 5), Paint()..color = Maxi.paper);
          canvas.drawRect(at(p.x + 2, p.y + 2, 3, 3), tones[1]);
        case 1:
          canvas.drawRect(at(p.x + 0.08, p.y + 0.08, 0.84, 0.84), tones[p.n]);
        default:
          final badge = at(p.x, p.y, p.n, p.n);
          canvas.drawRect(badge, Paint()..color = Maxi.paper);
          _star(canvas, badge.deflate(badge.width * 0.04));
      }
    }
  }

  void _star(Canvas canvas, Rect box) {
    final c = box.center;
    final r = box.width / 2;
    final path = Path();
    for (var i = 0; i < 10; i++) {
      final a = -3.14159265 / 2 + i * 3.14159265 / 5;
      final radius = i.isEven ? r : r * 0.45;
      final p = c + Offset.fromDirection(a, radius);
      i == 0 ? path.moveTo(p.dx, p.dy) : path.lineTo(p.dx, p.dy);
    }
    path.close();
    canvas.drawPath(path, Paint()..color = Maxi.lime);
    canvas.drawPath(
      path,
      Paint()
        ..color = Maxi.ink
        ..style = PaintingStyle.stroke
        ..strokeWidth = box.width * 0.06
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.art != art;
}

/// The creator's QR sticker with ways to share or retire it.
class OpenInvite extends StatelessWidget {
  const OpenInvite({
    super.key,
    required this.server,
    required this.poll,
    required this.code,
    required this.title,
    required this.onNewCode,
  });

  final String server;
  final String poll;
  final String code;
  final String title;
  final VoidCallback onNewCode;

  @override
  Widget build(BuildContext context) {
    final link = rust.joinLink(server: server, poll: poll, code: code);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 18),
        Text('…or let them scan', style: Maxi.display(16, Maxi.ink)),
        const SizedBox(height: 22),
        Center(child: QrSticker(link)),
        const SizedBox(height: 20),
        Wrap(spacing: 10, runSpacing: 10, children: [
          Pop(
            color: Maxi.ink,
            child: OutlinedButton.icon(
              onPressed: () => SharePlus.instance.share(
                ShareParams(text: 'Join $title on whisperpoll: $link', subject: title),
              ),
              icon: const Icon(Icons.ios_share, size: 18),
              label: const Text('Share join link'),
            ),
          ),
          Pop(
            color: Maxi.ink,
            child: OutlinedButton(onPressed: onNewCode, child: const Text('New code')),
          ),
        ]),
        const SizedBox(height: 12),
        const Text(
          'A new code retires this QR code: anyone who has it can\'t join with it anymore.',
          style: TextStyle(fontSize: 13),
        ),
      ],
    );
  }
}

/// Joins through an open invite: shows what it's for, asks for a name, and
/// returns the new participant's invite (or null if they backed out).
Future<Invite?> joinWithLink(BuildContext context, JoinLink link) async {
  Map<String, dynamic>? invitation;
  try {
    final response = await http.get(link.invitationUrl).timeout(const Duration(seconds: 10));
    if (response.statusCode == 200) invitation = jsonDecode(response.body) as Map<String, dynamic>;
  } catch (_) {}
  if (!context.mounted) return null;
  return showDialog<Invite>(
    context: context,
    builder: (context) => _JoinDialog(link: link, invitation: invitation),
  );
}

class _JoinDialog extends StatefulWidget {
  const _JoinDialog({required this.link, required this.invitation});
  final JoinLink link;
  final Map<String, dynamic>? invitation;

  @override
  State<_JoinDialog> createState() => _JoinDialogState();
}

class _JoinDialogState extends State<_JoinDialog> {
  final _name = TextEditingController();
  String? _error;
  bool _busy = false;

  Future<void> _join() async {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final response = await http.post(
        widget.link.joinUrl,
        headers: {'content-type': 'application/json'},
        body: jsonEncode({'code': widget.link.code, 'name': name}),
      );
      if (response.statusCode != 200) throw response.body;
      final created = jsonDecode(response.body) as Map<String, dynamic>;
      if (!mounted) return;
      Navigator.of(context).pop(Invite(
        server: widget.link.server,
        poll: created['id'] as String,
        token: created['token'] as String,
        title: widget.invitation?['title'] as String?,
      ));
    } catch (e) {
      setState(() {
        _error = '$e';
        _busy = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final inv = widget.invitation;
    return Dialog(
      backgroundColor: Colors.transparent,
      insetPadding: const EdgeInsets.all(20),
      child: ZigCard(
        color: inv == null ? Maxi.orange : Maxi.cyan,
        title: inv == null ? 'This invite no longer works' : "You're invited!!",
        icon: inv == null ? null : const Animated(Motion.bob, child: Icon(Icons.mail_outline, size: 28)),
        child: inv == null
            ? Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                const Text(
                  'Voting may have closed, the poll may be over a week old, or the creator made a new '
                  'QR code. Ask them for a fresh one.',
                ),
                const SizedBox(height: 14),
                Pop(
                  child: FilledButton(onPressed: () => Navigator.of(context).pop(), child: const Text('OK')),
                ),
              ])
            : Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                Text('${inv['from']} invites you to ${inv['title']}.',
                    style: const TextStyle(fontWeight: FontWeight.w700, fontSize: 16)),
                const SizedBox(height: 6),
                const Text('Pick the name everyone will see. Your phone makes its own key; nobody can read your vote.'),
                const SizedBox(height: 14),
                TextField(
                  controller: _name,
                  autofocus: true,
                  style: Maxi.pixel(22, Maxi.ink),
                  inputFormatters: [LengthLimitingTextInputFormatter(40)],
                  textCapitalization: TextCapitalization.words,
                  decoration: const InputDecoration(labelText: 'Your name', hintText: 'Alex'),
                  onSubmitted: (_) => _join(),
                ),
                if (_error != null) ...[
                  const SizedBox(height: 8),
                  Text(_error!, style: const TextStyle(color: Maxi.bad, fontWeight: FontWeight.w700)),
                ],
                const SizedBox(height: 14),
                Pop(
                  child: FilledButton(onPressed: _busy ? null : _join, child: const Text('Join the poll')),
                ),
                TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Not now')),
              ]),
      ),
    );
  }
}
