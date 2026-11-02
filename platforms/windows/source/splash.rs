//! The app's splash screen: its image centred on its background colour, drawn
//! in the window from launch until the app's first page has loaded.

use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;
use tao::dpi::LogicalUnit;
use tao::platform::windows::WindowExtWindows;
use tao::window::{Theme, Window};
use windows::Win32::Foundation::{COLORREF, GENERIC_READ, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateSolidBrush, DIB_RGB_COLORS, DeleteObject,
    ExcludeClipRect, FillRect, GetDC, HDC, ReleaseDC, SelectClipRgn, SetDIBitsToDevice,
    ValidateRect,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapFrameDecode,
    IWICImagingFactory, IWICMetadataQueryReader, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeMedianCut,
    WICDecodeMetadataCacheOnLoad,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToUInt16,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::{VARENUM, VT_UI1, VT_VECTOR};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{HSTRING, PCWSTR, w};

/// A GIF frame's disposal that restores the canvas to how it was before the frame.
const RESTORE_PREVIOUS: u16 = 3;
/// A GIF frame's disposal that clears its rectangle once it has been shown.
const RESTORE_BACKGROUND: u16 = 2;

/// The splash the build writes into `tokamak.json`: image files beside the
/// executable, the image width in device-independent pixels, and `#RRGGBB`
/// background colours.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplashConfig {
    image: Option<String>,
    image_dark: Option<String>,
    width: u32,
    background: String,
    background_dark: String,
}

/// The splash for the window's theme when the window opened.
pub(crate) struct Splash {
    background: [u8; 3],
    image: Option<Animation>,
}

impl Splash {
    /// The splash `config` describes for `window`, whose background takes the
    /// splash colour. An image that cannot be decoded is left out.
    ///
    /// # Errors
    ///
    /// Returns an error when a background colour is not `#RRGGBB`.
    pub(crate) fn open(root: &Path, config: &SplashConfig, window: &Window) -> Result<Self> {
        let (image, background) = if window.theme() == Theme::Dark {
            (&config.image_dark, &config.background_dark)
        } else {
            (&config.image, &config.background)
        };
        let background = colour(background)?;
        let [red, green, blue] = background;
        window.set_background_color(Some((red, green, blue, 255)));
        let width = LogicalUnit::new(f64::from(config.width))
            .to_physical::<u32>(window.scale_factor())
            .0;
        let image = image.as_ref().and_then(|name| {
            let path = root.join(name);
            Animation::decode(&path, width, background)
                .inspect_err(|error| {
                    eprintln!(
                        "tokamak could not decode the splash image {}: {error:#}",
                        path.display()
                    );
                })
                .ok()
        });
        Ok(Self { background, image })
    }

    /// When the image's next frame is due, while it animates.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.image.as_ref().and_then(|image| image.due)
    }

    /// Shows the image's next frame once it is due at `now`, redrawing `window`.
    pub(crate) fn animate(&mut self, window: &Window, now: Instant) {
        if self.image.as_mut().is_some_and(|image| image.advance(now)) {
            window.request_redraw();
        }
    }

    /// Draws the splash over `window`'s client area. The window is then
    /// valid, so Windows does not erase the drawing.
    pub(crate) fn paint(&self, window: &Window) {
        let hwnd = HWND(window.hwnd() as _);
        let mut client = RECT::default();
        if unsafe { GetClientRect(hwnd, &raw mut client) }.is_err() {
            return;
        }
        let [red, green, blue] = self.background;
        let colour = u32::from(red) | u32::from(green) << 8 | u32::from(blue) << 16;
        let image = self
            .image
            .as_ref()
            .map(|image| (image, image.origin(&client)));
        unsafe {
            let dc = GetDC(Some(hwnd));
            if let Some((image, (x, y))) = image {
                ExcludeClipRect(dc, x, y, x + image.width, y + image.height);
            }
            let brush = CreateSolidBrush(COLORREF(colour));
            FillRect(dc, &raw const client, brush);
            let _ = DeleteObject(brush.into());
            SelectClipRgn(dc, None);
            if let Some((image, origin)) = image {
                image.draw(dc, origin);
            }
            ReleaseDC(Some(hwnd), dc);
            let _ = ValidateRect(Some(hwnd), None);
        }
    }
}

/// The image's frames, scaled to the splash width and drawn over the
/// background, and the frame on show.
struct Animation {
    width: i32,
    height: i32,
    /// How the frames' pixels are laid out.
    info: BITMAPINFO,
    frames: Vec<Frame>,
    /// How many times the frames play, or `None` to repeat them forever.
    plays: Option<u32>,
    shown: usize,
    played: u32,
    /// When the next frame is due, while the image animates.
    due: Option<Instant>,
}

struct Frame {
    /// Opaque BGRA pixels, top row first.
    pixels: Vec<u8>,
    delay: Duration,
}

impl Animation {
    /// Decodes the image at `path` `width` pixels wide, at its aspect ratio,
    /// with each frame drawn over `background`.
    fn decode(path: &Path, width: u32, background: [u8; 3]) -> Result<Self> {
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let decoder = unsafe {
            factory.CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )?
        };
        let file = unsafe { decoder.GetMetadataQueryReader() }.ok();
        let screen = file.as_ref().and_then(|file| {
            let width = number(file, w!("/logscrdesc/Width"))?;
            let height = number(file, w!("/logscrdesc/Height"))?;
            (width > 0 && height > 0).then(|| (u32::from(width), u32::from(height)))
        });
        let count = unsafe { decoder.GetFrameCount()? };
        let mut canvas = None;
        let mut frames = Vec::new();
        for index in 0..count {
            let layer = Layer::read(&factory, &unsafe { decoder.GetFrame(index)? })?;
            let canvas = canvas.get_or_insert_with(|| {
                let (width, height) = screen.unwrap_or((layer.width, layer.height));
                Canvas::new(width, height)
            });
            frames.push(canvas.show(&layer, |canvas| render(&factory, canvas, width, background))?);
        }
        let canvas = canvas.context("image has no frames")?;
        let height = scaled_height(width, canvas.width, canvas.height);
        let plays = plays(
            file.as_ref()
                .and_then(|file| bytes(file, w!("/appext/Application")))
                .as_deref(),
            file.as_ref()
                .and_then(|file| bytes(file, w!("/appext/Data")))
                .as_deref(),
        );
        let due = (frames.len() > 1).then(|| Instant::now() + frames[0].delay);
        let (width, height) = (i32::try_from(width)?, i32::try_from(height)?);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>())?,
                biWidth: width,
                // A negative height stores the top row first.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        Ok(Self {
            width,
            height,
            info,
            frames,
            plays,
            shown: 0,
            played: 0,
            due,
        })
    }

    /// Shows the next frame once it is due at `now`, and returns whether the
    /// frame changed. The last frame of the last play stays.
    fn advance(&mut self, now: Instant) -> bool {
        if self.due.is_none_or(|due| due > now) {
            return false;
        }
        self.shown = (self.shown + 1) % self.frames.len();
        if self.shown == 0 {
            self.played += 1;
        }
        let last = self.shown + 1 == self.frames.len() && self.plays == Some(self.played + 1);
        self.due = (!last).then(|| now + self.frames[self.shown].delay);
        true
    }

    /// Where the image's top-left corner goes to centre it in `client`.
    fn origin(&self, client: &RECT) -> (i32, i32) {
        (
            client.left + (client.right - client.left - self.width) / 2,
            client.top + (client.bottom - client.top - self.height) / 2,
        )
    }

    fn draw(&self, dc: HDC, (x, y): (i32, i32)) {
        let rows = self.height.unsigned_abs();
        unsafe {
            SetDIBitsToDevice(
                dc,
                x,
                y,
                self.width.unsigned_abs(),
                rows,
                0,
                0,
                0,
                rows,
                self.frames[self.shown].pixels.as_ptr().cast(),
                &raw const self.info,
                DIB_RGB_COLORS,
            );
        }
    }
}

/// One frame as the file stores it: premultiplied BGRA pixels for a
/// rectangle of the canvas, how long the frame shows, and what happens to its
/// rectangle afterwards.
struct Layer {
    pixels: Vec<u8>,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    delay: Duration,
    disposal: u16,
}

impl Layer {
    fn read(factory: &IWICImagingFactory, frame: &IWICBitmapFrameDecode) -> Result<Self> {
        let converter = unsafe { factory.CreateFormatConverter()? };
        let (mut width, mut height) = (0, 0);
        unsafe {
            converter.Initialize(
                frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            converter.GetSize(&raw mut width, &raw mut height)?;
        }
        let mut pixels = vec![0; pixel_bytes(width, height)];
        unsafe { converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)? };
        let metadata = unsafe { frame.GetMetadataQueryReader() }.ok();
        let value = |name| {
            metadata
                .as_ref()
                .and_then(|metadata| number(metadata, name))
        };
        Ok(Self {
            pixels,
            left: value(w!("/imgdesc/Left")).map_or(0, u32::from),
            top: value(w!("/imgdesc/Top")).map_or(0, u32::from),
            width,
            height,
            delay: frame_delay(value(w!("/grctlext/Delay"))),
            disposal: value(w!("/grctlext/Disposal")).unwrap_or(0),
        })
    }
}

/// The premultiplied BGRA pixels the frames are drawn on, at the image's size.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; pixel_bytes(width, height)],
        }
    }

    /// Draws `layer` over the canvas, renders the result with `render`, then
    /// applies the layer's disposal for the next frame.
    fn show(
        &mut self,
        layer: &Layer,
        render: impl FnOnce(&Self) -> Result<Vec<u8>>,
    ) -> Result<Frame> {
        let previous = (layer.disposal == RESTORE_PREVIOUS).then(|| self.pixels.clone());
        self.draw(layer);
        let pixels = render(self)?;
        if let Some(previous) = previous {
            self.pixels = previous;
        } else if layer.disposal == RESTORE_BACKGROUND {
            self.clear(layer);
        }
        Ok(Frame {
            pixels,
            delay: layer.delay,
        })
    }

    fn draw(&mut self, layer: &Layer) {
        let rows = layer.pixels.chunks_exact(pixel_bytes(layer.width, 1));
        for (row, source) in (layer.top..self.height).zip(rows) {
            let span = self.span(layer.left, row, layer.width);
            for (target, source) in self.pixels[span]
                .chunks_exact_mut(4)
                .zip(source.chunks_exact(4))
            {
                over(target, source);
            }
        }
    }

    /// Clears `layer`'s rectangle to transparent.
    fn clear(&mut self, layer: &Layer) {
        for row in layer.top..self.height.min(layer.top.saturating_add(layer.height)) {
            let span = self.span(layer.left, row, layer.width);
            self.pixels[span].fill(0);
        }
    }

    /// The bytes of `row` from column `left`, `width` pixels long, within the canvas.
    fn span(&self, left: u32, row: u32, width: u32) -> Range<usize> {
        let start = self.width.min(left);
        let end = self.width.min(left.saturating_add(width));
        let row = pixel_bytes(self.width, row);
        row + pixel_bytes(start, 1)..row + pixel_bytes(end, 1)
    }
}

/// `canvas` scaled `width` pixels wide and drawn over `background`, as opaque
/// BGRA pixels.
fn render(
    factory: &IWICImagingFactory,
    canvas: &Canvas,
    width: u32,
    background: [u8; 3],
) -> Result<Vec<u8>> {
    let height = scaled_height(width, canvas.width, canvas.height);
    let mut pixels = vec![0; pixel_bytes(width, height)];
    unsafe {
        let bitmap = factory.CreateBitmapFromMemory(
            canvas.width,
            canvas.height,
            &GUID_WICPixelFormat32bppPBGRA,
            canvas.width * 4,
            &canvas.pixels,
        )?;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(
            &bitmap,
            width,
            height,
            WICBitmapInterpolationModeHighQualityCubic,
        )?;
        scaler.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
    }
    let [red, green, blue] = background;
    for pixel in pixels.chunks_exact_mut(4) {
        let mut opaque = [blue, green, red, 255];
        over(&mut opaque, pixel);
        pixel.copy_from_slice(&opaque);
    }
    Ok(pixels)
}

/// Draws the premultiplied pixel `source` over `target`.
fn over(target: &mut [u8], source: &[u8]) {
    let uncovered = 255 - u32::from(source[3]);
    for (target, source) in target.iter_mut().zip(source) {
        let value = u32::from(*source) + (u32::from(*target) * uncovered + 127) / 255;
        *target = u8::try_from(value).unwrap_or(u8::MAX);
    }
}

/// The number of bytes in `width` × `height` BGRA pixels.
fn pixel_bytes(width: u32, height: u32) -> usize {
    usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(usize::MAX)
}

/// The height of an image `width` pixels wide at the aspect ratio of
/// `original_width` × `original_height`, at least one pixel.
fn scaled_height(width: u32, original_width: u32, original_height: u32) -> u32 {
    let height = (u64::from(width) * u64::from(original_height) + u64::from(original_width) / 2)
        / u64::from(original_width.max(1));
    u32::try_from(height).unwrap_or(u32::MAX).max(1)
}

/// How long a GIF frame with a delay of `centiseconds` shows. Delays of 0 and
/// 10 ms play as 100 ms, as browsers and Apple's `ImageIO` play them.
fn frame_delay(centiseconds: Option<u16>) -> Duration {
    match centiseconds {
        None | Some(0 | 1) => Duration::from_millis(100),
        Some(centiseconds) => Duration::from_millis(u64::from(centiseconds) * 10),
    }
}

/// How many times a GIF plays from its `NETSCAPE2.0` application extension:
/// once without one, forever for a loop count of 0, and otherwise the loop
/// count after the first play.
fn plays(application: Option<&[u8]>, data: Option<&[u8]>) -> Option<u32> {
    let looping = matches!(application, Some(b"NETSCAPE2.0" | b"ANIMEXTS1.0"));
    match data {
        Some(&[size, 1, low, high, ..]) if looping && size >= 3 => {
            match u16::from_le_bytes([low, high]) {
                0 => None,
                count => Some(u32::from(count) + 1),
            }
        }
        _ => Some(1),
    }
}

/// `#RRGGBB` as red, green and blue.
fn colour(hex: &str) -> Result<[u8; 3]> {
    let parse = || -> Option<[u8; 3]> {
        let digits = hex.strip_prefix('#').filter(|digits| digits.len() == 6)?;
        let component = |index: usize| u8::from_str_radix(digits.get(index..index + 2)?, 16).ok();
        Some([component(0)?, component(2)?, component(4)?])
    };
    parse().with_context(|| format!("splash colour must be written as #RRGGBB: {hex}"))
}

/// The metadata value `name`, if the image has it as a number.
fn number(reader: &IWICMetadataQueryReader, name: PCWSTR) -> Option<u16> {
    let mut value = PROPVARIANT::default();
    unsafe {
        reader.GetMetadataByName(name, &raw mut value).ok()?;
        let number = PropVariantToUInt16(&raw const value).ok();
        let _ = PropVariantClear(&raw mut value);
        number
    }
}

/// The metadata value `name`, if the image has it as bytes.
fn bytes(reader: &IWICMetadataQueryReader, name: PCWSTR) -> Option<Vec<u8>> {
    let mut value = PROPVARIANT::default();
    unsafe {
        reader.GetMetadataByName(name, &raw mut value).ok()?;
        let variant = &value.Anonymous.Anonymous;
        let vector = variant.Anonymous.caub;
        let bytes = (variant.vt == VARENUM(VT_VECTOR.0 | VT_UI1.0) && !vector.pElems.is_null())
            .then(|| {
                let length = usize::try_from(vector.cElems).unwrap_or_default();
                std::slice::from_raw_parts(vector.pElems, length).to_vec()
            });
        let _ = PropVariantClear(&raw mut value);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::BITMAPINFO;

    use super::{
        Animation, Canvas, Frame, Layer, SplashConfig, colour, frame_delay, over, plays,
        scaled_height,
    };

    fn layer(left: u32, top: u32, width: u32, height: u32, pixel: [u8; 4], disposal: u16) -> Layer {
        Layer {
            pixels: pixel.repeat(usize::try_from(width * height).unwrap_or_default()),
            left,
            top,
            width,
            height,
            delay: Duration::from_millis(100),
            disposal,
        }
    }

    fn animation(frames: usize, plays: Option<u32>, start: Instant) -> Animation {
        Animation {
            width: 1,
            height: 1,
            info: BITMAPINFO::default(),
            frames: (0..frames)
                .map(|_| Frame {
                    pixels: vec![0; 4],
                    delay: Duration::from_millis(100),
                })
                .collect(),
            plays,
            shown: 0,
            played: 0,
            due: Some(start + Duration::from_millis(100)),
        }
    }

    /// Shows `layers` in turn on a 3 × 1 canvas, returning the canvas as each one shows.
    fn shown(layers: &[Layer]) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut canvas = Canvas::new(3, 1);
        layers
            .iter()
            .map(|layer| {
                canvas
                    .show(layer, |canvas| Ok(canvas.pixels.clone()))
                    .map(|frame| frame.pixels)
            })
            .collect()
    }

    const RED: [u8; 4] = [0, 0, 255, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];
    const CLEAR: [u8; 4] = [0; 4];

    #[test]
    fn reads_the_splash_from_the_shell_config() -> anyhow::Result<()> {
        let config: SplashConfig = serde_json::from_str(
            r##"{"image":"splash.png","imageDark":"splash-dark.gif","width":120,"background":"#0B1220","backgroundDark":"#000000"}"##,
        )?;
        assert_eq!(
            config,
            SplashConfig {
                image: Some("splash.png".into()),
                image_dark: Some("splash-dark.gif".into()),
                width: 120,
                background: "#0B1220".into(),
                background_dark: "#000000".into(),
            }
        );
        let config: SplashConfig = serde_json::from_str(
            r##"{"width":100,"background":"#FFFFFF","backgroundDark":"#FFFFFF"}"##,
        )?;
        assert_eq!(config.image, None);
        Ok(())
    }

    #[test]
    fn parses_colours_written_as_hex() -> anyhow::Result<()> {
        assert_eq!(colour("#0B1220")?, [0x0B, 0x12, 0x20]);
        assert_eq!(colour("#ffffff")?, [255, 255, 255]);
        for invalid in ["0B1220", "#0B122", "#0B122G", "#0B12201", "#é0000"] {
            assert!(colour(invalid).is_err(), "{invalid}");
        }
        Ok(())
    }

    #[test]
    fn composites_frames_with_their_disposal() -> anyhow::Result<()> {
        let frames = shown(&[
            layer(0, 0, 3, 1, RED, 1),
            layer(1, 0, 1, 1, BLUE, super::RESTORE_PREVIOUS),
            layer(2, 0, 1, 1, BLUE, super::RESTORE_BACKGROUND),
            layer(0, 0, 1, 1, CLEAR, 0),
        ])?;
        assert_eq!(frames[0], [RED, RED, RED].concat());
        assert_eq!(frames[1], [RED, BLUE, RED].concat());
        assert_eq!(frames[2], [RED, RED, BLUE].concat());
        assert_eq!(frames[3], [RED, RED, CLEAR].concat());
        Ok(())
    }

    #[test]
    fn clips_frames_to_the_canvas() -> anyhow::Result<()> {
        let frames = shown(&[layer(2, 0, 2, 2, BLUE, 0), layer(5, 3, 1, 1, RED, 0)])?;
        assert_eq!(frames[0], [CLEAR, CLEAR, BLUE].concat());
        assert_eq!(frames[1], frames[0]);
        Ok(())
    }

    #[test]
    fn draws_premultiplied_pixels_over_others() {
        let mut target = [200, 100, 0, 255];
        over(&mut target, &[0, 0, 128, 128]);
        assert_eq!(target, [100, 50, 128, 255]);
        let mut target = [10, 20, 30, 255];
        over(&mut target, &CLEAR);
        assert_eq!(target, [10, 20, 30, 255]);
    }

    #[test]
    fn plays_short_delays_as_a_tenth_of_a_second() {
        assert_eq!(frame_delay(None), Duration::from_millis(100));
        assert_eq!(frame_delay(Some(0)), Duration::from_millis(100));
        assert_eq!(frame_delay(Some(1)), Duration::from_millis(100));
        assert_eq!(frame_delay(Some(2)), Duration::from_millis(20));
        assert_eq!(frame_delay(Some(50)), Duration::from_millis(500));
    }

    #[test]
    fn reads_the_loop_count_from_the_netscape_extension() {
        let netscape = Some(b"NETSCAPE2.0".as_slice());
        assert_eq!(plays(netscape, Some(&[3, 1, 0, 0, 0])), None);
        assert_eq!(plays(netscape, Some(&[3, 1, 2, 0])), Some(3));
        assert_eq!(plays(netscape, Some(&[3, 1, 0, 1])), Some(257));
        assert_eq!(plays(Some(b"ANIMEXTS1.0"), Some(&[3, 1, 0, 0])), None);
        assert_eq!(plays(Some(b"XMPDataXMP"), Some(&[3, 1, 0, 0])), Some(1));
        assert_eq!(plays(netscape, Some(&[3, 2, 0, 0])), Some(1));
        assert_eq!(plays(None, None), Some(1));
    }

    #[test]
    fn steps_through_frames_until_the_last_play_ends() {
        let start = Instant::now();
        let mut once = animation(2, Some(1), start);
        assert!(!once.advance(start));
        assert!(once.advance(start + Duration::from_millis(100)));
        assert_eq!((once.shown, once.due), (1, None));
        assert!(!once.advance(start + Duration::from_secs(1)));

        let mut twice = animation(2, Some(2), start);
        let shown: Vec<_> = (1..=4)
            .map(|step| {
                twice.advance(start + Duration::from_millis(100 * step));
                (twice.shown, twice.due.is_some())
            })
            .collect();
        assert_eq!(shown, [(1, true), (0, true), (1, false), (1, false)]);

        let mut forever = animation(2, None, start);
        for step in 1..=10 {
            assert!(forever.advance(start + Duration::from_millis(100 * step)));
        }
    }

    #[test]
    fn keeps_the_aspect_ratio_and_centres_the_image() {
        assert_eq!(scaled_height(120, 400, 200), 60);
        assert_eq!(scaled_height(100, 3, 1), 33);
        assert_eq!(scaled_height(1, 1000, 1), 1);
        let image = Animation {
            width: 100,
            height: 50,
            ..animation(1, Some(1), Instant::now())
        };
        let client = RECT {
            left: 0,
            top: 0,
            right: 801,
            bottom: 600,
        };
        assert_eq!(image.origin(&client), (350, 275));
    }
}
