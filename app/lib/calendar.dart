import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'storage.dart';

/// One all-day event to add.
class DayEvent {
  DayEvent({required this.date, required this.title, required this.notes});

  /// ISO `YYYY-MM-DD`.
  final String date;
  final String title;
  final String notes;
}

/// Adds events to the device calendar via the native `CalendarBridge`
/// (ios/Runner), and remembers which polls' days were added.
class DeviceCalendar {
  static const _channel = MethodChannel('whisperpoll/calendar');

  /// Adds all-day events; returns how many were added. Throws a
  /// [PlatformException] with code `denied` if calendar access is refused.
  static Future<int> add(Invite invite, List<DayEvent> events) async {
    final added = await _channel.invokeMethod<int>('addAllDayEvents', {
      'events': [
        for (final e in events) {'date': e.date, 'title': e.title, 'notes': e.notes}
      ],
    });
    final prefs = await SharedPreferences.getInstance();
    await prefs.setStringList(_key(invite), [for (final e in events) e.date]);
    return added ?? 0;
  }

  /// The dates already added from this poll, so the button can say so.
  static Future<List<String>> added(Invite invite) async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getStringList(_key(invite)) ?? [];
  }

  /// Opens the Calendar app at [date].
  static Future<void> open(String date) => _channel.invokeMethod('openCalendar', date);

  static String _key(Invite invite) => 'calendar:${invite.key}';
}
