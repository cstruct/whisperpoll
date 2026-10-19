import EventKit
import Flutter
import UIKit

/// Adds the picked times straight to the person's calendar with EventKit.
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
    case "addEvents":
      guard let args = call.arguments as? [String: Any],
        let events = args["events"] as? [[String: Any]]
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
      // Opens the Calendar app at the given moment (milliseconds since the epoch).
      if let millis = call.arguments as? NSNumber {
        let date = Date(timeIntervalSince1970: millis.doubleValue / 1000)
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

  /// Saves one event per `{startMillis, minutes, title, notes}`; returns how many.
  private func add(_ events: [[String: Any]]) throws -> Int {
    guard let calendar = store.defaultCalendarForNewEvents else {
      throw NSError(
        domain: "whisperpoll", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "No calendar to add events to"])
    }
    var added = 0
    for spec in events {
      guard let millis = spec["startMillis"] as? NSNumber,
        let minutes = spec["minutes"] as? NSNumber
      else { continue }
      let start = Date(timeIntervalSince1970: millis.doubleValue / 1000)
      let event = EKEvent(eventStore: store)
      event.calendar = calendar
      event.title = spec["title"] as? String
      event.notes = spec["notes"] as? String
      event.startDate = start
      event.endDate = start.addingTimeInterval(minutes.doubleValue * 60)
      try store.save(event, span: .thisEvent, commit: false)
      added += 1
    }
    try store.commit()
    return added
  }
}
