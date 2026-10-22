import CoreLocation
import Foundation

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

/// The JSON of the configuration `setup` last applied.
private let setupKey = "tokamak.location.setup"
/// Whether the plugin asked for Always permission, which iOS asks once.
private let askedForAlwaysKey = "tokamak.location.askedForAlways"
/// How long an authorization change may take to arrive after the app is active again.
private let authorizationChangeDelay: TimeInterval = 1

final class TokamakLocationPlugin: NSObject, TokamakPlugin,
  CLLocationManagerDelegate
{
  let id = "location"

  /// A `requestPermission` waiting for the user to answer a prompt.
  private struct Prompt {
    /// The purpose key of the temporary precise location to ask for next.
    let purpose: String?
    let reply: TokamakPluginReply
  }

  private let host: TokamakHost
  private let defaults = UserDefaults.standard
  /// Reports the app's authorization, and asks for permission.
  private let authorization = CLLocationManager()
  /// Delivers the significant changes and visits `setup` names.
  private let monitoring = CLLocationManager()
  private var prompts: [UUID: Prompt] = [:]
  /// The prompts waiting when the app resigned active, as a prompt showing
  /// makes it.
  private var shown: [UUID] = []
  private let events: TokamakLocationEvents
  private let updates: TokamakLocationUpdates
  private let regions: TokamakLocationRegions
  #if os(iOS)
    /// The `CLServiceSession` iOS 18 needs for updates and regions to reach an
    /// app that is not in use, held while the app has Always permission.
    private var alwaysSession: AnyObject?
  #endif

  /// Resumes what ran when the app last ended, before launch finishes.
  init(host: TokamakHost) {
    self.host = host
    events = TokamakLocationEvents(host: host)
    updates = TokamakLocationUpdates(events: events)
    regions = TokamakLocationRegions(events: events)
    super.init()
    authorization.delegate = self
    monitoring.delegate = self
    if let stored = defaults.data(forKey: setupKey),
      let config = try? JSONSerialization.jsonObject(with: stored) as? [String: Any]
    {
      apply(config)
    }
    #if os(iOS)
      NotificationCenter.default.addObserver(
        self, selector: #selector(willResignActive),
        name: UIApplication.willResignActiveNotification, object: nil)
      NotificationCenter.default.addObserver(
        self, selector: #selector(didBecomeActive),
        name: UIApplication.didBecomeActiveNotification, object: nil)
    #endif
    updates.resume()
  }

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)? {
    let options = arguments as? [String: Any] ?? [:]
    do throws(TokamakPluginError) {
      switch method {
      case "setup":
        if options["significantChanges"] as? Bool == true
          && !CLLocationManager.significantLocationChangeMonitoringAvailable()
        {
          throw .notSupported("The device cannot monitor significant changes of location")
        }
        apply(options)
        defaults.set(try? JSONSerialization.data(withJSONObject: options), forKey: setupKey)
        reply(.success(nil))
      case "recentPosition":
        reply(.success(recentPosition(maximumAge: (options["maximumAge"] as? Double ?? 0) / 1000)))
      case "currentPosition":
        let fix = TokamakLocationPositions(reply: reply)
        fix.locate()
        // The cancellation owns the location manager until the call ends.
        return fix.stop
      case "permission":
        reply(.success(permission()))
      case "requestPermission":
        try requestPermission(options, reply)
      case "servicesEnabled":
        DispatchQueue.global(qos: .userInitiated).async {
          reply(.success(CLLocationManager.locationServicesEnabled()))
        }
      case "openSettings":
        try host.requireUI()
        openSettings(reply)
      case "startUpdates":
        // A new background activity session starts only on screen.
        try host.requireUI()
        try requirePermission()
        try updates.start(options)
        reply(.success(nil))
      case "stopUpdates":
        updates.stop()
        reply(.success(nil))
      case "getUpdates":
        reply(.success(updates.options))
      case "addRegion":
        try requirePermission()
        guard authorization.accuracyAuthorization == .fullAccuracy else {
          throw TokamakPluginError(
            name: "NotAllowedError", message: "Regions need precise location")
        }
        Task { @MainActor in
          do throws(TokamakPluginError) {
            try await regions.add(options)
            reply(.success(nil))
          } catch {
            reply(.failure(error))
          }
        }
      case "getRegions":
        Task { @MainActor in reply(.success(await regions.all())) }
      case "removeRegion":
        guard let id = options["id"] as? String else {
          throw TokamakPluginError(name: "TypeError", message: "id must be a string")
        }
        Task { @MainActor in
          await regions.remove(id)
          reply(.success(nil))
        }
      default:
        throw .notSupported("\(id).\(method) is not supported")
      }
    } catch {
      reply(.failure(error))
    }
    return nil
  }

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    if method == "watchPosition" {
      let watch = TokamakLocationPositions(reply: reply)
      watch.watch()
      // The release owns the location manager until the subscription ends.
      return watch.stop
    }
    guard let remove = events.listen(method, reply) else {
      throw .notSupported("\(id).\(method) is not supported")
    }
    return remove
  }

  /// Also called when each manager is created. Every manager reports the
  /// app's one authorization, which `authorization`'s calls handle.
  func locationManagerDidChangeAuthorization(
    _ manager: CLLocationManager
  ) {
    guard manager === authorization else { return }
    holdAlwaysSession()
    guard manager.authorizationStatus != .notDetermined else { return }
    answer(Array(prompts.keys))
  }

  func locationManager(
    _ manager: CLLocationManager,
    didUpdateLocations locations: [CLLocation]
  ) {
    guard manager === monitoring else { return }
    for location in locations {
      events.update(location)
    }
  }

  /// Monitoring continues while the position is not yet known.
  func locationManager(
    _ manager: CLLocationManager,
    didFailWithError error: Error
  ) {
    guard manager === monitoring, (error as NSError).code != CLError.Code.locationUnknown.rawValue
    else { return }
    events.fail(.location(error))
  }

  /// Core Location delivers each visit to every manager's delegate.
  func locationManager(_ manager: CLLocationManager, didVisit visit: CLVisit) {
    if manager === monitoring {
      events.visit(visit)
    }
  }

  /// Core Location's latest position when it is no older than `maximumAge`
  /// seconds, or nil.
  private func recentPosition(maximumAge: TimeInterval) -> Any? {
    guard let recent = authorization.location, -recent.timestamp.timeIntervalSinceNow <= maximumAge
    else { return nil }
    return TokamakLocationEvents.position(recent)
  }

  /// Starts the significant changes and visits `config` names, and stops the
  /// others. After a launch for one of their events, starting them again
  /// delivers it.
  private func apply(_ config: [String: Any]) {
    if config["significantChanges"] as? Bool == true {
      monitoring.startMonitoringSignificantLocationChanges()
    } else {
      monitoring.stopMonitoringSignificantLocationChanges()
    }
    if config["visits"] as? Bool == true {
      monitoring.startMonitoringVisits()
    } else {
      monitoring.stopMonitoringVisits()
    }
  }

  /// The app's in-use and background permission, and the precision it gets.
  private func permission() -> [String: Any] {
    let status = authorization.authorizationStatus
    return [
      "inUse": Self.state(status),
      "background": backgroundPermission(status),
      "precision": authorization.accuracyAuthorization == .reducedAccuracy
        ? "approximate" : "precise",
    ]
  }

  private func backgroundPermission(_ status: CLAuthorizationStatus) -> String {
    if status == .authorizedAlways { return "granted" }
    let asks = status == .notDetermined ? Self.declaresAlways : canUpgradeToAlways(status)
    return asks ? "prompt" : "denied"
  }

  /// Whether asking for Always shows a prompt: once, from When In Use, on iOS.
  private func canUpgradeToAlways(_ status: CLAuthorizationStatus) -> Bool {
    #if os(iOS)
      status == .authorizedWhenInUse && Self.declaresAlways && !askedForAlways
    #else
      false
    #endif
  }

  private var hasPermission: Bool {
    ![.notDetermined, .denied, .restricted].contains(authorization.authorizationStatus)
  }

  /// Throws `NotAllowedError` unless the app has location permission.
  private func requirePermission() throws(TokamakPluginError) {
    guard hasPermission else {
      throw TokamakPluginError(
        name: "NotAllowedError", message: "Location permission has not been granted")
    }
  }

  private func holdAlwaysSession() {
    #if os(iOS)
      guard #available(iOS 18, *) else { return }
      let always = authorization.authorizationStatus == .authorizedAlways
      if always && alwaysSession == nil {
        alwaysSession = CLServiceSession(authorization: .always)
      } else if !always {
        (alwaysSession as? CLServiceSession)?.invalidate()
        alwaysSession = nil
      }
    #endif
  }

  private static func state(_ status: CLAuthorizationStatus) -> String {
    switch status {
    case .notDetermined: "prompt"
    case .denied, .restricted: "denied"
    default: "granted"
    }
  }

  /// Whether the Info.plist declares Always permission, as the `background`
  /// build setting does on iOS and every macOS build does.
  private static let declaresAlways =
    Bundle.main.object(forInfoDictionaryKey: "NSLocationAlwaysAndWhenInUseUsageDescription") != nil

  private var askedForAlways: Bool {
    defaults.bool(forKey: askedForAlwaysKey)
  }

  /// Asks for in-use permission while the user has not answered, or for
  /// Always permission once from When In Use, then for temporary precise
  /// location when `options` name a purpose.
  private func requestPermission(
    _ options: [String: Any], _ reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) {
    let always = options["background"] as? Bool == true
    if always && !Self.declaresAlways {
      throw .notSupported(
        "Background location needs the location plugin's background build setting")
    }
    let prompt = Prompt(purpose: try Self.purpose(options), reply: reply)
    let status = authorization.authorizationStatus
    let upgrades = always && canUpgradeToAlways(status)
    guard status == .notDetermined || upgrades else {
      respond(to: prompt)
      return
    }
    try host.requireUI()
    prompts[UUID()] = prompt
    if always {
      defaults.set(true, forKey: askedForAlwaysKey)
      authorization.requestAlwaysAuthorization()
    } else {
      authorization.requestWhenInUseAuthorization()
    }
  }

  /// The purpose key of the temporary precise location `options` ask for.
  private static func purpose(_ options: [String: Any]) throws(TokamakPluginError) -> String? {
    guard options["precision"] as? String == "precise" else { return nil }
    let purposes =
      Bundle.main.object(forInfoDictionaryKey: "NSLocationTemporaryUsageDescriptionDictionary")
      as? [String: Any]
    guard let purpose = options["purpose"] as? String, purposes?[purpose] != nil else {
      throw TokamakPluginError(
        name: "TypeError",
        message: "precise location needs a purpose from the accuracy-purposes build setting")
    }
    return purpose
  }

  #if os(iOS)
    @objc private func willResignActive() {
      shown = Array(prompts.keys)
    }

    /// Answers the prompts that were showing, as the system reports no change
    /// when the user keeps When In Use.
    @objc private func didBecomeActive() {
      let answered = shown
      shown = []
      guard !answered.isEmpty else { return }
      DispatchQueue.main.asyncAfter(deadline: .now() + authorizationChangeDelay) {
        self.answer(answered)
      }
    }
  #endif

  /// Answers the prompts `keys` that still wait.
  private func answer(_ keys: [UUID]) {
    for key in keys {
      if let prompt = prompts.removeValue(forKey: key) {
        respond(to: prompt)
      }
    }
  }

  /// Replies to `prompt` with the app's permission, first asking for temporary
  /// precise location for its purpose when the app has approximate location.
  private func respond(to prompt: Prompt) {
    guard let purpose = prompt.purpose, hasPermission,
      authorization.accuracyAuthorization == .reducedAccuracy
    else {
      prompt.reply(.success(permission()))
      return
    }
    do throws(TokamakPluginError) {
      try host.requireUI()
    } catch {
      prompt.reply(.failure(error))
      return
    }
    authorization.requestTemporaryFullAccuracyAuthorization(withPurposeKey: purpose) { _ in
      DispatchQueue.main.async { prompt.reply(.success(self.permission())) }
    }
  }

  /// Opens the app's page of the system settings on iOS, and Location Services on macOS.
  private func openSettings(_ reply: @escaping TokamakPluginReply) {
    let notOpened = TokamakPluginError(
      name: "OperationError", message: "The location settings could not be opened")
    #if os(iOS)
      UIApplication.shared.open(URL(string: UIApplication.openSettingsURLString)!) { opened in
        reply(opened ? .success(nil) : .failure(notOpened))
      }
    #else
      let page = "com.apple.preference.security?Privacy_LocationServices"
      let opened = URL(string: "x-apple.systempreferences:\(page)").map(NSWorkspace.shared.open)
      reply(opened == true ? .success(nil) : .failure(notOpened))
    #endif
  }
}
