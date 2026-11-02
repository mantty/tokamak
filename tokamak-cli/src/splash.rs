//! The app's splash screen: an image centred on a background colour, which the
//! shell shows from launch until the app's first page has loaded.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tokamak_cli::{Platform, SHARED_PLATFORM_KEYS, VariableKind};

/// The image width, in points, dp or device-independent pixels, when
/// `splash-width` is unset.
const DEFAULT_WIDTH: &str = "100";
/// The background colour when `splash-background` is unset.
const DEFAULT_BACKGROUND: &str = "#FFFFFF";
/// Each image format: its name, file signature and file extensions.
const IMAGE_FORMATS: [(&str, &[u8], &[&str]); 4] = [
    ("PNG", b"\x89PNG\r\n\x1a\n", &["png"]),
    ("JPEG", b"\xff\xd8\xff", &["jpg", "jpeg"]),
    ("GIF", b"GIF87a", &["gif"]),
    ("GIF", b"GIF89a", &["gif"]),
];

/// The splash keys, which are the shared keys starting `splash-`, with their kinds.
pub(crate) fn keys() -> impl Iterator<Item = (&'static str, VariableKind)> {
    SHARED_PLATFORM_KEYS
        .into_iter()
        .filter(|(key, _, _)| key.starts_with("splash-"))
        .map(|(key, kind, _)| (key, kind))
}

/// `platform`'s splash values, by key, checked, with defaults for the width and
/// background, and the dark image and background falling back to the light
/// ones. Empty when `values` sets no image or background.
pub(crate) fn complete(
    platform: Platform,
    mut values: BTreeMap<&'static str, OsString>,
) -> Result<BTreeMap<&'static str, OsString>> {
    let shown = [
        "splash-image",
        "splash-image-dark",
        "splash-background",
        "splash-background-dark",
    ];
    if !shown.iter().any(|key| values.contains_key(key)) {
        return Ok(BTreeMap::new());
    }
    for (key, value) in &values {
        let problem = match *key {
            "splash-image" | "splash-image-dark" => image_problem(Path::new(value)),
            "splash-width" => width_problem(value),
            _ => colour_problem(value),
        };
        if let Some(problem) = problem {
            bail!("{} {key} {problem}", platform.display_name());
        }
    }
    values.entry("splash-width").or_insert(DEFAULT_WIDTH.into());
    values
        .entry("splash-background")
        .or_insert(DEFAULT_BACKGROUND.into());
    for (dark, light) in [
        ("splash-image-dark", "splash-image"),
        ("splash-background-dark", "splash-background"),
    ] {
        if let Some(value) = values.get(light).filter(|_| !values.contains_key(dark)) {
            values.insert(dark, value.clone());
        }
    }
    Ok(values)
}

/// Why `path` cannot be a splash image, when it cannot: it must be a PNG, JPEG
/// or GIF file with one of its format's extensions.
fn image_problem(path: &Path) -> Option<String> {
    let mut start = [0; 8];
    let read = File::open(path)
        .and_then(|mut file| file.read(&mut start))
        .with_context(|| format!("cannot be read: {}", path.display()));
    let read = match read {
        Ok(read) => read,
        Err(error) => return Some(format!("{error:#}")),
    };
    let Some((format, _, extensions)) = IMAGE_FORMATS
        .into_iter()
        .find(|(_, signature, _)| start[..read].starts_with(signature))
    else {
        return Some(format!(
            "must be a PNG, JPEG or GIF image: {}",
            path.display()
        ));
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.is_some_and(|extension| extensions.contains(&extension.as_str())) {
        return None;
    }
    Some(format!(
        "is a {format} image, so its name must end in .{}: {}",
        extensions.join(" or ."),
        path.display()
    ))
}

fn width_problem(value: &OsString) -> Option<String> {
    let width = value.to_str().and_then(|value| value.parse::<u32>().ok());
    width
        .is_none_or(|width| width == 0)
        .then(|| "must be a positive integer".to_owned())
}

fn colour_problem(value: &OsString) -> Option<String> {
    let hex = value.to_str().and_then(|value| value.strip_prefix('#'));
    let valid =
        hex.is_some_and(|hex| hex.len() == 6 && hex.chars().all(|digit| digit.is_ascii_hexdigit()));
    (!valid).then(|| "must be a colour written as #RRGGBB".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    use anyhow::Result;
    use tokamak_cli::{Platform, VariableKind};

    use super::{complete, keys};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    fn values(entries: &[(&'static str, &str)]) -> BTreeMap<&'static str, OsString> {
        entries
            .iter()
            .map(|(key, value)| (*key, OsString::from(value)))
            .collect()
    }

    fn file(directory: &Path, name: &str, contents: &[u8]) -> Result<String> {
        let path = directory.join(name);
        fs::write(&path, contents)?;
        Ok(path.to_string_lossy().into_owned())
    }

    fn problem(entries: &[(&'static str, &str)]) -> String {
        match complete(Platform::Android, values(entries)) {
            Ok(values) => format!("accepted {values:?}"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn lists_the_splash_keys_with_image_paths() {
        assert_eq!(
            keys().collect::<Vec<_>>(),
            [
                ("splash-image", VariableKind::Path),
                ("splash-image-dark", VariableKind::Path),
                ("splash-width", VariableKind::String),
                ("splash-background", VariableKind::String),
                ("splash-background-dark", VariableKind::String),
            ]
        );
    }

    #[test]
    fn has_no_splash_without_an_image_or_background() -> Result<()> {
        assert!(complete(Platform::Ios, values(&[]))?.is_empty());
        assert!(complete(Platform::Ios, values(&[("splash-width", "0")]))?.is_empty());
        Ok(())
    }

    #[test]
    fn applies_defaults_and_dark_fallbacks() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let image = file(temporary.path(), "splash.PNG", PNG)?;
        assert_eq!(
            complete(Platform::Ios, values(&[("splash-image", &image)]))?,
            values(&[
                ("splash-image", &image),
                ("splash-image-dark", &image),
                ("splash-width", "100"),
                ("splash-background", "#FFFFFF"),
                ("splash-background-dark", "#FFFFFF"),
            ])
        );
        assert_eq!(
            complete(
                Platform::Ios,
                values(&[
                    ("splash-width", "120"),
                    ("splash-background", "#0b1220"),
                    ("splash-background-dark", "#000000"),
                ])
            )?,
            values(&[
                ("splash-width", "120"),
                ("splash-background", "#0b1220"),
                ("splash-background-dark", "#000000"),
            ])
        );
        let dark = file(temporary.path(), "dark.gif", b"GIF89a")?;
        assert_eq!(
            complete(Platform::Ios, values(&[("splash-image-dark", &dark)]))?,
            values(&[
                ("splash-image-dark", &dark),
                ("splash-width", "100"),
                ("splash-background", "#FFFFFF"),
                ("splash-background-dark", "#FFFFFF"),
            ])
        );
        Ok(())
    }

    #[test]
    fn accepts_png_jpeg_and_gif_images() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        for (name, contents) in [
            ("a.png", PNG),
            ("b.jpg", b"\xff\xd8\xff\xe0".as_slice()),
            ("c.JPEG", b"\xff\xd8\xff\xdb".as_slice()),
            ("d.gif", b"GIF87a".as_slice()),
            ("e.gif", b"GIF89a".as_slice()),
        ] {
            let image = file(temporary.path(), name, contents)?;
            complete(Platform::Ios, values(&[("splash-image", &image)]))?;
        }
        Ok(())
    }

    #[test]
    fn rejects_other_images_and_mismatched_names() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let webp = file(temporary.path(), "splash.png", b"RIFF\0\0\0\0WEBP")?;
        assert_eq!(
            problem(&[("splash-image", &webp)]),
            format!("Android splash-image must be a PNG, JPEG or GIF image: {webp}")
        );
        let jpeg = file(temporary.path(), "splash.png", b"\xff\xd8\xff\xe0")?;
        assert_eq!(
            problem(&[("splash-image-dark", &jpeg)]),
            format!(
                "Android splash-image-dark is a JPEG image, so its name must end in .jpg or .jpeg: {jpeg}"
            )
        );
        let unnamed = file(temporary.path(), "splash", PNG)?;
        assert_eq!(
            problem(&[("splash-image", &unnamed)]),
            format!("Android splash-image is a PNG image, so its name must end in .png: {unnamed}")
        );
        let empty = file(temporary.path(), "empty.gif", b"")?;
        assert_eq!(
            problem(&[("splash-image", &empty)]),
            format!("Android splash-image must be a PNG, JPEG or GIF image: {empty}")
        );
        let missing = temporary.path().join("missing.png");
        let missing = missing.to_string_lossy();
        assert!(
            problem(&[("splash-image", &missing)])
                .starts_with(&format!("Android splash-image cannot be read: {missing}: ")),
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_widths_and_colours() {
        let colour = "must be a colour written as #RRGGBB";
        for (entries, expected) in [
            (
                [("splash-background", "#000000"), ("splash-width", "0")],
                "Android splash-width must be a positive integer",
            ),
            (
                [("splash-background", "#000000"), ("splash-width", "1.5")],
                "Android splash-width must be a positive integer",
            ),
            (
                [("splash-background", "#000000"), ("splash-width", "-2")],
                "Android splash-width must be a positive integer",
            ),
            (
                [("splash-background", "#FFF"), ("splash-width", "1")],
                &format!("Android splash-background {colour}"),
            ),
            (
                [("splash-background", "red"), ("splash-width", "1")],
                &format!("Android splash-background {colour}"),
            ),
            (
                [("splash-background-dark", "#00000G"), ("splash-width", "1")],
                &format!("Android splash-background-dark {colour}"),
            ),
        ] {
            assert_eq!(problem(&entries), expected);
        }
    }
}
