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

  private static let permissionDenied = TokamakPluginError(
    name: "NotAllowedError",
    message: "Location permission was denied"
  )
  private static let timedOut = TokamakPluginError(
    name: "TimeoutError",
    message: "Location was not found in time"
  )

  /// A current position request, timed from when locating starts.
  private struct Request {
    let reply: TokamakPluginReply
    let maximumAge: TimeInterval
    let timeout: TimeInterval
    var timer: DispatchWorkItem?
  }

  /// A `requestPermission` waiting for the user to answer a prompt.
  private struct Prompt {
    /// The purpose key of the temporary precise location to ask for next.
    let purpose: String?
    let reply: TokamakPluginReply
  }

  private let host: TokamakHost
  private let defaults = UserDefaults.standard
  /// Locates for watchers, and asks for permission.
  private let watching = CLLocationManager()
  /// Takes single fixes for current position requests, apart from watching.
  private let fixes = CLLocationManager()
  /// Delivers the significant changes and visits `setup` names.
  private let monitoring = CLLocationManager()
  private var current: [UUID: Request] = [:]
  private var watchers: [UUID: TokamakPluginReply] = [:]
  private var prompts: [UUID: Prompt] = [:]
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
    for manager in [watching, fixes] {
      manager.delegate = self
      manager.desiredAccuracy = kCLLocationAccuracyBest
    }
    monitoring.delegate = self
    if let stored = defaults.data(forKey: setupKey),
      let config = try? JSONSerialization.jsonObject(with: stored) as? [String: Any]
    {
      apply(config)
    }
    #if os(iOS)
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
  ) {
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
      case "getCurrentPosition":
        current[UUID()] = Request(
          reply: reply,
          maximumAge: (options["maximumAge"] as? Double ?? 0) / 1000,
          timeout: (options["timeout"] as? Double ?? Double(UInt32.max)) / 1000
        )
        start()
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
        guard watching.accuracyAuthorization == .fullAccuracy else {
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
  }

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    if let remove = events.listen(method, reply) {
      return remove
    }
    guard method == "watchPosition" else {
      throw .notSupported("\(id).\(method) is not supported")
    }
    let subscription = UUID()
    watchers[subscription] = reply
    start()
    return { [weak self] in
      self?.watchers.removeValue(forKey: subscription)
      if self?.watchers.isEmpty == true {
        self?.watching.stopUpdatingLocation()
      }
    }
  }

  /// Also called when each manager is created, before anything asks for a
  /// position. Every manager reports the app's one authorization, which
  /// `watching`'s calls handle.
  func locationManagerDidChangeAuthorization(
    _ manager: CLLocationManager
  ) {
    guard manager === watching else { return }
    holdAlwaysSession()
    guard manager.authorizationStatus != .notDetermined else { return }
    answer(Array(prompts.keys))
    if !current.isEmpty || !watchers.isEmpty {
      serve(manager.authorizationStatus)
    }
  }

  func locationManager(
    _ manager: CLLocationManager,
    didUpdateLocations locations: [CLLocation]
  ) {
    if manager === monitoring {
      for location in locations {
        events.update(location)
      }
      return
    }
    guard let location = locations.last else { return }
    deliver(.success(TokamakLocationEvents.position(location)), from: manager)
  }

  /// A fix that is not yet available is taken again; updates continue.
  func locationManager(
    _ manager: CLLocationManager,
    didFailWithError error: Error
  ) {
    if (error as NSError).code == CLError.Code.locationUnknown.rawValue {
      if manager === fixes && !current.isEmpty {
        fixes.requestLocation()
      }
    } else if manager === monitoring {
      events.fail(locationError(error))
    } else {
      deliver(.failure(locationError(error)), from: manager)
    }
  }

  func locationManager(_ manager: CLLocationManager, didVisit visit: CLVisit) {
    events.visit(visit)
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

  /// Checks location services off the main thread, which the check can block.
  private func start() {
    DispatchQueue.global(qos: .userInitiated).async {
      let enabled = CLLocationManager.locationServicesEnabled()
      DispatchQueue.main.async { self.start(servicesEnabled: enabled) }
    }
  }

  private func start(servicesEnabled: Bool) {
    guard servicesEnabled else {
      fail(.locationUnavailable("Location services are disabled"))
      return
    }
    let status = watching.authorizationStatus
    guard status == .notDetermined else {
      serve(status)
      return
    }
    do throws(TokamakPluginError) {
      try host.requireUI()
      watching.requestWhenInUseAuthorization()
    } catch {
      fail(error)
    }
  }

  /// Locates for the waiting requests and watchers when `status` allows it,
  /// and fails them when it forbids it.
  private func serve(_ status: CLAuthorizationStatus) {
    switch status {
    case .authorizedAlways, .authorizedWhenInUse:
      if !watchers.isEmpty {
        watching.startUpdatingLocation()
      }
      locate()
    case .denied, .restricted:
      fail(Self.permissionDenied)
    case .notDetermined:
      break
    @unknown default:
      fail(.notSupported("Location authorization is not supported"))
    }
  }

  /// Answers each new request with a fix no older than its `maximumAge`, and
  /// takes a fix for the rest, each failing after its `timeout`.
  private func locate() {
    for (key, request) in current where request.timer == nil {
      if let fix = fixes.location, -fix.timestamp.timeIntervalSinceNow <= request.maximumAge {
        finish(key, .success(TokamakLocationEvents.position(fix)))
        continue
      }
      let timer = DispatchWorkItem { [weak self] in self?.finish(key, .failure(Self.timedOut)) }
      current[key]?.timer = timer
      DispatchQueue.main.asyncAfter(deadline: .now() + request.timeout, execute: timer)
    }
    if !current.isEmpty {
      fixes.requestLocation()
    }
  }

  /// Answers the waiting requests with a fix, or the watchers with an update.
  private func deliver(_ result: Result<Any?, TokamakPluginError>, from manager: CLLocationManager)
  {
    guard manager === fixes else {
      for reply in watchers.values {
        reply(result)
      }
      return
    }
    for key in current.keys {
      finish(key, result)
    }
  }

  private func fail(_ error: TokamakPluginError) {
    deliver(.failure(error), from: fixes)
    deliver(.failure(error), from: watching)
  }

  private func finish(_ key: UUID, _ result: Result<Any?, TokamakPluginError>) {
    guard let request = current.removeValue(forKey: key) else { return }
    request.timer?.cancel()
    request.reply(result)
    if current.isEmpty {
      fixes.stopUpdatingLocation()
    }
  }

  /// The app's in-use and background permission, and the precision it gets.
  private func permission() -> [String: Any] {
    let status = watching.authorizationStatus
    return [
      "inUse": Self.permission(status),
      "background": backgroundPermission(status),
      "precision": watching.accuracyAuthorization == .reducedAccuracy ? "approximate" : "precise",
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
    Self.permission(watching.authorizationStatus) == "granted"
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
      let always = watching.authorizationStatus == .authorizedAlways
      if always && alwaysSession == nil {
        alwaysSession = CLServiceSession(authorization: .always)
      } else if !always {
        (alwaysSession as? CLServiceSession)?.invalidate()
        alwaysSession = nil
      }
    #endif
  }

  private static func permission(_ status: CLAuthorizationStatus) -> String {
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
    let status = watching.authorizationStatus
    let upgrades = always && canUpgradeToAlways(status)
    guard status == .notDetermined || upgrades else {
      respond(to: prompt)
      return
    }
    try host.requireUI()
    prompts[UUID()] = prompt
    if always {
      defaults.set(true, forKey: askedForAlwaysKey)
      watching.requestAlwaysAuthorization()
    } else {
      watching.requestWhenInUseAuthorization()
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
    /// Answers the prompts showing as the app became active, as the system
    /// reports no change when the user keeps When In Use.
    @objc private func didBecomeActive() {
      let showing = Array(prompts.keys)
      guard !showing.isEmpty else { return }
      DispatchQueue.main.asyncAfter(deadline: .now() + authorizationChangeDelay) {
        self.answer(showing)
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
      watching.accuracyAuthorization == .reducedAccuracy
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
    watching.requestTemporaryFullAccuracyAuthorization(withPurposeKey: purpose) { _ in
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

  private func locationError(_ error: Error) -> TokamakPluginError {
    if (error as NSError).code == CLError.Code.denied.rawValue {
      return Self.permissionDenied
    }
    return .locationUnavailable(error.localizedDescription)
  }
}
