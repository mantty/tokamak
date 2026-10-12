import Foundation
import ImageIO
import UniformTypeIdentifiers
import UserNotifications

/// Loads a notification's attachments from `https` URLs or paths the app serves.
struct TokamakNotificationAttachments {
  let host: TokamakHost

  /// Loads the attachments of a notification's `image` and `attachments` in parallel, then calls
  /// `completion` on the main thread with them in order, leaving out those that do not load within
  /// `timeout` or that Apple rejects. Call it on the main thread.
  func load(
    _ fields: [String: Any], timeout: TimeInterval,
    completion: @escaping ([UNNotificationAttachment]) -> Void
  ) {
    var specs = fields["attachments"] as? [[String: Any]] ?? []
    if let image = fields["image"] as? String {
      specs.insert(["url": image], at: 0)
    }
    var loaded = [UNNotificationAttachment?](repeating: nil, count: specs.count)
    let group = DispatchGroup()
    for (index, spec) in specs.enumerated() {
      group.enter()
      attachment(spec, timeout: timeout) { attachment in
        DispatchQueue.main.async {
          loaded[index] = attachment
          group.leave()
        }
      }
    }
    group.notify(queue: .main) { completion(loaded.compactMap { $0 }) }
  }

  /// Calls `completion` off the main thread with the attachment `spec` describes, or nil when it
  /// does not load within `timeout` or Apple rejects it.
  private func attachment(
    _ spec: [String: Any], timeout: TimeInterval,
    completion: @escaping (UNNotificationAttachment?) -> Void
  ) {
    let source = spec["url"] as? String ?? ""
    data(source, timeout: timeout) { result in
      do {
        let (data, fileExtension) = try result.get()
        completion(try Self.attachment(data, fileExtension: fileExtension, spec: spec))
      } catch {
        print("tokamak notification attachment \(source) failed: \(error)")
        completion(nil)
      }
    }
  }

  /// An attachment of `data` in a temporary file, which the system moves into its store when it
  /// adds the notification. Apple reads the type from the file's extension: `fileExtension`, or
  /// for an image that names no type, its own.
  private static func attachment(
    _ data: Data, fileExtension: String?, spec: [String: Any]
  ) throws -> UNNotificationAttachment {
    let name = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let file =
      (fileExtension ?? imageExtension(data)).map { name.appendingPathExtension($0) } ?? name
    try data.write(to: file)
    do {
      return try UNNotificationAttachment(
        identifier: UUID().uuidString, url: file, options: options(spec))
    } catch {
      try? FileManager.default.removeItem(at: file)
      throw error
    }
  }

  /// The file extension of the image type `data` holds, if it is an image.
  private static func imageExtension(_ data: Data) -> String? {
    guard let image = CGImageSourceCreateWithData(data as CFData, nil),
      let type = CGImageSourceGetType(image)
    else { return nil }
    return UTType(type as String)?.preferredFilenameExtension
  }

  /// Calls `completion` off the main thread with the contents at `source` and the file extension
  /// of their type, from the response's `Content-Type` or the path, failing unless they arrive
  /// within `timeout`.
  private func data(
    _ source: String, timeout: TimeInterval,
    completion: @escaping (Result<(Data, String?), Error>) -> Void
  ) {
    if source.hasPrefix("/") {
      host.fetch(source, timeout: timeout) { result in
        DispatchQueue.global(qos: .userInitiated).async {
          completion(result.map { ($0, Self.pathExtension(URL(string: source))) })
        }
      }
      return
    }
    guard let url = URL(string: source) else {
      completion(.failure(URLError(.badURL)))
      return
    }
    let task = URLSession.shared.dataTask(with: url) { data, response, error in
      guard let data, let response = response as? HTTPURLResponse else {
        completion(.failure(error ?? URLError(.badServerResponse)))
        return
      }
      guard (200..<300).contains(response.statusCode) else {
        completion(.failure(URLError(.badServerResponse)))
        return
      }
      let fileExtension =
        response.mimeType.flatMap { UTType(mimeType: $0)?.preferredFilenameExtension }
        ?? Self.pathExtension(url)
      completion(.success((data, fileExtension)))
    }
    task.resume()
    // A request's own timeout applies between packets, so the task is cancelled at the deadline.
    DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { task.cancel() }
  }

  /// The extension of `url`'s path, without its query, or nil when it has none.
  private static func pathExtension(_ url: URL?) -> String? {
    url.map(\.pathExtension).flatMap { $0.isEmpty ? nil : $0 }
  }

  private static func options(_ spec: [String: Any]) -> [AnyHashable: Any] {
    var options: [AnyHashable: Any] = [:]
    if let hidden = spec["hideThumbnail"] as? Bool {
      options[UNNotificationAttachmentOptionsThumbnailHiddenKey] = hidden
    }
    if let rect = spec["thumbnailClippingRect"] as? [String: Double] {
      options[UNNotificationAttachmentOptionsThumbnailClippingRectKey] =
        CGRect(
          x: rect["x"] ?? 0, y: rect["y"] ?? 0, width: rect["width"] ?? 1,
          height: rect["height"] ?? 1
        ).dictionaryRepresentation
    }
    if let time = spec["thumbnailTime"] as? Double {
      options[UNNotificationAttachmentOptionsThumbnailTimeKey] = time
    }
    return options
  }
}
