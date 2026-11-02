//! The app's splash screen, from the `TOKAMAK_<PLATFORM>_SPLASH_*` values tok
//! passes: its images in the bundle's resources, its background colours in the
//! asset catalog, and the Info.plist keys the launch screen and shell read.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use serde_json::json;

/// The asset catalog colour of the splash background, with a dark appearance.
const BACKGROUND_COLOR: &str = "TokamakSplashBackground";

/// The app's splash screen for one platform.
#[derive(Debug, PartialEq)]
pub struct Splash {
    image: Option<PathBuf>,
    image_dark: Option<PathBuf>,
    width: u32,
    background: String,
    background_dark: String,
}

impl Splash {
    /// The splash `platform`'s settings describe in the environment, if the app has one.
    ///
    /// # Errors
    ///
    /// Returns an error when a value is missing or invalid.
    pub fn from_environment(platform: &str) -> Result<Option<Self>> {
        Self::read(platform, |name| env::var_os(name))
    }

    fn read(platform: &str, variable: impl Fn(&str) -> Option<OsString>) -> Result<Option<Self>> {
        let name = |key: &str| format!("TOKAMAK_{}_SPLASH_{key}", platform.to_ascii_uppercase());
        let text = |key: &str| -> Result<String> {
            let name = name(key);
            variable(&name)
                .with_context(|| format!("{name} is required with the splash settings"))?
                .into_string()
                .map_err(|_| anyhow::anyhow!("{name} must contain valid UTF-8"))
        };
        let Some(width) = variable(&name("WIDTH")) else {
            return Ok(None);
        };
        let width = width
            .to_str()
            .and_then(|width| width.parse().ok())
            .with_context(|| format!("{} must be a positive integer", name("WIDTH")))?;
        Ok(Some(Self {
            image: variable(&name("IMAGE")).map(PathBuf::from),
            image_dark: variable(&name("IMAGE_DARK")).map(PathBuf::from),
            width,
            background: text("BACKGROUND")?,
            background_dark: text("BACKGROUND_DARK")?,
        }))
    }

    /// The splash's files: its images, copied into `resources`, and its
    /// background colours, written to the asset catalog `catalog`.
    ///
    /// # Errors
    ///
    /// Returns an error when an image cannot be copied, a colour is invalid,
    /// or the catalog cannot be written.
    pub fn write_files(&self, resources: &Path, catalog: &Path) -> Result<()> {
        for (image, name, _) in self.images()? {
            let destination = resources.join(name);
            fs::copy(image, &destination).with_context(|| {
                format!(
                    "copy splash image {} to {}",
                    image.display(),
                    destination.display()
                )
            })?;
        }
        let color_set = catalog.join(format!("{BACKGROUND_COLOR}.colorset"));
        fs::create_dir_all(&color_set)
            .with_context(|| format!("create {}", color_set.display()))?;
        let info = json!({ "author": "xcode", "version": 1 });
        write_json(&catalog.join("Contents.json"), &json!({ "info": info }))?;
        let colors = json!({
            "colors": [
                { "color": color(&self.background)?, "idiom": "universal" },
                {
                    "appearances": [{ "appearance": "luminosity", "value": "dark" }],
                    "color": color(&self.background_dark)?,
                    "idiom": "universal",
                },
            ],
            "info": info,
        });
        write_json(&color_set.join("Contents.json"), &colors)
    }

    /// Adds the keys the shell reads, and on iOS the launch screen's colour.
    ///
    /// # Errors
    ///
    /// Returns an error when an image has no file extension.
    pub(crate) fn add_to_plist(&self, plist: &mut Dictionary, platform: &str) -> Result<()> {
        for (_, name, key) in self.images()? {
            plist.insert(key.into(), Value::String(name));
        }
        plist.insert(
            "TokamakSplashWidth".into(),
            Value::Integer(self.width.into()),
        );
        if platform == "ios" {
            let mut launch_screen = Dictionary::new();
            launch_screen.insert("UIColorName".into(), Value::String(BACKGROUND_COLOR.into()));
            plist.insert("UILaunchScreen".into(), Value::Dictionary(launch_screen));
        }
        Ok(())
    }

    /// Each image, with its name in the bundle's resources and the Info.plist
    /// key naming it. A dark image that is the light one shares its file.
    fn images(&self) -> Result<Vec<(&Path, String, &'static str)>> {
        let mut images = Vec::new();
        if let Some(image) = &self.image {
            let name = resource_name(image, "TokamakSplash")?;
            images.push((image.as_path(), name, "TokamakSplashImage"));
        }
        if let Some(image) = &self.image_dark {
            let shared = self.image.as_ref() == Some(image);
            let name = resource_name(
                image,
                if shared {
                    "TokamakSplash"
                } else {
                    "TokamakSplashDark"
                },
            )?;
            images.push((image.as_path(), name, "TokamakSplashImageDark"));
        }
        Ok(images)
    }
}

/// `name` with the lowercase extension of `image`, which tok checked matches its format.
fn resource_name(image: &Path, name: &str) -> Result<String> {
    let extension = image
        .extension()
        .and_then(|extension| extension.to_str())
        .with_context(|| format!("splash image has no file extension: {}", image.display()))?;
    Ok(format!("{name}.{}", extension.to_ascii_lowercase()))
}

/// The asset catalog colour of `#RRGGBB`. Each component is written with a
/// decimal point, since actool reads one without as an 8-bit value.
fn color(hex: &str) -> Result<serde_json::Value> {
    let digits = hex
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .with_context(|| format!("splash colour must be written as #RRGGBB: {hex}"))?;
    let component = |index: usize| -> Result<String> {
        let Ok(value) = u8::from_str_radix(&digits[index..index + 2], 16) else {
            bail!("splash colour must be written as #RRGGBB: {hex}");
        };
        Ok(format!("{:.3}", f64::from(value) / 255.0))
    };
    Ok(json!({
        "color-space": "srgb",
        "components": {
            "red": component(0)?,
            "green": component(2)?,
            "blue": component(4)?,
            "alpha": "1.000",
        },
    }))
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use anyhow::Result;
    use plist::{Dictionary, Value};
    use serde_json::json;

    use super::{Splash, color};

    fn splash(image: Option<&str>, image_dark: Option<&str>) -> Splash {
        Splash {
            image: image.map(PathBuf::from),
            image_dark: image_dark.map(PathBuf::from),
            width: 120,
            background: "#0B1220".into(),
            background_dark: "#000000".into(),
        }
    }

    #[test]
    fn reads_the_platform_s_splash_from_the_environment() -> Result<()> {
        let variables = BTreeMap::from([
            ("TOKAMAK_MACOS_SPLASH_IMAGE", "/app/splash.PNG"),
            ("TOKAMAK_MACOS_SPLASH_WIDTH", "120"),
            ("TOKAMAK_MACOS_SPLASH_BACKGROUND", "#0B1220"),
            ("TOKAMAK_MACOS_SPLASH_BACKGROUND_DARK", "#000000"),
            ("TOKAMAK_IOS_SPLASH_WIDTH", "x"),
        ]);
        let variable = |name: &str| variables.get(name).map(OsString::from);

        assert_eq!(
            Splash::read("macos", variable)?,
            Some(splash(Some("/app/splash.PNG"), None))
        );
        assert!(Splash::read("ios", variable).is_err_and(|error| {
            error.to_string() == "TOKAMAK_IOS_SPLASH_WIDTH must be a positive integer"
        }));
        assert_eq!(Splash::read("android", variable)?, None);
        Ok(())
    }

    #[test]
    fn writes_components_with_a_decimal_point() -> Result<()> {
        assert_eq!(
            color("#0B1220")?,
            json!({
                "color-space": "srgb",
                "components": {
                    "red": "0.043",
                    "green": "0.071",
                    "blue": "0.125",
                    "alpha": "1.000",
                },
            })
        );
        assert_eq!(color("#ffffff")?["components"]["red"], "1.000");
        assert_eq!(color("#000000")?["components"]["blue"], "0.000");
        assert!(color("#0B122").is_err());
        assert!(color("#0B122G").is_err());
        Ok(())
    }

    #[test]
    fn copies_the_images_and_writes_the_background_colour_set() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let image = temporary.path().join("logo.JPG");
        let image_dark = temporary.path().join("logo.gif");
        fs::write(&image, "jpeg")?;
        fs::write(&image_dark, "gif")?;
        let resources = temporary.path().join("resources");
        let catalog = temporary.path().join("TokamakSplash.xcassets");
        fs::create_dir(&resources)?;
        let splash = Splash {
            image: Some(image),
            image_dark: Some(image_dark),
            ..splash(None, None)
        };

        splash.write_files(&resources, &catalog)?;

        assert_eq!(fs::read(resources.join("TokamakSplash.jpg"))?, b"jpeg");
        assert_eq!(fs::read(resources.join("TokamakSplashDark.gif"))?, b"gif");
        let colors: serde_json::Value = serde_json::from_slice(&fs::read(
            catalog.join("TokamakSplashBackground.colorset/Contents.json"),
        )?)?;
        assert_eq!(colors["colors"][0]["color"], color("#0B1220")?);
        assert_eq!(colors["colors"][1]["color"], color("#000000")?);
        assert_eq!(
            colors["colors"][1]["appearances"],
            json!([{ "appearance": "luminosity", "value": "dark" }])
        );
        assert!(catalog.join("Contents.json").is_file());
        Ok(())
    }

    #[test]
    fn adds_the_shell_s_keys_and_the_ios_launch_screen_colour() -> Result<()> {
        let mut ios = Dictionary::new();
        splash(Some("/app/splash.png"), Some("/app/dark.GIF")).add_to_plist(&mut ios, "ios")?;
        assert_eq!(
            ios.get("TokamakSplashImage"),
            Some(&Value::String("TokamakSplash.png".into()))
        );
        assert_eq!(
            ios.get("TokamakSplashImageDark"),
            Some(&Value::String("TokamakSplashDark.gif".into()))
        );
        assert_eq!(
            ios.get("TokamakSplashWidth"),
            Some(&Value::Integer(120.into()))
        );
        let mut launch_screen = Dictionary::new();
        launch_screen.insert(
            "UIColorName".into(),
            Value::String("TokamakSplashBackground".into()),
        );
        assert_eq!(
            ios.get("UILaunchScreen"),
            Some(&Value::Dictionary(launch_screen))
        );

        let mut shared = Dictionary::new();
        splash(Some("/app/splash.png"), Some("/app/splash.png"))
            .add_to_plist(&mut shared, "macos")?;
        assert_eq!(
            shared.get("TokamakSplashImageDark"),
            Some(&Value::String("TokamakSplash.png".into()))
        );

        let mut macos = Dictionary::new();
        splash(None, None).add_to_plist(&mut macos, "macos")?;
        assert_eq!(macos.len(), 1);
        assert_eq!(
            macos.get("TokamakSplashWidth"),
            Some(&Value::Integer(120.into()))
        );
        Ok(())
    }
}
