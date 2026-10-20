// The maximalist look, shared by every screen (the web client's style.css
// is the same design): a starfield, loud flat colors, black outlines with
// hard shadows, zigzag cards, a scrolling ticker, and icons that spin, bob
// and blink. Animations stop when the system asks for reduced motion.

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

abstract final class Maxi {
  static const night = Color(0xFF1A0B3B);
  static const ink = Colors.black;
  static const paper = Colors.white;
  static const lime = Color(0xFFC6FF00);
  static const pink = Color(0xFFFF2BD6);
  static const cyan = Color(0xFF00F0FF);
  static const yellow = Color(0xFFFFE600);
  static const orange = Color(0xFFFF6A00);
  static const lilac = Color(0xFFCFC4FF);
  static const inkSoft = Color(0xFF2A2142);
  static const ok = Color(0xFF08561A);
  static const bad = Color(0xFF9C0019);

  static TextStyle display([double size = 18, Color? color]) =>
      GoogleFonts.bungee(fontSize: size, color: color, height: 1.1);

  static TextStyle pixel([double size = 22, Color? color]) =>
      GoogleFonts.vt323(fontSize: size, color: color, height: 1.0);

  /// The poll title: two offset shadows, pink and cyan.
  static TextStyle chroma([double size = 30]) => display(size, paper).copyWith(shadows: const [
        Shadow(offset: Offset(3, 3), color: pink),
        Shadow(offset: Offset(-2, -2), color: cyan),
      ]);

  static List<BoxShadow> hard([Color color = ink, double d = 4]) =>
      [BoxShadow(color: color, offset: Offset(d, d))];

  static ThemeData theme() {
    final scheme = const ColorScheme.dark(
      primary: lime,
      onPrimary: ink,
      secondary: pink,
      onSecondary: ink,
      tertiary: cyan,
      surface: night,
      onSurface: paper,
      onSurfaceVariant: lilac,
      error: orange,
      onError: ink,
      errorContainer: orange,
      onErrorContainer: ink,
      primaryContainer: lime,
      onPrimaryContainer: ink,
      outline: ink,
    );
    final text = GoogleFonts.spaceGroteskTextTheme(ThemeData.dark().textTheme)
        .apply(bodyColor: paper, displayColor: paper);
    const square = RoundedRectangleBorder(borderRadius: BorderRadius.zero);
    const border = BorderSide(color: ink, width: 3);
    final label = display(13);
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: night,
      textTheme: text,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
      appBarTheme: AppBarTheme(
        backgroundColor: night,
        foregroundColor: lime,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        titleTextStyle: pixel(28, lime),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: ink,
          foregroundColor: lime,
          disabledBackgroundColor: const Color(0xFF555555),
          disabledForegroundColor: const Color(0xFFBBBBBB),
          shape: square,
          side: border,
          minimumSize: const Size(48, 48),
          textStyle: label,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          backgroundColor: paper,
          foregroundColor: ink,
          shape: square,
          side: border,
          minimumSize: const Size(44, 46),
          textStyle: label.copyWith(fontSize: 12),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(foregroundColor: ink, textStyle: label.copyWith(fontSize: 12)),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: paper,
        hintStyle: pixel(22, const Color(0xFF6B6480)),
        labelStyle: display(12, ink),
        floatingLabelStyle: display(12, ink),
        contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        border: const OutlineInputBorder(borderRadius: BorderRadius.zero, borderSide: border),
        enabledBorder: const OutlineInputBorder(borderRadius: BorderRadius.zero, borderSide: border),
        focusedBorder: const OutlineInputBorder(
          borderRadius: BorderRadius.zero,
          borderSide: BorderSide(color: pink, width: 3),
        ),
      ),
      textSelectionTheme: const TextSelectionThemeData(cursorColor: ink),
      progressIndicatorTheme: const ProgressIndicatorThemeData(
        color: pink,
        linearTrackColor: paper,
        linearMinHeight: 14,
      ),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: yellow,
        contentTextStyle: text.bodyMedium?.copyWith(color: ink, fontWeight: FontWeight.w700),
        actionTextColor: ink,
        shape: const RoundedRectangleBorder(side: border),
        behavior: SnackBarBehavior.floating,
      ),
      popupMenuTheme: PopupMenuThemeData(
        color: paper,
        textStyle: pixel(22, ink),
        shape: const RoundedRectangleBorder(side: border),
      ),
      dropdownMenuTheme: DropdownMenuThemeData(textStyle: pixel(22, ink)),
      dividerTheme: const DividerThemeData(color: ink, thickness: 2),
      expansionTileTheme: const ExpansionTileThemeData(
        shape: Border(),
        collapsedShape: Border(),
        iconColor: ink,
        collapsedIconColor: ink,
      ),
      datePickerTheme: const DatePickerThemeData(backgroundColor: paper),
    );
  }

  /// The theme inside a colored card: black text on a light ground.
  static ThemeData onLight(ThemeData theme) => theme.copyWith(
        colorScheme: theme.colorScheme.copyWith(
          onSurface: ink,
          onSurfaceVariant: inkSoft,
          primary: ink,
          onPrimary: lime,
        ),
        textTheme: theme.textTheme.apply(bodyColor: ink, displayColor: ink),
        iconTheme: const IconThemeData(color: ink),
        listTileTheme: const ListTileThemeData(textColor: ink, iconColor: ink),
        dropdownMenuTheme: theme.dropdownMenuTheme,
        canvasColor: paper,
      );
}

/// The night sky behind every screen: three offset grids of colored dots.
class Starfield extends StatelessWidget {
  const Starfield({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => CustomPaint(painter: _StarPainter(), child: child);
}

class _StarPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = Maxi.night);
    void grid(double step, Offset offset, Color color, double r) {
      final paint = Paint()..color = color;
      for (var y = offset.dy; y < size.height; y += step) {
        for (var x = offset.dx; x < size.width; x += step) {
          canvas.drawCircle(Offset(x, y), r, paint);
        }
      }
    }

    grid(41, Offset.zero, Maxi.yellow, 1.3);
    grid(29, const Offset(13, 19), Maxi.cyan, 1.1);
    grid(53, const Offset(27, 7), Maxi.pink, 1.1);
  }

  @override
  bool shouldRepaint(_StarPainter old) => false;
}

/// A card with zigzag top and bottom edges and a hard black shadow.
class ZigCard extends StatelessWidget {
  const ZigCard({
    super.key,
    required this.child,
    this.color = Maxi.cyan,
    this.title,
    this.icon,
    this.trailing,
    this.padding = const EdgeInsets.fromLTRB(18, 26, 18, 26),
  });

  final Widget child;
  final Color color;
  final String? title;
  final Widget? icon;
  final Widget? trailing;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    final theme = Maxi.onLight(Theme.of(context));
    return Padding(
      padding: const EdgeInsets.only(bottom: 18, right: 6),
      child: CustomPaint(
        painter: _ZigPainter(color),
        child: Theme(
          data: theme,
          child: DefaultTextStyle.merge(
            style: const TextStyle(color: Maxi.ink),
            child: Padding(
              padding: padding,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (title != null) ...[
                    Row(children: [
                      if (icon != null) ...[icon!, const SizedBox(width: 10)],
                      Expanded(child: Text(title!, style: Maxi.display(18, Maxi.ink))),
                      ?trailing,
                    ]),
                    const SizedBox(height: 12),
                  ],
                  child,
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ZigPainter extends CustomPainter {
  _ZigPainter(this.color);
  final Color color;

  static const tooth = 18.0;
  static const depth = 9.0;

  Path _path(Size size) {
    final w = size.width, h = size.height;
    final path = Path()..moveTo(0, depth);
    for (var x = 0.0; x < w; x += tooth) {
      final end = math.min(x + tooth, w);
      path.lineTo(math.min(x + tooth / 2, w), 0);
      path.lineTo(end, depth);
    }
    path.lineTo(w, h - depth);
    for (var x = w; x > 0; x -= tooth) {
      final end = math.max(x - tooth, 0.0);
      path.lineTo(math.max(x - tooth / 2, 0.0), h);
      path.lineTo(end, h - depth);
    }
    return path..close();
  }

  @override
  void paint(Canvas canvas, Size size) {
    final path = _path(size);
    canvas.drawPath(path.shift(const Offset(6, 6)), Paint()..color = Maxi.ink);
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_ZigPainter old) => old.color != color;
}

/// A flat box with a black outline and hard shadow, optionally tilted.
class Sticker extends StatelessWidget {
  const Sticker({
    super.key,
    required this.child,
    this.color = Maxi.paper,
    this.tilt = 0,
    this.shadow = 4,
    this.border = 3,
    this.padding = const EdgeInsets.all(12),
  });

  final Widget child;
  final Color color;

  /// Degrees.
  final double tilt;
  final double shadow;
  final double border;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    final box = Container(
      padding: padding,
      decoration: BoxDecoration(
        color: color,
        border: Border.all(color: Maxi.ink, width: border),
        boxShadow: shadow > 0 ? Maxi.hard(Maxi.ink, shadow) : null,
      ),
      child: child,
    );
    return tilt == 0 ? box : Transform.rotate(angle: tilt * math.pi / 180, child: box);
  }
}

/// A small label: `NEW!`, `CREATOR`, `YOUR TURN`.
class Tag extends StatelessWidget {
  const Tag(this.text, {super.key, this.color = Maxi.ink, this.textColor = Maxi.yellow, this.tilt = 0});
  final String text;
  final Color color;
  final Color textColor;
  final double tilt;

  @override
  Widget build(BuildContext context) => Transform.rotate(
        angle: tilt * math.pi / 180,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
          decoration: BoxDecoration(color: color, border: Border.all(color: Maxi.ink, width: 2)),
          child: Text(text.toUpperCase(), style: Maxi.display(10, textColor)),
        ),
      );
}

/// Digits on a black LCD: `002`.
class Lcd extends StatelessWidget {
  const Lcd(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        decoration: BoxDecoration(color: Maxi.ink, border: Border.all(color: Maxi.lime, width: 2)),
        child: Text(text, style: Maxi.pixel(22, Maxi.lime).copyWith(letterSpacing: 3)),
      );
}

/// Adds a hard colored shadow behind a (square) button.
class Pop extends StatelessWidget {
  const Pop({super.key, required this.child, this.color = Maxi.pink});
  final Widget child;
  final Color color;

  @override
  Widget build(BuildContext context) =>
      DecoratedBox(decoration: BoxDecoration(boxShadow: Maxi.hard(color)), child: child);
}

enum Motion { spin, bob, blink, wiggle, nudge, pulse }

/// Keeps [child] moving in one of the [Motion] ways, forever.
class Animated extends StatefulWidget {
  const Animated(this.motion, {super.key, required this.child, this.period});
  final Motion motion;
  final Widget child;
  final Duration? period;

  @override
  State<Animated> createState() => _AnimatedState();
}

class _AnimatedState extends State<Animated> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: widget.period ??
        switch (widget.motion) {
          Motion.spin => const Duration(seconds: 7),
          Motion.bob => const Duration(milliseconds: 1400),
          Motion.blink => const Duration(seconds: 1),
          Motion.wiggle => const Duration(seconds: 1),
          Motion.nudge => const Duration(seconds: 1),
          Motion.pulse => const Duration(milliseconds: 1200),
        },
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _c.stop();
      _c.value = 0;
    } else if (!_c.isAnimating) {
      _c.repeat();
    }
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: _c,
        child: widget.child,
        builder: (context, child) {
          final t = _c.value;
          final wave = math.sin(t * 2 * math.pi);
          return switch (widget.motion) {
            Motion.spin => Transform.rotate(angle: t * 2 * math.pi, child: child),
            Motion.bob => Transform.translate(
                offset: Offset(0, -5 * (1 - math.cos(t * 2 * math.pi)) / 2),
                child: Transform.rotate(angle: -0.14 * (1 - math.cos(t * 2 * math.pi)) / 2, child: child),
              ),
            Motion.blink => Opacity(opacity: t < 0.5 ? 1 : 0, child: child),
            Motion.wiggle => Transform.rotate(angle: 0.17 * wave, child: child),
            Motion.nudge => Transform.translate(offset: Offset(5 * (1 - math.cos(t * 2 * math.pi)) / 2, 0), child: child),
            Motion.pulse => Transform.scale(scale: 1 + 0.15 * (1 - math.cos(t * 2 * math.pi)) / 2, child: child),
          };
        },
      );
}

/// The yellow star with a black outline, spinning.
class SpinningStar extends StatelessWidget {
  const SpinningStar({super.key, this.size = 26});
  final double size;

  @override
  Widget build(BuildContext context) => Animated(
        Motion.spin,
        child: SizedBox(
          width: size,
          height: size,
          child: CustomPaint(painter: _BurstPainter(points: 5, inner: 0.45, fill: Maxi.yellow, ring: null)),
        ),
      );
}

/// A slowly turning starburst, yellow in the middle with pink spikes.
class Starburst extends StatelessWidget {
  const Starburst({super.key, this.size = 84});
  final double size;

  @override
  Widget build(BuildContext context) => Animated(
        Motion.spin,
        period: const Duration(seconds: 18),
        child: SizedBox(
          width: size,
          height: size,
          child: CustomPaint(painter: _BurstPainter(points: 18, inner: 0.82, fill: Maxi.pink, ring: Maxi.yellow)),
        ),
      );
}

class _BurstPainter extends CustomPainter {
  _BurstPainter({required this.points, required this.inner, required this.fill, required this.ring});
  final int points;
  final double inner;
  final Color fill;
  final Color? ring;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide / 2;
    final path = Path();
    for (var i = 0; i < points * 2; i++) {
      final a = -math.pi / 2 + i * math.pi / points;
      final radius = i.isEven ? r : r * inner;
      final p = c + Offset(math.cos(a), math.sin(a)) * radius;
      i == 0 ? path.moveTo(p.dx, p.dy) : path.lineTo(p.dx, p.dy);
    }
    path.close();
    canvas.drawPath(path, Paint()..color = fill);
    if (ring == null) {
      canvas.drawPath(
        path,
        Paint()
          ..color = Maxi.ink
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.6
          ..strokeJoin = StrokeJoin.round,
      );
    } else {
      canvas.drawCircle(c, r * 0.7, Paint()..color = ring!);
    }
  }

  @override
  bool shouldRepaint(_BurstPainter old) => false;
}

/// A hazard-striped strip with [text] scrolling across it.
class Ticker extends StatefulWidget {
  const Ticker(this.text, {super.key});
  final String text;

  @override
  State<Ticker> createState() => _TickerState();
}

class _TickerState extends State<Ticker> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(vsync: this, duration: const Duration(seconds: 16));

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _c.stop();
    } else if (!_c.isAnimating) {
      _c.repeat();
    }
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final style = Maxi.pixel(21, Maxi.ink);
    final run = '★ ${widget.text.toUpperCase()} ';
    final width = (TextPainter(
      text: TextSpan(text: run, style: style),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout())
        .width;
    return ExcludeSemantics(
      child: Column(children: [
        const _Stripes(),
        Container(
          height: 30,
          color: Maxi.yellow,
          alignment: Alignment.centerLeft,
          child: ClipRect(
            child: LayoutBuilder(
              builder: (context, box) => AnimatedBuilder(
                animation: _c,
                builder: (context, _) => OverflowBox(
                  alignment: Alignment.centerLeft,
                  maxWidth: double.infinity,
                  child: Transform.translate(
                    offset: Offset(-_c.value * width, 0),
                    child: Row(mainAxisSize: MainAxisSize.min, children: [
                      for (var i = 0; i < (box.maxWidth / width).ceil() + 1; i++) Text(run, style: style, maxLines: 1),
                    ]),
                  ),
                ),
              ),
            ),
          ),
        ),
        const _Stripes(),
      ]),
    );
  }
}

class _Stripes extends StatelessWidget {
  const _Stripes();

  @override
  Widget build(BuildContext context) =>
      SizedBox(height: 5, width: double.infinity, child: CustomPaint(painter: _StripePainter()));
}

class _StripePainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = Maxi.yellow);
    final black = Paint()..color = Maxi.ink;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    for (var x = -size.height; x < size.width; x += 16) {
      canvas.drawPath(
        Path()
          ..moveTo(x, size.height)
          ..lineTo(x + 8, size.height)
          ..lineTo(x + 8 + size.height, 0)
          ..lineTo(x + size.height, 0)
          ..close(),
        black,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_StripePainter old) => false;
}

/// The 88×31 buttons at the bottom of every page, 1996 style.
class Badges extends StatelessWidget {
  const Badges({super.key});

  @override
  Widget build(BuildContext context) {
    Widget badge(String text, Color bg, Color fg) => Container(
          width: 88,
          height: 31,
          alignment: Alignment.center,
          decoration: BoxDecoration(color: bg, border: Border.all(color: Maxi.paper, width: 2)),
          child: Text(text, textAlign: TextAlign.center, style: Maxi.pixel(13, fg).copyWith(height: 0.85)),
        );
    return ExcludeSemantics(
      child: Padding(
        padding: const EdgeInsets.only(top: 24, bottom: 8),
        child: Wrap(
          alignment: WrapAlignment.center,
          spacing: 8,
          runSpacing: 8,
          children: [
            badge('BEST VIEWED IN ANY BROWSER', Maxi.pink, Maxi.ink),
            badge('100% ENCRYPTED', Maxi.ink, Maxi.lime),
            badge('KEYS NEVER LEAVE', Maxi.cyan, Maxi.ink),
          ],
        ),
      ),
    );
  }
}

/// A round status dot: lime and blinking when online.
class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.online, this.onColor = Maxi.lime});
  final bool online;
  final Color onColor;

  @override
  Widget build(BuildContext context) {
    final dot = Container(
      width: 12,
      height: 12,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: online ? onColor : const Color(0xFF7A6F8F),
        border: Border.all(color: Maxi.ink, width: 2),
      ),
    );
    return online ? Animated(Motion.blink, period: const Duration(milliseconds: 1200), child: dot) : dot;
  }
}
