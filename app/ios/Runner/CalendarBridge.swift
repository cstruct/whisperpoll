import EventKit
import Flutter
import UIKit

/// Adds the picked days straight to the person's calendar with EventKit.
/// Asks only for write access (iOS 17+), so the app can add events but never
/// read anything already in the calendar. Dart talks to it on the
/// `whisperpoll/calendar` channel.
final class CalendarBridge: NSObject, FlutterPlugin {
  private let store = EKEventStore()

  static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(
      name: "whisperpoll/calendar", binaryMessenger: registrar.messenger())
    registrar.addMethodCallDelegate(CalendarBridge(), channel: channel)
  }

  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    switch call.method {
    case "addAllDayEvents":
      guard let args = call.arguments as? [String: Any],
        let events = args["events"] as? [[String: String]]
      else {
        result(FlutterError(code: "bad-args", message: "expected events", details: nil))
        return
      }
      requestAccess { granted in
        DispatchQueue.main.async {
          guard granted else {
            result(FlutterError(code: "denied", message: "Calendar access was not allowed", details: nil))
            return
          }
          do {
            result(try self.add(events))
          } catch {
            result(FlutterError(code: "save-failed", message: error.localizedDescription, details: nil))
          }
        }
      }
    case "openCalendar":
      // Opens the Calendar app at the given ISO date.
      if let iso = call.arguments as? String, let date = Self.day(iso) {
        let url = URL(string: "calshow:\(date.timeIntervalSinceReferenceDate)")!
        UIApplication.shared.open(url)
      }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func requestAccess(_ done: @escaping (Bool) -> Void) {
    if #available(iOS 17.0, *) {
      store.requestWriteOnlyAccessToEvents { granted, _ in done(granted) }
    } else {
      store.requestAccess(to: .event) { granted, _ in done(granted) }
    }
  }

  /// Saves one all-day event per `{date, title, notes}`; returns how many.
  private func add(_ events: [[String: String]]) throws -> Int {
    guard let calendar = store.defaultCalendarForNewEvents else {
      throw NSError(
        domain: "whisperpoll", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "No calendar to add events to"])
    }
    var added = 0
    for spec in events {
      guard let iso = spec["date"], let day = Self.day(iso) else { continue }
      let event = EKEvent(eventStore: store)
      event.calendar = calendar
      event.title = spec["title"]
      event.notes = spec["notes"]
      event.isAllDay = true
      event.startDate = day
      event.endDate = day
      try store.save(event, span: .thisEvent, commit: false)
      added += 1
    }
    try store.commit()
    return added
  }

  /// Local midnight of an ISO `YYYY-MM-DD` date.
  private static func day(_ iso: String) -> Date? {
    let formatter = DateFormatter()
    formatter.calendar = Calendar(identifier: .gregorian)
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.dateFormat = "yyyy-MM-dd"
    return formatter.date(from: iso)
  }
}
