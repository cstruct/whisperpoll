import Flutter
import UIKit
import UserNotifications

/// Bridges Apple push notifications to Dart, without Firebase: the device
/// token, background wake-ups (pushes with `content-available`), and taps.
/// Dart talks to it on the `whisperpoll/push` channel.
final class PushBridge: NSObject, FlutterPlugin, UNUserNotificationCenterDelegate {
  private let channel: FlutterMethodChannel
  /// Calls to Dart made before its handler is installed (e.g. a wake-up that
  /// launched the app) wait here until Dart says it's ready.
  private var dartReady = false
  private var pending: [(method: String, args: Any?, done: ((Any?) -> Void)?)] = []

  init(channel: FlutterMethodChannel) {
    self.channel = channel
  }

  static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(
      name: "whisperpoll/push", binaryMessenger: registrar.messenger())
    let instance = PushBridge(channel: channel)
    registrar.addMethodCallDelegate(instance, channel: channel)
    registrar.addApplicationDelegate(instance)
    UNUserNotificationCenter.current().delegate = instance
    // Silent wake-ups need no permission, so always ask for a token.
    DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
  }

  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    switch call.method {
    case "ready":
      dartReady = true
      let queued = pending
      pending.removeAll()
      for call in queued { toDart(call.method, call.args, call.done) }
      result(nil)
    case "requestPermission":
      UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
        granted, _ in
        DispatchQueue.main.async {
          UIApplication.shared.registerForRemoteNotifications()
          result(granted)
        }
      }
    case "clearWake":
      // The phone did its part, so "your phone is needed" is no longer true.
      let poll = call.arguments as? String
      let center = UNUserNotificationCenter.current()
      center.getDeliveredNotifications { delivered in
        let ids = delivered
          .filter { n in
            let info = n.request.content.userInfo
            let aps = info["aps"] as? [String: Any]
            return info["poll"] as? String == poll && aps?["content-available"] as? Int == 1
          }
          .map(\.request.identifier)
        center.removeDeliveredNotifications(withIdentifiers: ids)
      }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func toDart(_ method: String, _ args: Any?, _ done: ((Any?) -> Void)? = nil) {
    guard dartReady else {
      pending.append((method, args, done))
      return
    }
    channel.invokeMethod(method, arguments: args) { done?($0) }
  }

  func application(
    _ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    toDart("token", deviceToken.map { String(format: "%02x", $0) }.joined())
  }

  func application(
    _ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error
  ) {
    #if targetEnvironment(simulator)
      // Without a paid developer account there's no push entitlement and so no
      // real token. Register a placeholder: in development the server delivers
      // to the simulator with `xcrun simctl push` (WHISPERPOLL_PUSH=simulator:…).
      toDart("token", "simulator")
    #endif
  }

  /// A push with `content-available`: iOS gives the app about 30 seconds in
  /// the background, which Dart uses to do this phone's trustee step.
  func application(
    _ application: UIApplication,
    didReceiveRemoteNotification userInfo: [AnyHashable: Any],
    fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
  ) -> Bool {
    guard let poll = userInfo["poll"] as? String else { return false }
    var finished = false
    let finish = { (result: UIBackgroundFetchResult) in
      guard !finished else { return }
      finished = true
      completionHandler(result)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 25) { finish(.failed) }
    toDart("wake", poll) { _ in finish(.newData) }
    return true
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    // While the app is open it does any trustee step itself; only show
    // notifications meant for the person.
    let aps = notification.request.content.userInfo["aps"] as? [String: Any]
    let wake = aps?["content-available"] as? Int == 1
    completionHandler(wake ? [] : [.banner, .list, .sound])
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    if let poll = response.notification.request.content.userInfo["poll"] as? String {
      toDart("open", poll)
    }
    completionHandler()
  }
}
