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
  let host: TokamakHost

  /// A `requestPermission` waiting for the user to answer a prompt.
  private struct Prompt {
    /// The purpose key of the temporary precise location to ask for next.
    let purpose: String?
    let answer: CheckedContinuation<LocationPermission, Error>
  }

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
      let config = try? JSONDecoder().decode(SetupConfig.self, from: stored)
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

  /// Applies long-term location awareness, which the plugin applies again at each start.
  func setup(_ config: SetupConfig) throws(TokamakPluginError) {
    if config.significantChanges == true
      && !CLLocationManager.significantLocationChangeMonitoringAvailable()
    {
      throw .notSupported("The device cannot monitor significant changes of location")
    }
    apply(config)
    defaults.set(try? JSONEncoder().encode(config), forKey: setupKey)
  }

  /// Core Location's latest position when it is no older than `maximumAge`
  /// milliseconds, or nil.
  func recentPosition(maximumAge: Double) -> Position? {
    guard let recent = authorization.location,
      -recent.timestamp.timeIntervalSinceNow <= maximumAge / 1000
    else { return nil }
    return Position(recent)
  }

  /// A new fix, from a location manager that the call owns until it ends.
  @MainActor
  func currentPosition() async throws -> Position {
    for try await position in TokamakLocationPositions.stream(watching: false) {
      return position
    }
    throw CancellationError()
  }

  func watchPosition() -> AsyncThrowingStream<Position, Error> {
    TokamakLocationPositions.stream(watching: true)
  }

  func permission() -> LocationPermission {
    let status = authorization.authorizationStatus
    return LocationPermission(
      inUse: Self.state(status),
      background: backgroundPermission(status),
      precision: authorization.accuracyAuthorization == .reducedAccuracy ? .approximate : .precise
    )
  }

  /// Asks for in-use permission while the user has not answered, or for
  /// Always permission once from When In Use, then for temporary precise
  /// location when `request` names a purpose.
  @MainActor
  func requestPermission(_ request: PermissionRequest?) async throws -> LocationPermission {
    let always = request?.background == true
    if always && !Self.declaresAlways {
      throw TokamakPluginError.notSupported(
        "Background location needs the location plugin's background build setting")
    }
    let purpose = try Self.purpose(request)
    let status = authorization.authorizationStatus
    guard status == .notDetermined || always && canUpgradeToAlways(status) else {
      return try await respond(purpose: purpose)
    }
    try host.requireUI()
    return try await withCheckedThrowingContinuation { answer in
      prompts[UUID()] = Prompt(purpose: purpose, answer: answer)
      if always {
        defaults.set(true, forKey: askedForAlwaysKey)
        authorization.requestAlwaysAuthorization()
      } else {
        authorization.requestWhenInUseAuthorization()
      }
    }
  }

  /// Whether location services are on, which Core Location reports off the
  /// main thread.
  func servicesEnabled() async -> Bool {
    CLLocationManager.locationServicesEnabled()
  }

  /// Opens the app's page of the system settings on iOS, and Location Services on macOS.
  @MainActor
  func openSettings() async throws {
    try host.requireUI()
    #if os(iOS)
      let opened = await UIApplication.shared.open(
        URL(string: UIApplication.openSettingsURLString)!)
    #else
      let page = "com.apple.preference.security?Privacy_LocationServices"
      let opened =
        URL(string: "x-apple.systempreferences:\(page)").map(NSWorkspace.shared.open) == true
    #endif
    guard opened else {
      throw TokamakPluginError(
        name: "OperationError", message: "The location settings could not be opened")
    }
  }

  /// Starts continuous updates, which a new background activity session
  /// allows only on screen.
  func startUpdates(_ options: UpdateOptions?) throws(TokamakPluginError) {
    try host.requireUI()
    try requirePermission()
    try updates.start(options ?? UpdateOptions(activity: nil))
  }

  func stopUpdates() {
    updates.stop()
  }

  func getUpdates() -> UpdateOptions? {
    updates.options
  }

  @MainActor
  func addRegion(_ region: Region) async throws {
    try requirePermission()
    guard authorization.accuracyAuthorization == .fullAccuracy else {
      throw TokamakPluginError(name: "NotAllowedError", message: "Regions need precise location")
    }
    try await regions.add(region)
  }

  @MainActor
  func getRegions() async -> [MonitoredRegion] {
    await regions.all()
  }

  @MainActor
  func removeRegion(_ id: String) async {
    await regions.remove(id)
  }

  func onLocationUpdate() -> AsyncStream<LocationUpdate> {
    events.updates.stream()
  }

  func onRegionTransition() -> AsyncStream<RegionTransition> {
    events.transitions.stream()
  }

  func onVisit() -> AsyncStream<Visit> {
    events.visits.stream()
  }

  func onLocationError() -> AsyncStream<LocationErrorEvent> {
    events.errors.stream()
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

  /// Starts the significant changes and visits `config` names, and stops the
  /// others. After a launch for one of their events, starting them again
  /// delivers it.
  private func apply(_ config: SetupConfig) {
    if config.significantChanges == true {
      monitoring.startMonitoringSignificantLocationChanges()
    } else {
      monitoring.stopMonitoringSignificantLocationChanges()
    }
    if config.visits == true {
      monitoring.startMonitoringVisits()
    } else {
      monitoring.stopMonitoringVisits()
    }
  }

  private func backgroundPermission(_ status: CLAuthorizationStatus) -> Permission {
    if status == .authorizedAlways { return .granted }
    let asks = status == .notDetermined ? Self.declaresAlways : canUpgradeToAlways(status)
    return asks ? .prompt : .denied
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

  private static func state(_ status: CLAuthorizationStatus) -> Permission {
    switch status {
    case .notDetermined: .prompt
    case .denied, .restricted: .denied
    default: .granted
    }
  }

  /// Whether the Info.plist declares Always permission, as the `background`
  /// build setting does on iOS and every macOS build does.
  private static let declaresAlways =
    Bundle.main.object(forInfoDictionaryKey: "NSLocationAlwaysAndWhenInUseUsageDescription") != nil

  private var askedForAlways: Bool {
    defaults.bool(forKey: askedForAlwaysKey)
  }

  /// The purpose key of the temporary precise location `request` asks for.
  private static func purpose(_ request: PermissionRequest?) throws(TokamakPluginError) -> String? {
    guard request?.precision == .precise else { return nil }
    let purposes =
      Bundle.main.object(forInfoDictionaryKey: "NSLocationTemporaryUsageDescriptionDictionary")
      as? [String: Any]
    guard let purpose = request?.purpose, purposes?[purpose] != nil else {
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
      guard let prompt = prompts.removeValue(forKey: key) else { continue }
      Task { @MainActor in await answer(prompt) }
    }
  }

  @MainActor
  private func answer(_ prompt: Prompt) async {
    do {
      prompt.answer.resume(returning: try await respond(purpose: prompt.purpose))
    } catch {
      prompt.answer.resume(throwing: error)
    }
  }

  /// The app's permission, after asking for temporary precise location for
  /// `purpose` when the app has approximate location.
  @MainActor
  private func respond(purpose: String?) async throws -> LocationPermission {
    if let purpose, hasPermission, authorization.accuracyAuthorization == .reducedAccuracy {
      try host.requireUI()
      _ = try? await authorization.requestTemporaryFullAccuracyAuthorization(
        withPurposeKey: purpose)
    }
    return permission()
  }
}
