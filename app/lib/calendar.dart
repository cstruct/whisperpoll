import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/rust/api/poll.dart' as rust;
import 'storage.dart';

/// One picked slot to add as a calendar event.
class SlotEvent {
  SlotEvent({required this.slot, required this.title, required this.notes});

  /// Canonical slot text.
  final String slot;
  final String title;
  final String notes;
}

/// Adds events to the device calendar via the native `CalendarBridge`
/// (ios/Runner), and remembers which polls' slots were added.
class DeviceCalendar {
  static const _channel = MethodChannel('whisperpoll/calendar');

  /// Adds the events; returns how many were added. Throws a
  /// [PlatformException] with code `denied` if calendar access is refused.
  static Future<int> add(Invite invite, List<SlotEvent> events) async {
    final added = await _channel.invokeMethod<int>('addEvents', {
      'events': [
        for (final e in events)
          if (rust.slotTimes(slot: e.slot) case final times?)
            {
              'startMillis': times.startMillis,
              'minutes': times.minutes,
              'title': e.title,
              'notes': e.notes,
            },
      ],
    });
    final prefs = await SharedPreferences.getInstance();
    await prefs.setStringList(_key(invite), [for (final e in events) e.slot]);
    return added ?? 0;
  }

  /// The slots already added from this poll, so the button can say so.
  static Future<List<String>> added(Invite invite) async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getStringList(_key(invite)) ?? [];
  }

  /// Opens the Calendar app at the start of [slot].
  static Future<void> open(String slot) async {
    final times = rust.slotTimes(slot: slot);
    if (times != null) await _channel.invokeMethod('openCalendar', times.startMillis);
  }

  static String _key(Invite invite) => 'calendar:${invite.key}';
}
