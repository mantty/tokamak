import Foundation
import ImageIO

#if os(macOS)
  import AppKit
#else
  import UIKit
#endif

/// The asset catalog colour the build writes from the splash background.
private let backgroundColorName = "TokamakSplashBackground"

/// The app's splash screen: its image centred on its background colour, shown
/// over the WebView from launch until the app's first page has loaded. The
/// build names the images and width in Info.plist.
final class TokamakSplash {
  #if os(macOS)
    let view: NSView
  #else
    let view: UIView
    /// Whether the image's animation stops at its next frame.
    private var stopped = false
  #endif

  /// The width the image is drawn at, in points.
  private static var width: CGFloat? {
    (Bundle.main.object(forInfoDictionaryKey: "TokamakSplashWidth") as? NSNumber)
      .map { CGFloat($0.doubleValue) }
  }

  /// The image file for the appearance, if the splash has one.
  private static func imageURL(dark: Bool) -> URL? {
    let key = dark ? "TokamakSplashImageDark" : "TokamakSplashImage"
    return (Bundle.main.object(forInfoDictionaryKey: key) as? String)
      .flatMap { Bundle.main.url(forResource: $0, withExtension: nil) }
  }

  /// The image's frame: `width` wide at its aspect ratio, centred in `bounds`.
  private static func imageFrame(size: CGSize, width: CGFloat, in bounds: CGRect) -> CGRect {
    let height = size.width > 0 ? width * size.height / size.width : 0
    return CGRect(
      x: bounds.midX - width / 2, y: bounds.midY - height / 2, width: width, height: height
    )
  }

  #if os(macOS)
    /// The app's splash filling `frame`, in dark mode or not, or nil when the app has none.
    init?(frame: CGRect, dark: Bool) {
      guard let width = Self.width else { return nil }
      view = BackgroundView(frame: frame)
      view.autoresizingMask = [.width, .height]
      view.setAccessibilityElement(false)
      guard let url = Self.imageURL(dark: dark), let image = NSImage(contentsOf: url) else {
        return
      }
      // An image view plays an animated GIF by itself.
      let imageView = NSImageView(
        frame: Self.imageFrame(size: image.size, width: width, in: view.bounds)
      )
      imageView.image = image
      imageView.imageScaling = .scaleProportionallyUpOrDown
      imageView.autoresizingMask = [.minXMargin, .maxXMargin, .minYMargin, .maxYMargin]
      imageView.setAccessibilityElement(false)
      view.addSubview(imageView)
    }

    func remove() {
      view.removeFromSuperview()
    }

    /// A view filled with the splash background in its appearance.
    private final class BackgroundView: NSView {
      override var wantsUpdateLayer: Bool { true }

      override func updateLayer() {
        layer?.backgroundColor = NSColor(named: backgroundColorName)?.cgColor
      }
    }
  #else
    /// The app's splash filling `frame`, in dark mode or not, or nil when the app has none.
    init?(frame: CGRect, dark: Bool) {
      guard let width = Self.width else { return nil }
      view = UIView(frame: frame)
      view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      view.backgroundColor = UIColor(named: backgroundColorName)
      view.accessibilityElementsHidden = true
      guard
        let url = Self.imageURL(dark: dark),
        let source = CGImageSourceCreateWithURL(url as CFURL, nil)
      else { return }
      let animated = CGImageSourceGetCount(source) > 1
      // A JPEG's orientation applies through UIImage; a GIF has none.
      let image =
        animated
        ? CGImageSourceCreateImageAtIndex(source, 0, nil).map { UIImage(cgImage: $0) }
        : UIImage(contentsOfFile: url.path)
      guard let image else { return }
      let imageView = UIImageView(
        frame: Self.imageFrame(size: image.size, width: width, in: view.bounds)
      )
      imageView.image = image
      imageView.contentMode = .scaleAspectFit
      imageView.autoresizingMask = [
        .flexibleLeftMargin, .flexibleRightMargin, .flexibleTopMargin, .flexibleBottomMargin,
      ]
      view.addSubview(imageView)
      guard animated else { return }
      // An image view does not play a GIF, so each frame is shown at the
      // file's timing, from the main queue.
      CGAnimateImageAtURLWithBlock(url as CFURL, nil) {
        [weak self, weak imageView] _, frame, stop in
        guard let self, let imageView, !self.stopped else {
          stop.pointee = true
          return
        }
        imageView.image = UIImage(cgImage: frame)
      }
    }

    func remove() {
      stopped = true
      view.removeFromSuperview()
    }
  #endif
}
