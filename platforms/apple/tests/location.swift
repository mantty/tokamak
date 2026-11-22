import CoreLocation
import Foundation
import ObjectiveC
import TokamakPlugin
import WebKit

@testable import TokamakLocation

/// What the plugin asked Core Location for, recorded in place of Core
/// Location acting on it.
private var requests: [String] = []
/// Whether the device monitors significant changes, as Core Location reports.
private var monitorsSignificantChanges = true

extension CLLocationManager {
  @objc fileprivate func recordStartSignificantChanges() {
    requests.append("start significant changes")
  }
  @objc fileprivate func recordStopSignificantChanges() {
    requests.append("stop significant changes")
  }
  @objc fileprivate func recordStartVisits() { requests.append("start visits") }
  @objc fileprivate func recordStopVisits() { requests.append("stop visits") }
  @objc fileprivate class func recordedAvailability() -> Bool { monitorsSignificantChanges }
}

@main
enum LocationTests {
  static func main() {
    for (request, recording) in [
      (
        #selector(CLLocationManager.startMonitoringSignificantLocationChanges),
        #selector(CLLocationManager.recordStartSignificantChanges)
      ),
      (
        #selector(CLLocationManager.stopMonitoringSignificantLocationChanges),
        #selector(CLLocationManager.recordStopSignificantChanges)
      ),
      (
        #selector(CLLocationManager.startMonitoringVisits),
        #selector(CLLocationManager.recordStartVisits)
      ),
      (
        #selector(CLLocationManager.stopMonitoringVisits),
        #selector(CLLocationManager.recordStopVisits)
      ),
    ] {
      method_exchangeImplementations(
        class_getInstanceMethod(CLLocationManager.self, request)!,
        class_getInstanceMethod(CLLocationManager.self, recording)!)
    }
    method_exchangeImplementations(
      class_getClassMethod(
        CLLocationManager.self,
        #selector(CLLocationManager.significantLocationChangeMonitoringAvailable))!,
      class_getClassMethod(
        CLLocationManager.self, #selector(CLLocationManager.recordedAvailability))!)

    let app = App()
    let plugin = TokamakLocationPlugin(host: TokamakHost(app: app, plugin: "location"))
    precondition(requests.isEmpty, "the plugin starts significant changes and visits at launch")

    try! plugin.startSignificantChanges()
    plugin.startVisits()
    plugin.stopSignificantChanges()
    plugin.stopVisits()
    precondition(
      requests == [
        "start significant changes", "start visits", "stop significant changes", "stop visits",
      ])

    monitorsSignificantChanges = false
    do {
      try plugin.startSignificantChanges()
      preconditionFailure("significant changes started on a device that cannot monitor them")
    } catch {
      precondition(error.name == "NotSupportedError")
    }
    precondition(requests.count == 4)
    print("Passed location significant change and visit tests")
  }
}

// The methods under test use none of the app's facilities.
private final class App: TokamakHostApp {
  weak var webView: WKWebView?
  let resources = TokamakResources()
  var plugins: [String: any TokamakPlugin] = [:]
  func emit(
    _ name: String, payload: Data, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) { preconditionFailure("Unexpected event") }
  func fetch(
    _ path: String, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) { preconditionFailure("Unexpected fetch") }
}
