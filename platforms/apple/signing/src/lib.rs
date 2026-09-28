#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Apple platform-pack plist generation and signing asset discovery.

mod entitlements;
mod info_plist;
pub use info_plist::write_info_plist;

#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::env;
#[cfg(any(target_os = "macos", test))]
use std::fmt::Write as _;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(any(target_os = "macos", test))]
use std::io::Cursor;
#[cfg(target_os = "macos")]
use std::io::{self, IsTerminal, Write};
use std::path::Path;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command, Output, Stdio};
#[cfg(any(target_os = "macos", test))]
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
#[cfg(any(target_os = "macos", test))]
use plist::{Dictionary, Value as PlistValue};
#[cfg(target_os = "macos")]
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "macos", test))]
use sha1::{Digest as Sha1Digest, Sha1};
#[cfg(any(target_os = "macos", test))]
use sha2::{Digest as Sha2Digest, Sha256};
/// A signing identity and provisioning profile selected for an app.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct Selection {
    identity: String,
    profile: PathBuf,
}

/// An Apple platform that apps are provisioned for.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Platform {
    Ios,
    Macos,
}

#[cfg(any(target_os = "macos", test))]
impl Platform {
    const fn application_identifier_key(self) -> &'static str {
        match self {
            Self::Ios => "application-identifier",
            Self::Macos => "com.apple.application-identifier",
        }
    }

    /// Whether a profile's `Platform` entry names this platform.
    fn is_named(self, profile_platform: &str) -> bool {
        match self {
            Self::Ios => matches!(profile_platform, "iOS" | "iPhoneOS"),
            Self::Macos => matches!(profile_platform, "OSX" | "macOS"),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Ios => "iOS",
            Self::Macos => "macOS",
        }
    }

    #[cfg(target_os = "macos")]
    const fn profile_extension(self) -> &'static str {
        match self {
            Self::Ios => "mobileprovision",
            Self::Macos => "provisionprofile",
        }
    }

    /// The provisioning profile's path inside a signed bundle.
    #[cfg(target_os = "macos")]
    const fn embedded_profile(self) -> &'static str {
        match self {
            Self::Ios => "embedded.mobileprovision",
            Self::Macos => "Contents/embedded.provisionprofile",
        }
    }

    /// The stem of the selection cache file and the saved-profile directory.
    #[cfg(target_os = "macos")]
    const fn cache_name(self) -> &'static str {
        match self {
            Self::Ios => "ios-signing",
            Self::Macos => "macos-signing",
        }
    }

    #[cfg(target_os = "macos")]
    const fn sdk(self) -> &'static str {
        match self {
            Self::Ios => "iphoneos",
            Self::Macos => "macosx",
        }
    }

    #[cfg(target_os = "macos")]
    const fn probe_project(self) -> &'static str {
        match self {
            Self::Ios => IOS_SIGNING_PROBE_PROJECT,
            Self::Macos => MACOS_SIGNING_PROBE_PROJECT,
        }
    }

    /// Entitlements that make Xcode provision a profile for the probe.
    #[cfg(target_os = "macos")]
    const fn probe_entitlements(self) -> &'static str {
        match self {
            Self::Ios => IOS_SIGNING_PROBE_ENTITLEMENTS,
            Self::Macos => MACOS_SIGNING_PROBE_ENTITLEMENTS,
        }
    }

    /// The environment variable naming the app's entitlements file.
    #[cfg(target_os = "macos")]
    const fn entitlements_variable(self) -> &'static str {
        match self {
            Self::Ios => "TOKAMAK_IOS_ENTITLEMENTS",
            Self::Macos => "TOKAMAK_MACOS_ENTITLEMENTS",
        }
    }

    /// The setting naming the app's entitlements file.
    #[cfg(target_os = "macos")]
    const fn entitlements_setting(self) -> &'static str {
        match self {
            Self::Ios => "ios.entitlements",
            Self::Macos => "macos.entitlements",
        }
    }

    #[cfg(target_os = "macos")]
    fn probe_destination(self, device_id: Option<&str>) -> String {
        match (self, device_id) {
            (Self::Ios, Some(device_id)) => format!("id={device_id}"),
            (Self::Ios, None) => "generic/platform=iOS".to_owned(),
            (Self::Macos, _) => "platform=macOS".to_owned(),
        }
    }

    /// The probe application's path inside Xcode's derived data.
    #[cfg(target_os = "macos")]
    const fn probe_product(self) -> &'static str {
        match self {
            Self::Ios => "Build/Products/Debug-iphoneos/TokamakSigningProbe.app",
            Self::Macos => "Build/Products/Debug/TokamakSigningProbe.app",
        }
    }
}

/// The app, device, team, and entitlements a signing selection is made for.
#[cfg(target_os = "macos")]
struct SigningRequest<'a> {
    platform: Platform,
    project: &'a Path,
    bundle_id: &'a str,
    device_id: Option<&'a str>,
    team_id: &'a str,
    entitlements: Option<&'a Dictionary>,
}

/// Return installed iOS signing identities and provisioning profiles.
///
/// # Errors
///
/// Returns an error when signing assets cannot be inspected or the host is not
/// macOS.
pub fn inventory() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let identities = discover_identities()?;
        signing_inventory(&identities, &discover_profiles(Platform::Ios))
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("iOS signing discovery requires a macOS host");
    }
}

/// Resolve and apply iOS signing to a completed application bundle.
///
/// # Errors
///
/// Returns an error when signing assets cannot be selected or the bundle
/// cannot be signed.
pub fn sign_ios_bundle(
    project: &Path,
    bundle: &Path,
    bundle_id: &str,
    device_id: Option<&str>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let declared = entitlements::declared(Platform::Ios.entitlements_variable())?;
        let selection = resolve_ios(project, bundle_id, device_id, declared.as_ref())?;
        sign_bundle(Platform::Ios, bundle, &selection, declared.as_ref())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (project, bundle, bundle_id, device_id);
        bail!("iOS signing requires a macOS host with Xcode");
    }
}

/// Sign a completed macOS application bundle with a development profile for
/// the team in `TOKAMAK_MACOS_TEAM_ID` that includes this Mac, or ad-hoc when
/// no team is set.
///
/// # Errors
///
/// Returns an error when signing assets cannot be selected or provisioned, an
/// ad-hoc build declares an entitlement that needs a profile, or the bundle
/// cannot be signed.
pub fn sign_macos_bundle(project: &Path, bundle: &Path, bundle_id: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let declared = entitlements::declared(Platform::Macos.entitlements_variable())?;
        let environment_team = env::var("TOKAMAK_MACOS_TEAM_ID").ok();
        match configured_team_id("TOKAMAK_MACOS_TEAM_ID", environment_team.as_deref())? {
            Some(team_id) => {
                let selection = resolve_macos(project, bundle_id, &team_id, declared.as_ref())?;
                sign_bundle(Platform::Macos, bundle, &selection, declared.as_ref())
            }
            None => sign_ad_hoc(bundle, declared.as_ref()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (project, bundle, bundle_id);
        bail!("macOS signing requires a macOS host with Xcode");
    }
}

#[cfg(target_os = "macos")]
fn signing_inventory(identities: &[Identity], profiles: &[Result<Profile>]) -> Result<String> {
    let mut output = String::from("iOS signing identities (certificate and private key):\n");
    let mut count = 0;
    for identity in identities.iter().filter(|identity| {
        [
            "Apple Development:",
            "Apple Distribution:",
            "iPhone Developer:",
            "iPhone Distribution:",
        ]
        .iter()
        .any(|prefix| identity.name.starts_with(prefix))
    }) {
        count += 1;
        writeln!(output, "\n  {}", identity.name)?;
        writeln!(output, "    SHA-1: {}", identity.selector)?;
        if let Some(team_id) = certificate_team_id(&identity.certificate_der) {
            writeln!(output, "    Team: {team_id}")?;
        }
        if let Ok((_, certificate)) = x509_parser::parse_x509_certificate(&identity.certificate_der)
        {
            let validity = certificate.validity();
            let status = if validity.is_valid() {
                ""
            } else {
                " (outside validity period)"
            };
            writeln!(output, "    Expires: {}{status}", validity.not_after)?;
        }
    }
    if count == 0 {
        writeln!(output, "  None found.")?;
    }

    writeln!(output, "\niOS provisioning profiles:")?;
    if profiles.is_empty() {
        writeln!(output, "  None found.")?;
    }
    for result in profiles {
        let profile = match result {
            Ok(profile) => profile,
            Err(error) => {
                writeln!(output, "\n  Could not read profile: {error:#}")?;
                continue;
            }
        };
        let status = if profile.expiration > SystemTime::now() {
            ""
        } else {
            " (expired)"
        };
        writeln!(output, "\n  {} ({})", profile.name, profile.id)?;
        writeln!(output, "    Path: {}", profile.path.display())?;
        writeln!(output, "    Team: {}", profile.team_id)?;
        writeln!(output, "    App ID: {}", profile.application_identifier)?;
        writeln!(output, "    Expires: {}{status}", profile.expiration_label)?;
        writeln!(output, "    Matching installed identities (SHA-1):")?;
        let matching = identities
            .iter()
            .filter(|identity| {
                profile
                    .developer_certificates
                    .contains(&identity.certificate_der)
            })
            .collect::<Vec<_>>();
        if matching.is_empty() {
            writeln!(output, "      None found.")?;
        }
        for identity in matching {
            writeln!(output, "      {}  {}", identity.selector, identity.name)?;
        }
    }
    Ok(output)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug)]
struct Profile {
    path: PathBuf,
    id: String,
    name: String,
    team_id: String,
    application_identifier: String,
    expiration: SystemTime,
    expiration_label: String,
    devices: Vec<String>,
    developer_certificates: Vec<Vec<u8>>,
    entitlements: Dictionary,
}

#[cfg(any(target_os = "macos", test))]
impl Profile {
    /// The first declared entitlement this profile does not permit.
    fn unpermitted<'a>(&self, declared: Option<&'a Dictionary>) -> Option<&'a str> {
        declared.and_then(|declared| entitlements::unpermitted(&self.entitlements, declared))
    }

    fn matches(&self, bundle_id: &str, device_id: Option<&str>, identity: &Identity) -> bool {
        let device_matches = device_id.is_none_or(|device_id| {
            self.devices
                .iter()
                .any(|device| device.eq_ignore_ascii_case(device_id))
        });
        self.expiration > SystemTime::now()
            && !self.devices.is_empty()
            && device_matches
            && app_identifier_matches(&self.application_identifier, bundle_id)
            && self
                .developer_certificates
                .iter()
                .any(|certificate| certificate == &identity.certificate_der)
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug)]
struct Identity {
    name: String,
    fingerprint: String,
    selector: String,
    certificate_der: Vec<u8>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
struct Candidate {
    identity: Identity,
    profile: Profile,
}

#[cfg(target_os = "macos")]
const IOS_SIGNING_PROBE_PROJECT: &str = include_str!("../resources/ios-signing-probe.pbxproj");
#[cfg(target_os = "macos")]
const MACOS_SIGNING_PROBE_PROJECT: &str = include_str!("../resources/macos-signing-probe.pbxproj");
#[cfg(target_os = "macos")]
const IOS_SIGNING_PROBE_ENTITLEMENTS: &str =
    include_str!("../resources/ios-signing-probe.entitlements");
#[cfg(target_os = "macos")]
const MACOS_SIGNING_PROBE_ENTITLEMENTS: &str =
    include_str!("../resources/macos-signing-probe.entitlements");
#[cfg(target_os = "macos")]
const SIGNING_PROBE_SCHEME: &str = include_str!("../resources/signing-probe.xcscheme");
#[cfg(target_os = "macos")]
const SIGNING_PROBE_SOURCE: &str = "int main(void) { return 0; }\n";

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SelectionCache {
    entries: BTreeMap<String, CachedSelection>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CachedSelection {
    identity_fingerprint: String,
    profile_id: String,
}

/// Write the entitlements an iOS simulator build embeds in its executable.
///
/// # Errors
///
/// Returns an error when the app's entitlements file cannot be read or the
/// output cannot be written.
pub fn write_simulator_entitlements(identifier: &str, output: &Path) -> Result<()> {
    let declared = entitlements::declared("TOKAMAK_IOS_ENTITLEMENTS")?;
    plist::Value::Dictionary(entitlements::for_simulator(identifier, declared.as_ref()))
        .to_file_xml(output)
        .with_context(|| format!("write simulator entitlements {}", output.display()))
}

#[cfg(target_os = "macos")]
fn resolve_ios(
    project: &Path,
    bundle_id: &str,
    device_id: Option<&str>,
    declared: Option<&Dictionary>,
) -> Result<Selection> {
    let environment_team = env::var("TOKAMAK_IOS_TEAM_ID").ok();
    let configured_team = configured_team_id("TOKAMAK_IOS_TEAM_ID", environment_team.as_deref())?;
    if let Some(selection) = explicit_selection(configured_team.is_some(), device_id)? {
        return Ok(selection);
    }

    let profiles = development_profiles(discover_profiles(Platform::Ios));
    let identities = discover_identities()?;
    let team_id = match configured_team {
        Some(team_id) => team_id,
        None => automatic_team_id(project, bundle_id, device_id, &identities, &profiles).map_err(
            |error| automatic_signing_error(Platform::Ios, bundle_id, device_id, &error),
        )?,
    };
    let request = SigningRequest {
        platform: Platform::Ios,
        project,
        bundle_id,
        device_id,
        team_id: &team_id,
        entitlements: declared,
    };
    select_signing(&request, &identities, &profiles)
}

#[cfg(target_os = "macos")]
fn resolve_macos(
    project: &Path,
    bundle_id: &str,
    team_id: &str,
    declared: Option<&Dictionary>,
) -> Result<Selection> {
    let device_id = mac_provisioning_udid()?;
    let request = SigningRequest {
        platform: Platform::Macos,
        project,
        bundle_id,
        device_id: Some(&device_id),
        team_id,
        entitlements: declared,
    };
    let profiles = development_profiles(discover_profiles(Platform::Macos));
    select_signing(&request, &discover_identities()?, &profiles)
}

/// Select an installed identity and profile for the request, asking Xcode to
/// provision one when none matches.
#[cfg(target_os = "macos")]
fn select_signing(
    request: &SigningRequest<'_>,
    identities: &[Identity],
    profiles: &[Profile],
) -> Result<Selection> {
    let SigningRequest {
        platform,
        project,
        bundle_id,
        device_id,
        team_id,
        entitlements: declared,
    } = *request;
    let mut candidates = identities
        .iter()
        .flat_map(|identity| {
            profiles
                .iter()
                .filter(move |profile| {
                    profile.team_id == team_id
                        && profile.matches(bundle_id, device_id, identity)
                        && profile.unpermitted(declared).is_none()
                })
                .map(move |profile| Candidate {
                    identity: identity.clone(),
                    profile: profile.clone(),
                })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        left.identity
            .fingerprint
            .cmp(&right.identity.fingerprint)
            .then(left.profile.id.cmp(&right.profile.id))
            .then(left.profile.path.cmp(&right.profile.path))
    });
    candidates.dedup_by(|left, right| {
        left.identity.fingerprint == right.identity.fingerprint
            && left.profile.id == right.profile.id
    });
    candidates.sort_by(|left, right| {
        left.identity
            .name
            .cmp(&right.identity.name)
            .then(left.profile.name.cmp(&right.profile.name))
            .then(left.profile.team_id.cmp(&right.profile.team_id))
            .then(left.profile.id.cmp(&right.profile.id))
    });

    if candidates.is_empty() {
        return automatic_selection(request)
            .map_err(|error| automatic_signing_error(platform, bundle_id, device_id, &error));
    }

    let key = cache_key(project, bundle_id, device_id);
    let mut cache = load_cache(platform);
    let candidate = match cache.entries.get(&key).and_then(|cached| {
        candidates.iter().find(|candidate| {
            candidate.identity.fingerprint == cached.identity_fingerprint
                && candidate.profile.id == cached.profile_id
        })
    }) {
        Some(candidate) => candidate.clone(),
        None => choose_candidate(platform, &candidates)?,
    };

    let selection = Selection {
        identity: candidate.identity.selector,
        profile: candidate.profile.path,
    };
    cache.entries.insert(
        key,
        CachedSelection {
            identity_fingerprint: candidate.identity.fingerprint,
            profile_id: candidate.profile.id,
        },
    );
    save_cache(platform, &cache);
    Ok(selection)
}

#[cfg(target_os = "macos")]
fn sign_bundle(
    platform: Platform,
    bundle: &Path,
    selection: &Selection,
    declared: Option<&Dictionary>,
) -> Result<()> {
    let name = platform.name();
    let profile = fs::read(&selection.profile).with_context(|| {
        format!(
            "read {name} provisioning profile {}",
            selection.profile.display()
        )
    })?;
    fs::write(bundle.join(platform.embedded_profile()), &profile)
        .with_context(|| format!("embed the {name} provisioning profile"))?;

    let profile = decode_profile(&profile)
        .with_context(|| format!("decode the {name} provisioning profile"))?;
    let permitted = profile
        .as_dictionary()
        .and_then(|root| root.get("Entitlements"))
        .and_then(PlistValue::as_dictionary)
        .context("provisioning profile entitlements are missing")?;
    if let Some(key) = declared.and_then(|declared| entitlements::unpermitted(permitted, declared))
    {
        bail!(
            "the provisioning profile {} does not permit the entitlement '{key}' declared in {}",
            selection.profile.display(),
            platform.entitlements_setting()
        );
    }
    codesign(
        bundle,
        &selection.identity,
        &entitlements::for_signing(permitted, declared),
    )
    .with_context(|| format!("sign {name} application bundle"))
}

/// Sign a macOS bundle without a team, with only the entitlements an ad-hoc
/// signature can carry.
#[cfg(target_os = "macos")]
fn sign_ad_hoc(bundle: &Path, declared: Option<&Dictionary>) -> Result<()> {
    let declared = declared.cloned().unwrap_or_default();
    if let Some(key) = entitlements::first_requiring_profile(&declared) {
        bail!(
            "macos.entitlements declares '{key}', which needs a provisioning profile; set macos.team-id to sign with one"
        );
    }
    let embedded = bundle.join(Platform::Macos.embedded_profile());
    if embedded.exists() {
        fs::remove_file(&embedded).context("remove the embedded provisioning profile")?;
    }
    codesign(bundle, "-", &declared).context("ad-hoc sign macOS application bundle")
}

#[cfg(target_os = "macos")]
fn codesign(bundle: &Path, identity: &str, entitlements: &Dictionary) -> Result<()> {
    let temporary = tempfile::tempdir().context("create signing directory")?;
    let entitlements_path = temporary.path().join("entitlements.plist");
    PlistValue::Dictionary(entitlements.clone())
        .to_file_xml(&entitlements_path)
        .context("write signing entitlements")?;
    let output = Command::new("codesign")
        .args([
            "--force",
            "--timestamp=none",
            "--sign",
            identity,
            "--generate-entitlement-der",
            "--entitlements",
        ])
        .arg(&entitlements_path)
        .arg(bundle)
        .output()
        .context("run codesign")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!("codesign failed: {}", command_output_detail(&output));
    }
}

#[cfg(target_os = "macos")]
fn automatic_selection(request: &SigningRequest<'_>) -> Result<Selection> {
    let SigningRequest {
        platform,
        project,
        bundle_id,
        device_id,
        team_id,
        entitlements: declared,
    } = *request;
    println!(
        "No matching {} signing profile found; asking Xcode to provision {bundle_id} automatically",
        platform.name()
    );

    let temporary = tempfile::tempdir().context("create Xcode signing probe directory")?;
    let embedded_profile = provision_signing_probe(request, temporary.path())?;
    let bytes = fs::read(&embedded_profile).with_context(|| {
        format!(
            "Xcode did not produce a provisioning profile at {}",
            embedded_profile.display()
        )
    })?;
    let profile = parse_profile(platform, &embedded_profile, &bytes)
        .context("parse the provisioning profile generated by Xcode")?;
    if let Some(key) = profile.unpermitted(declared) {
        bail!(
            "Xcode generated a profile that does not permit the entitlement '{key}' declared in {}",
            platform.entitlements_setting()
        );
    }
    let identities = discover_identities()?;
    let identity = identities
        .iter()
        .find(|identity| {
            profile.team_id == team_id && profile.matches(bundle_id, device_id, identity)
        })
        .context(
            "Xcode generated a profile that does not match an installed Apple Development identity",
        )?;
    let profile_path = persist_profile(platform, &profile.id, &bytes)?;
    let key = cache_key(project, bundle_id, device_id);
    let mut cache = load_cache(platform);
    cache.entries.insert(
        key,
        CachedSelection {
            identity_fingerprint: identity.fingerprint.clone(),
            profile_id: profile.id,
        },
    );
    save_cache(platform, &cache);
    Ok(Selection {
        identity: identity.selector.clone(),
        profile: profile_path,
    })
}

/// Build the signing probe in `root` with Xcode's automatic provisioning, and
/// return the path of the profile Xcode embeds in it.
#[cfg(target_os = "macos")]
fn provision_signing_probe(request: &SigningRequest<'_>, root: &Path) -> Result<PathBuf> {
    let platform = request.platform;
    let project_path =
        write_signing_probe(platform, root, request.bundle_id, request.entitlements)?;
    let derived_data = root.join("DerivedData");
    let mut command = Command::new("xcodebuild");
    command
        .current_dir(root)
        .arg("-project")
        .arg(&project_path)
        .args([
            "-scheme",
            "TokamakSigningProbe",
            "-configuration",
            "Debug",
            "-sdk",
            platform.sdk(),
        ])
        .arg("-destination")
        .arg(platform.probe_destination(request.device_id))
        .arg("-derivedDataPath")
        .arg(&derived_data)
        .arg("-allowProvisioningUpdates");
    if request.device_id.is_some() {
        command.arg("-allowProvisioningDeviceRegistration");
    }
    let output = command
        .args([
            "CODE_SIGN_STYLE=Automatic",
            "CODE_SIGN_IDENTITY=Apple Development",
        ])
        .arg(format!("DEVELOPMENT_TEAM={}", request.team_id))
        .arg("build")
        .stdin(Stdio::inherit())
        .output()
        .with_context(|| {
            format!(
                "run xcodebuild for automatic {} provisioning",
                platform.name()
            )
        })?;
    if !output.status.success() {
        bail!("xcodebuild failed: {}", command_output_detail(&output));
    }
    Ok(derived_data
        .join(platform.probe_product())
        .join(platform.embedded_profile()))
}

#[cfg(target_os = "macos")]
fn automatic_team_id(
    project: &Path,
    bundle_id: &str,
    device_id: Option<&str>,
    identities: &[Identity],
    profiles: &[Profile],
) -> Result<String> {
    if let Some(cached) = load_cache(Platform::Ios)
        .entries
        .get(&cache_key(project, bundle_id, device_id))
        && let Some(identity) = identities
            .iter()
            .find(|identity| identity.fingerprint == cached.identity_fingerprint)
        && let Some(team_id) = identity_team_id(identity, profiles)
    {
        return Ok(team_id);
    }

    let mut teams = BTreeMap::<String, u8>::new();
    for identity in identities
        .iter()
        .filter(|identity| identity.name.starts_with("Apple Development:"))
    {
        let Some(team_id) = identity_team_id(identity, profiles) else {
            continue;
        };
        let score = profiles
            .iter()
            .filter(|profile| {
                profile
                    .developer_certificates
                    .iter()
                    .any(|certificate| certificate == &identity.certificate_der)
            })
            .map(|profile| profile_relevance(profile, bundle_id))
            .max()
            .unwrap_or_default();
        teams
            .entry(team_id)
            .and_modify(|best| *best = (*best).max(score))
            .or_insert(score);
    }

    let Some(best_score) = teams.values().copied().max() else {
        bail!(
            "no Apple Development identity with a discoverable team was found; create one in Xcode"
        );
    };
    let best = teams
        .iter()
        .filter_map(|(team_id, score)| (*score == best_score).then_some(team_id))
        .collect::<Vec<_>>();
    if best.len() != 1 {
        let available = teams
            .keys()
            .map(|team_id| format!("  {}", team_label(team_id, identities, profiles)))
            .collect::<Vec<_>>()
            .join("\n");
        let command = team_command(device_id);
        let manual_command = manual_command(device_id);
        bail!(
            "Multiple Apple Development teams are available:\n\n  Team ID     Name\n{available}\n\n\
             Choose the team that owns this app. Replace TEAM_ID below with its ID.\n\n  \
             Set it for this shell, then rerun your command:\n    \
             export TOKAMAK_IOS_TEAM_ID=TEAM_ID\n\n  \
             Or select it for a single command:\n    {command}\n\n\
             Tokamak will select the signing identity and provisioning profile automatically.\n\n\
             Alternatively, use manual signing:\n\n  \
             First, list installed identities and provisioning profiles:\n    \
             tok certs\n\n  \
             Both values are required and must belong together.\n\n  \
             Set them for this shell:\n    \
             export TOKAMAK_IOS_SIGNING_IDENTITY=\"IDENTITY_SHA1\"\n    \
             export TOKAMAK_IOS_PROVISIONING_PROFILE=\"/path/to/profile.mobileprovision\"\n\n  \
             Or set them for a single command:\n    {manual_command}"
        );
    }
    Ok(best[0].clone())
}

#[cfg(target_os = "macos")]
fn identity_team_id(identity: &Identity, profiles: &[Profile]) -> Option<String> {
    profiles
        .iter()
        .find(|profile| {
            profile
                .developer_certificates
                .iter()
                .any(|certificate| certificate == &identity.certificate_der)
        })
        .map(|profile| profile.team_id.clone())
        .or_else(|| certificate_team_id(&identity.certificate_der))
}

#[cfg(target_os = "macos")]
fn certificate_team_id(certificate_der: &[u8]) -> Option<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate_der).ok()?;
    certificate
        .subject()
        .iter_organizational_unit()
        .find_map(|attribute| attribute.as_str().ok().map(str::to_owned))
}

#[cfg(target_os = "macos")]
fn team_label(team_id: &str, identities: &[Identity], profiles: &[Profile]) -> String {
    let mut fallback = format!("{team_id}  (team name unavailable)");
    for identity in identities.iter().filter(|identity| {
        identity.name.starts_with("Apple Development:")
            && identity_team_id(identity, profiles).as_deref() == Some(team_id)
    }) {
        fallback = format!(
            "{team_id}  (team name unavailable; signing identity: {})",
            identity.name
        );
        let Ok((_, certificate)) = x509_parser::parse_x509_certificate(&identity.certificate_der)
        else {
            continue;
        };
        if let Some(name) = certificate
            .subject()
            .iter_organization()
            .find_map(|attribute| {
                attribute
                    .as_str()
                    .ok()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
            })
        {
            return format!("{team_id}  {name}");
        }
    }
    fallback
}

#[cfg(target_os = "macos")]
fn profile_relevance(profile: &Profile, bundle_id: &str) -> u8 {
    let Some((_, pattern)) = profile.application_identifier.split_once('.') else {
        return 0;
    };
    if pattern == bundle_id {
        3
    } else if pattern.starts_with("com.tokamak.") {
        2
    } else {
        1
    }
}

/// Write the probe project, declaring the app's entitlements so Xcode enables
/// the matching capabilities on the App ID.
#[cfg(target_os = "macos")]
fn write_signing_probe(
    platform: Platform,
    root: &Path,
    bundle_id: &str,
    declared: Option<&Dictionary>,
) -> Result<PathBuf> {
    let project = root.join("TokamakSigningProbe.xcodeproj");
    let schemes = project.join("xcshareddata/xcschemes");
    fs::create_dir_all(&schemes).context("create Xcode signing probe project")?;
    fs::write(
        project.join("project.pbxproj"),
        platform
            .probe_project()
            .replace("__TOKAMAK_BUNDLE_ID__", &pbx_escape(bundle_id)),
    )?;
    fs::write(root.join("main.m"), SIGNING_PROBE_SOURCE)?;
    fs::write(
        schemes.join("TokamakSigningProbe.xcscheme"),
        SIGNING_PROBE_SCHEME,
    )?;
    let mut entitlements = PlistValue::from_reader_xml(platform.probe_entitlements().as_bytes())?
        .into_dictionary()
        .context("probe entitlements must be a dictionary")?;
    if let Some(declared) = declared {
        info_plist::overlay_dictionary(&mut entitlements, declared.clone());
    }
    PlistValue::Dictionary(entitlements)
        .to_file_xml(root.join("TokamakSigningProbe.entitlements"))?;
    Ok(project)
}

#[cfg(target_os = "macos")]
fn pbx_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "macos")]
fn command_output_detail(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if detail.is_empty() {
        output.status.code().map_or_else(
            || "terminated by signal".to_owned(),
            |code| format!("exit code {code}"),
        )
    } else {
        detail
    }
}

#[cfg(target_os = "macos")]
fn automatic_signing_error(
    platform: Platform,
    bundle_id: &str,
    device_id: Option<&str>,
    error: &anyhow::Error,
) -> anyhow::Error {
    let name = platform.name();
    let target = device_id.map_or_else(
        || format!("a generic {name} device"),
        |device_id| format!("{name} device {device_id}"),
    );
    anyhow::anyhow!(
        "automatic {name} signing failed for bundle {bundle_id} and {target}:\n\n{error:#}"
    )
}

#[cfg(target_os = "macos")]
fn team_command(device_id: Option<&str>) -> String {
    device_id.map_or_else(
        || "tok build ios --ios-team-id TEAM_ID".to_owned(),
        |device_id| format!("tok dev {device_id} --ios-team-id TEAM_ID -- <dev-command>"),
    )
}

#[cfg(target_os = "macos")]
fn manual_command(device_id: Option<&str>) -> String {
    device_id.map_or_else(
        || {
            "tok build ios --ios-signing-identity IDENTITY_SHA1 \\\n    --ios-provisioning-profile /path/to/profile.mobileprovision"
                .to_owned()
        },
        |device_id| {
            format!(
                "tok dev {device_id} \\\n    --ios-signing-identity IDENTITY_SHA1 \\\n    --ios-provisioning-profile /path/to/profile.mobileprovision \\\n    -- <dev-command>"
            )
        },
    )
}

#[cfg(target_os = "macos")]
fn explicit_selection(team_configured: bool, device_id: Option<&str>) -> Result<Option<Selection>> {
    let identity = env::var("TOKAMAK_IOS_SIGNING_IDENTITY").ok();
    let profile = env::var_os("TOKAMAK_IOS_PROVISIONING_PROFILE").map(PathBuf::from);
    match (identity, profile) {
        (Some(identity), Some(profile)) => {
            if team_configured {
                bail!(
                    "automatic and manual iOS signing cannot be combined.\n\n\
                     Choose ONE signing mode:\n  \
                     Automatic: set TOKAMAK_IOS_TEAM_ID or use --ios-team-id TEAM_ID\n  \
                     Manual:    set BOTH TOKAMAK_IOS_SIGNING_IDENTITY and\n             \
                     TOKAMAK_IOS_PROVISIONING_PROFILE, or use the matching options\n\n  \
                     Automatic and manual signing cannot be combined for one command.\n\n  \
                     Automatic example:\n    \
                     {}\n\n  \
                     Manual example:\n    \
                     {}",
                    team_command(device_id),
                    manual_command(device_id)
                );
            }
            if identity.trim().is_empty() {
                bail!("TOKAMAK_IOS_SIGNING_IDENTITY must not be empty");
            }
            if !profile.is_file() {
                bail!(
                    "TOKAMAK_IOS_PROVISIONING_PROFILE does not point to a file: {}",
                    profile.display()
                );
            }
            let profile =
                fs::canonicalize(profile).context("resolve TOKAMAK_IOS_PROVISIONING_PROFILE")?;
            Ok(Some(Selection { identity, profile }))
        }
        (None, None) => Ok(None),
        _ => bail!(
            "TOKAMAK_IOS_SIGNING_IDENTITY and TOKAMAK_IOS_PROVISIONING_PROFILE must be provided together"
        ),
    }
}

#[cfg(any(target_os = "macos", test))]
fn configured_team_id(variable: &str, environment: Option<&str>) -> Result<Option<String>> {
    let Some(team_id) = environment else {
        return Ok(None);
    };
    let team_id = team_id.trim();
    if team_id.is_empty() {
        bail!("{variable} must not be empty");
    }
    Ok(Some(team_id.to_owned()))
}

#[cfg(target_os = "macos")]
fn mac_provisioning_udid() -> Result<String> {
    let output = Command::new("system_profiler")
        .args(["-json", "SPHardwareDataType"])
        .output()
        .context("run system_profiler to identify this Mac")?;
    if !output.status.success() {
        bail!("system_profiler failed: {}", command_output_detail(&output));
    }
    provisioning_udid(&output.stdout)
}

/// The provisioning UDID in a `system_profiler -json SPHardwareDataType` report.
#[cfg(any(target_os = "macos", test))]
fn provisioning_udid(hardware_report: &[u8]) -> Result<String> {
    let report: serde_json::Value = serde_json::from_slice(hardware_report)
        .context("parse the system_profiler hardware report")?;
    report["SPHardwareDataType"][0]["provisioning_UDID"]
        .as_str()
        .map(str::to_owned)
        .context("the system_profiler hardware report has no provisioning UDID")
}

#[cfg(target_os = "macos")]
fn discover_identities() -> Result<Vec<Identity>> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};

    let mut options = ItemSearchOptions::new();
    options
        .class(ItemClass::identity())
        .load_refs(true)
        .limit(Limit::All);
    let results = match options.search() {
        Ok(results) => results,
        Err(error) if error.code() == -25300 => Vec::new(), // errSecItemNotFound
        Err(error) => bail!("search macOS signing identities: {error}"),
    };

    let mut identities = results
        .into_iter()
        .filter_map(|result| {
            let SearchResult::Ref(Reference::Identity(identity)) = result else {
                return None;
            };
            let certificate = identity.certificate().ok()?;
            identity.private_key().ok()?;
            let certificate_der = certificate.to_der();
            Some(Identity {
                name: certificate.subject_summary(),
                fingerprint: fingerprint(&certificate_der),
                selector: sha1_fingerprint(&certificate_der),
                certificate_der,
            })
        })
        .collect::<Vec<_>>();
    identities.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.fingerprint.cmp(&right.fingerprint))
    });
    identities.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    Ok(identities)
}

#[cfg(target_os = "macos")]
fn development_profiles(profiles: Vec<Result<Profile>>) -> Vec<Profile> {
    profiles
        .into_iter()
        .filter_map(Result::ok)
        .filter(|profile| !profile.devices.is_empty())
        .collect()
}

#[cfg(target_os = "macos")]
fn discover_profiles(platform: Platform) -> Vec<Result<Profile>> {
    let mut paths = profile_paths(platform);
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| parse_profile(platform, &path, &bytes))
                .with_context(|| path.display().to_string())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn profile_paths(platform: Platform) -> Vec<PathBuf> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut directories = vec![
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ];
    if let Some(directory) = profile_cache_directory(platform) {
        directories.push(directory);
    }
    directories
        .into_iter()
        .flat_map(|directory| {
            let Ok(entries) = fs::read_dir(directory) else {
                return Vec::new();
            };
            entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == platform.profile_extension())
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn persist_profile(platform: Platform, profile_id: &str, bytes: &[u8]) -> Result<PathBuf> {
    let name = platform.name();
    let directory = profile_cache_directory(platform)
        .with_context(|| format!("resolve tokamak {name} signing cache"))?;
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "create tokamak {name} signing profile cache: {}",
            directory.display()
        )
    })?;
    let path = directory.join(format!("{profile_id}.{}", platform.profile_extension()));
    fs::write(&path, bytes)
        .with_context(|| format!("save Xcode provisioning profile: {}", path.display()))?;
    fs::canonicalize(path).context("resolve saved Xcode provisioning profile")
}

#[cfg(any(target_os = "macos", test))]
fn parse_profile(platform: Platform, path: &Path, bytes: &[u8]) -> Result<Profile> {
    let value = decode_profile(bytes)?;
    let root = value
        .as_dictionary()
        .context("provisioning profile root is not a dictionary")?;
    let entitlements = root
        .get("Entitlements")
        .and_then(PlistValue::as_dictionary)
        .context("provisioning profile entitlements are missing")?;
    let application_identifier = string_value(entitlements, platform.application_identifier_key())
        .context("provisioning profile application identifier is missing")?;
    let team_id = string_array(root, "TeamIdentifier")
        .into_iter()
        .next()
        .or_else(|| application_identifier.split('.').next().map(str::to_owned))
        .context("provisioning profile team identifier is missing")?;
    let expiration_date = root
        .get("ExpirationDate")
        .and_then(PlistValue::as_date)
        .context("provisioning profile expiration date is missing")?;
    let expiration = SystemTime::from(expiration_date);
    let expiration_label = expiration_date.to_xml_format();
    let devices = string_array(root, "ProvisionedDevices");
    let developer_certificates = data_array(root, "DeveloperCertificates");
    if developer_certificates.is_empty() {
        bail!("provisioning profile does not contain developer certificates");
    }
    let platforms = string_array(root, "Platform");
    if !platforms.is_empty() && !platforms.iter().any(|name| platform.is_named(name)) {
        bail!("provisioning profile is not for {}", platform.name());
    }

    let id = string_value(root, "UUID").unwrap_or_else(|| fingerprint(bytes));
    let name = string_value(root, "Name").unwrap_or_else(|| id.clone());
    Ok(Profile {
        path: path.to_path_buf(),
        id,
        name,
        team_id,
        application_identifier,
        expiration,
        expiration_label,
        devices,
        developer_certificates,
        entitlements: entitlements.clone(),
    })
}

#[cfg(any(target_os = "macos", test))]
fn decode_profile(bytes: &[u8]) -> Result<PlistValue> {
    if let Ok(value) = PlistValue::from_reader(Cursor::new(bytes)) {
        return Ok(value);
    }

    #[cfg(target_os = "macos")]
    {
        use security_framework::cms::CMSDecoder;

        let decoder = CMSDecoder::create()
            .map_err(|error| anyhow::anyhow!("create provisioning-profile decoder: {error}"))?;
        decoder
            .update_message(bytes)
            .map_err(|error| anyhow::anyhow!("decode provisioning profile: {error}"))?;
        decoder
            .finalize_message()
            .map_err(|error| anyhow::anyhow!("finalize provisioning profile: {error}"))?;
        let content = decoder
            .get_content()
            .map_err(|error| anyhow::anyhow!("read provisioning-profile contents: {error}"))?;
        PlistValue::from_reader(Cursor::new(content))
            .context("provisioning profile contents are not a plist")
    }

    #[cfg(not(target_os = "macos"))]
    {
        bail!("CMS provisioning profiles can only be decoded on macOS");
    }
}

#[cfg(any(target_os = "macos", test))]
fn string_value(values: &Dictionary, key: &str) -> Option<String> {
    values.get(key)?.as_string().map(str::to_owned)
}

#[cfg(any(target_os = "macos", test))]
fn string_array(values: &Dictionary, key: &str) -> Vec<String> {
    values
        .get(key)
        .and_then(PlistValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(PlistValue::as_string)
        .map(str::to_owned)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn data_array(values: &Dictionary, key: &str) -> Vec<Vec<u8>> {
    values
        .get(key)
        .and_then(PlistValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(PlistValue::as_data)
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn app_identifier_matches(application_identifier: &str, bundle_id: &str) -> bool {
    let Some((_, pattern)) = application_identifier.split_once('.') else {
        return false;
    };
    pattern == bundle_id
        || pattern == "*"
        || pattern.strip_suffix(".*").is_some_and(|prefix| {
            bundle_id.starts_with(prefix) && bundle_id.as_bytes().get(prefix.len()) == Some(&b'.')
        })
}

#[cfg(any(target_os = "macos", test))]
fn fingerprint(bytes: &[u8]) -> String {
    hex_digest(<Sha256 as Sha2Digest>::digest(bytes))
}

#[cfg(any(target_os = "macos", test))]
fn sha1_fingerprint(bytes: &[u8]) -> String {
    hex_digest(<Sha1 as Sha1Digest>::digest(bytes))
}

#[cfg(any(target_os = "macos", test))]
fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(target_os = "macos")]
fn cache_key(project: &Path, bundle_id: &str, device_id: Option<&str>) -> String {
    let target = device_id.unwrap_or("<generic>");
    format!("{}\n{bundle_id}\n{target}", project.display())
}

#[cfg(target_os = "macos")]
fn tokamak_support_directory() -> Option<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join("Library/Application Support/tokamak"))
}

#[cfg(target_os = "macos")]
fn cache_path(platform: Platform) -> Option<PathBuf> {
    Some(tokamak_support_directory()?.join(format!("{}.json", platform.cache_name())))
}

#[cfg(target_os = "macos")]
fn profile_cache_directory(platform: Platform) -> Option<PathBuf> {
    Some(tokamak_support_directory()?.join(format!("{}-profiles", platform.cache_name())))
}

#[cfg(target_os = "macos")]
fn load_cache(platform: Platform) -> SelectionCache {
    let Some(path) = cache_path(platform) else {
        return SelectionCache::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn save_cache(platform: Platform, cache: &SelectionCache) {
    let Some(path) = cache_path(platform) else {
        return;
    };
    let Some(parent) = path.parent().map(Path::to_path_buf) else {
        return;
    };
    if let Err(error) = (|| -> Result<()> {
        fs::create_dir_all(&parent)?;
        fs::write(path, serde_json::to_vec_pretty(cache)?)?;
        Ok(())
    })() {
        eprintln!(
            "warning: could not save {} signing selection: {error}",
            platform.name()
        );
    }
}

#[cfg(target_os = "macos")]
fn choose_candidate(platform: Platform, candidates: &[Candidate]) -> Result<Candidate> {
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    let name = platform.name();
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let remedy = match platform {
            Platform::Ios => {
                "set TOKAMAK_IOS_SIGNING_IDENTITY and TOKAMAK_IOS_PROVISIONING_PROFILE for non-interactive use"
            }
            Platform::Macos => "run the build in a terminal to choose one",
        };
        bail!("multiple valid {name} signing profiles match this app and device; {remedy}");
    }

    println!("Multiple valid {name} signing profiles match this app and device:");
    for (index, candidate) in candidates.iter().enumerate() {
        println!(
            "  {}) {} (SHA-1 {}) — {} [{}] (team {}, expires {})",
            index + 1,
            candidate.identity.name,
            candidate.identity.selector,
            candidate.profile.name,
            candidate.profile.id,
            candidate.profile.team_id,
            candidate.profile.expiration_label
        );
    }
    loop {
        print!("Select a profile [1-{}]: ", candidates.len());
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            bail!("no {name} signing profile was selected");
        }
        if let Ok(number) = input.trim().parse::<usize>()
            && (1..=candidates.len()).contains(&number)
        {
            return Ok(candidates[number - 1].clone());
        }
        eprintln!("Enter a number from 1 to {}.", candidates.len());
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use anyhow::Context;
    use plist::{Dictionary, Value};

    use super::{
        Identity, Platform, app_identifier_matches, configured_team_id, fingerprint, parse_profile,
        provisioning_udid, sha1_fingerprint,
    };

    #[test]
    fn configured_team_id_reads_the_environment() -> anyhow::Result<()> {
        assert_eq!(
            configured_team_id("TOKAMAK_IOS_TEAM_ID", Some(" ENV "))?.as_deref(),
            Some("ENV")
        );
        assert_eq!(configured_team_id("TOKAMAK_IOS_TEAM_ID", None)?, None);
        Ok(())
    }

    #[test]
    fn empty_team_selection_reports_the_environment() -> anyhow::Result<()> {
        let error = configured_team_id("TOKAMAK_IOS_TEAM_ID", Some(" "))
            .err()
            .context("expected an empty-team error")?;
        assert_eq!(error.to_string(), "TOKAMAK_IOS_TEAM_ID must not be empty");
        let error = configured_team_id("TOKAMAK_MACOS_TEAM_ID", Some(""))
            .err()
            .context("expected an empty-team error")?;
        assert_eq!(error.to_string(), "TOKAMAK_MACOS_TEAM_ID must not be empty");
        Ok(())
    }

    #[test]
    fn reads_the_provisioning_udid_from_the_hardware_report() -> anyhow::Result<()> {
        let report = br#"{"SPHardwareDataType":[{"platform_UUID":"HARDWARE","provisioning_UDID":"00006001-001C59300A02801E"}]}"#;
        assert_eq!(provisioning_udid(report)?, "00006001-001C59300A02801E");
        assert!(provisioning_udid(br#"{"SPHardwareDataType":[{}]}"#).is_err());
        assert!(provisioning_udid(b"not json").is_err());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    use super::{
        Profile, automatic_signing_error, automatic_team_id, pbx_escape, write_signing_probe,
    };

    #[cfg(target_os = "macos")]
    fn development_identity(team_id: &str, team_name: Option<&str>) -> anyhow::Result<Identity> {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

        let mut subject = DistinguishedName::new();
        subject.push(DnType::CommonName, "Apple Development: Same Developer");
        subject.push(DnType::OrganizationalUnitName, team_id);
        if let Some(name) = team_name {
            subject.push(DnType::OrganizationName, name);
        }
        let mut params = CertificateParams::default();
        params.distinguished_name = subject;
        let key = KeyPair::generate()?;
        let certificate = params.self_signed(&key)?;
        let certificate_der = certificate.der().to_vec();
        Ok(Identity {
            name: "Apple Development: Same Developer".to_owned(),
            fingerprint: fingerprint(&certificate_der),
            selector: sha1_fingerprint(&certificate_der),
            certificate_der,
        })
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signing_inventory_reports_empty_results() -> anyhow::Result<()> {
        assert_eq!(
            super::signing_inventory(&[], &[])?,
            "iOS signing identities (certificate and private key):\n  None found.\n\niOS provisioning profiles:\n  None found.\n"
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signing_inventory_lists_profile_metadata_and_matching_identity_selectors()
    -> anyhow::Result<()> {
        let development = development_identity("TEAM", Some("Example Company"))?;
        let mut distribution = development_identity("TEAM", Some("Example Company"))?;
        distribution.name = "Apple Distribution: Example Company".to_owned();
        let mut unrelated = development_identity("OTHER", None)?;
        unrelated.name = "Unrelated TLS identity".to_owned();
        let mut value = profile_value("TEAM.com.example.app", "DEVICE");
        let root = value.as_dictionary_mut().context("profile dictionary")?;
        root.remove("ProvisionedDevices");
        root.insert("Name".to_owned(), Value::String("App Store".to_owned()));
        root.insert(
            "DeveloperCertificates".to_owned(),
            Value::Array(vec![Value::Data(distribution.certificate_der.clone())]),
        );
        root.insert(
            "ExpirationDate".to_owned(),
            Value::Date(SystemTime::UNIX_EPOCH.into()),
        );
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;
        let profile = parse_profile(
            Platform::Ios,
            Path::new("/profiles/app.mobileprovision"),
            &bytes,
        )?;
        let identities = [development, distribution, unrelated];
        let output = super::signing_inventory(&identities, &[Ok(profile)])?;
        let (identity_output, profile_output) = output
            .split_once("iOS provisioning profiles:")
            .context("profile heading")?;
        for identity in &identities[..2] {
            assert!(identity_output.contains(&format!("SHA-1: {}", identity.selector)));
        }
        assert!(!identity_output.contains("Unrelated TLS identity"));
        assert!(identity_output.contains("Team: TEAM"));
        assert!(identity_output.contains("Expires:"));
        assert!(profile_output.contains("App Store (PROFILE-1)"));
        assert!(profile_output.contains("Path: /profiles/app.mobileprovision"));
        assert!(profile_output.contains("Team: TEAM"));
        assert!(profile_output.contains("App ID: TEAM.com.example.app"));
        assert!(profile_output.contains("Expires: 1970-01-01T00:00:00Z (expired)"));
        assert!(profile_output.contains(&format!(
            "{}  Apple Distribution: Example Company",
            identities[1].selector
        )));
        assert!(!profile_output.contains(&identities[0].selector));
        assert!(!profile_output.contains(&identities[2].selector));
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signing_inventory_reports_unmatched_and_unreadable_profiles() -> anyhow::Result<()> {
        let mut bytes = Vec::new();
        profile_value("TEAM.com.example.app", "DEVICE").to_writer_xml(&mut bytes)?;
        let profile = parse_profile(
            Platform::Ios,
            Path::new("/profiles/dev.mobileprovision"),
            &bytes,
        )?;
        let error = anyhow::anyhow!("invalid profile").context("/profiles/broken.mobileprovision");
        let output = super::signing_inventory(&[], &[Ok(profile), Err(error)])?;
        assert!(output.contains("Matching installed identities (SHA-1):\n      None found."));
        assert!(
            output.contains(
                "Could not read profile: /profiles/broken.mobileprovision: invalid profile"
            )
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ambiguous_teams_show_names_and_selection_examples() -> anyhow::Result<()> {
        let identities = [
            development_identity("TEAMBBBBBB", Some("Example Company Ltd"))?,
            development_identity("TEAMAAAAAA", Some("Example Person"))?,
            development_identity("TEAMBBBBBB", Some("Example Company Ltd"))?,
        ];
        let project = tempfile::tempdir()?;
        let error = automatic_team_id(
            project.path(),
            "com.example.app",
            Some("DEVICE"),
            &identities,
            &[],
        )
        .err()
        .context("a team must be selected")?;
        let error =
            automatic_signing_error(Platform::Ios, "com.example.app", Some("DEVICE"), &error);
        assert_eq!(
            error.to_string(),
            concat!(
                "automatic iOS signing failed for bundle com.example.app and iOS device DEVICE:\n\n",
                "Multiple Apple Development teams are available:\n\n",
                "  Team ID     Name\n",
                "  TEAMAAAAAA  Example Person\n",
                "  TEAMBBBBBB  Example Company Ltd\n\n",
                "Choose the team that owns this app. Replace TEAM_ID below with its ID.\n\n",
                "  Set it for this shell, then rerun your command:\n",
                "    export TOKAMAK_IOS_TEAM_ID=TEAM_ID\n\n",
                "  Or select it for a single command:\n",
                "    tok dev DEVICE --ios-team-id TEAM_ID -- <dev-command>\n\n",
                "Tokamak will select the signing identity and provisioning profile automatically.\n\n",
                "Alternatively, use manual signing:\n\n",
                "  First, list installed identities and provisioning profiles:\n",
                "    tok certs\n\n",
                "  Both values are required and must belong together.\n\n",
                "  Set them for this shell:\n",
                "    export TOKAMAK_IOS_SIGNING_IDENTITY=\"IDENTITY_SHA1\"\n",
                "    export TOKAMAK_IOS_PROVISIONING_PROFILE=\"/path/to/profile.mobileprovision\"\n\n",
                "  Or set them for a single command:\n",
                "    tok dev DEVICE \\\n    --ios-signing-identity IDENTITY_SHA1 \\\n    --ios-provisioning-profile /path/to/profile.mobileprovision \\\n    -- <dev-command>"
            )
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ambiguous_teams_include_personal_teams_without_matching_profiles() -> anyhow::Result<()> {
        let identities = [
            development_identity("TEAMAAAAAA", Some("First Company"))?,
            development_identity("TEAMBBBBBB", Some("Second Company"))?,
            development_identity("TEAMCCCCCC", Some("Personal Developer"))?,
        ];
        let mut profiles = Vec::new();
        for (team_id, identity) in ["TEAMAAAAAA", "TEAMBBBBBB"].iter().zip(&identities) {
            let mut value = profile_value(&format!("{team_id}.com.example.app"), "DEVICE");
            let root = value
                .as_dictionary_mut()
                .context("profile fixture is a dictionary")?;
            root.insert(
                "TeamIdentifier".to_owned(),
                Value::Array(vec![Value::String((*team_id).to_owned())]),
            );
            root.insert(
                "DeveloperCertificates".to_owned(),
                Value::Array(vec![Value::Data(identity.certificate_der.clone())]),
            );
            let mut bytes = Vec::new();
            value.to_writer_xml(&mut bytes)?;
            profiles.push(parse_profile(
                Platform::Ios,
                Path::new("profile.mobileprovision"),
                &bytes,
            )?);
        }
        let project = tempfile::tempdir()?;
        for device_id in [Some("DEVICE"), None] {
            let error = automatic_team_id(
                project.path(),
                "com.example.app",
                device_id,
                &identities,
                &profiles,
            )
            .err()
            .context("matching company teams are ambiguous")?
            .to_string();
            for team in [
                "TEAMAAAAAA  First Company",
                "TEAMBBBBBB  Second Company",
                "TEAMCCCCCC  Personal Developer",
            ] {
                assert!(error.contains(team), "missing {team}: {error}");
            }
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unnamed_teams_show_the_identity_without_claiming_it_is_the_team_name() -> anyhow::Result<()>
    {
        let identities = [
            development_identity("TEAMAAAAAA", None)?,
            development_identity("TEAMBBBBBB", Some("  "))?,
        ];
        let project = tempfile::tempdir()?;
        let error = automatic_team_id(project.path(), "com.example.app", None, &identities, &[])
            .err()
            .context("a team must be selected")?
            .to_string();
        for team in ["TEAMAAAAAA", "TEAMBBBBBB"] {
            assert!(error.contains(&format!("{team}  (team name unavailable; signing identity: Apple Development: Same Developer)")));
        }
        assert!(error.contains("tok build ios --ios-team-id TEAM_ID"));
        assert!(!error.contains("tok dev"));
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_signing_errors_preserve_details_without_unrelated_manual_instructions() {
        let cause =
            anyhow::anyhow!("No devices are registered").context("Xcode provisioning failed");
        let error = automatic_signing_error(Platform::Ios, "com.example.app", None, &cause);
        assert_eq!(
            error.to_string(),
            concat!(
                "automatic iOS signing failed for bundle com.example.app and a generic iOS device:\n\n",
                "Xcode provisioning failed: No devices are registered"
            )
        );
    }

    fn profile_value(application_identifier: &str, device: &str) -> Value {
        let mut entitlements = Dictionary::new();
        entitlements.insert(
            "application-identifier".to_owned(),
            Value::String(application_identifier.to_owned()),
        );
        let mut root = Dictionary::new();
        root.insert("UUID".to_owned(), Value::String("PROFILE-1".to_owned()));
        root.insert("Name".to_owned(), Value::String("Development".to_owned()));
        root.insert(
            "TeamIdentifier".to_owned(),
            Value::Array(vec![Value::String("TEAM".to_owned())]),
        );
        root.insert(
            "ExpirationDate".to_owned(),
            Value::Date((SystemTime::now() + Duration::from_hours(1)).into()),
        );
        root.insert(
            "ProvisionedDevices".to_owned(),
            Value::Array(vec![Value::String(device.to_owned())]),
        );
        root.insert(
            "DeveloperCertificates".to_owned(),
            Value::Array(vec![Value::Data(vec![1, 2, 3])]),
        );
        root.insert(
            "Platform".to_owned(),
            Value::Array(vec![Value::String("iOS".to_owned())]),
        );
        root.insert("Entitlements".to_owned(), Value::Dictionary(entitlements));
        Value::Dictionary(root)
    }

    #[test]
    fn parses_and_matches_a_development_profile() -> Result<(), Box<dyn std::error::Error>> {
        let value = profile_value("TEAM.com.example.app", "DEVICE");
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;
        let profile = parse_profile(
            Platform::Ios,
            Path::new("PROFILE-1.mobileprovision"),
            &bytes,
        )?;
        let identity = Identity {
            name: "Apple Development: Test".to_owned(),
            fingerprint: fingerprint(&[1, 2, 3]),
            selector: sha1_fingerprint(&[1, 2, 3]),
            certificate_der: vec![1, 2, 3],
        };

        assert_eq!(profile.path, Path::new("PROFILE-1.mobileprovision"));
        assert_eq!(profile.id, "PROFILE-1");
        assert_eq!(profile.name, "Development");
        assert_eq!(profile.team_id, "TEAM");
        assert!(!profile.expiration_label.is_empty());
        assert_eq!(identity.name, "Apple Development: Test");
        assert_eq!(identity.fingerprint, fingerprint(&[1, 2, 3]));
        assert_eq!(identity.selector, sha1_fingerprint(&[1, 2, 3]));
        assert!(profile.matches("com.example.app", Some("DEVICE"), &identity));
        assert!(!profile.matches("com.example.other", Some("DEVICE"), &identity));
        assert!(!profile.matches("com.example.app", Some("OTHER"), &identity));
        assert!(profile.matches("com.example.app", None, &identity));
        Ok(())
    }

    #[test]
    fn names_a_declared_entitlement_the_profile_does_not_permit() -> anyhow::Result<()> {
        let mut bytes = Vec::new();
        profile_value("TEAM.com.example.app", "DEVICE").to_writer_xml(&mut bytes)?;
        let profile = parse_profile(Platform::Ios, Path::new("profile"), &bytes)?;
        let mut declared = Dictionary::new();
        declared.insert(
            "aps-environment".to_owned(),
            Value::String("development".to_owned()),
        );

        assert_eq!(profile.unpermitted(None), None);
        assert_eq!(
            profile.unpermitted(Some(&declared)),
            Some("aps-environment")
        );
        Ok(())
    }

    #[test]
    fn parses_profiles_without_devices_but_excludes_them_from_automatic_signing()
    -> anyhow::Result<()> {
        let mut value = profile_value("TEAM.com.example.app", "DEVICE");
        value
            .as_dictionary_mut()
            .context("profile dictionary")?
            .remove("ProvisionedDevices");
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;
        let profile = parse_profile(
            Platform::Ios,
            Path::new("distribution.mobileprovision"),
            &bytes,
        )?;
        let identity = Identity {
            name: "Apple Distribution: Test".to_owned(),
            fingerprint: fingerprint(&[1, 2, 3]),
            selector: sha1_fingerprint(&[1, 2, 3]),
            certificate_der: vec![1, 2, 3],
        };
        assert!(profile.devices.is_empty());
        assert!(!profile.matches("com.example.app", None, &identity));
        assert!(!profile.matches("com.example.app", Some("DEVICE"), &identity));
        Ok(())
    }

    #[test]
    fn accepts_wildcard_application_identifiers() {
        assert!(app_identifier_matches(
            "TEAM.com.example.*",
            "com.example.app"
        ));
        assert!(!app_identifier_matches(
            "TEAM.com.example.*",
            "com.other.app"
        ));
        assert!(app_identifier_matches("TEAM.*", "com.other.app"));
    }

    #[test]
    fn rejects_profiles_for_other_platforms() -> Result<(), Box<dyn std::error::Error>> {
        let mut value = profile_value("TEAM.com.example.app", "DEVICE");
        let Value::Dictionary(root) = &mut value else {
            unreachable!("profile fixture is a dictionary");
        };
        root.insert(
            "Platform".to_owned(),
            Value::Array(vec![Value::String("macOS".to_owned())]),
        );
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;

        assert!(
            parse_profile(
                Platform::Ios,
                Path::new("PROFILE-1.mobileprovision"),
                &bytes
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn parses_and_matches_a_macos_development_profile() -> anyhow::Result<()> {
        let mut value = profile_value("unused", "MAC");
        let root = value.as_dictionary_mut().context("profile dictionary")?;
        let mut entitlements = Dictionary::new();
        entitlements.insert(
            "com.apple.application-identifier".to_owned(),
            Value::String("TEAM.com.example.app".to_owned()),
        );
        root.insert("Entitlements".to_owned(), Value::Dictionary(entitlements));
        root.insert(
            "Platform".to_owned(),
            Value::Array(vec![Value::String("OSX".to_owned())]),
        );
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;
        let identity = Identity {
            name: "Apple Development: Test".to_owned(),
            fingerprint: fingerprint(&[1, 2, 3]),
            selector: sha1_fingerprint(&[1, 2, 3]),
            certificate_der: vec![1, 2, 3],
        };

        let path = Path::new("PROFILE-1.provisionprofile");
        let profile = parse_profile(Platform::Macos, path, &bytes)?;
        assert_eq!(profile.application_identifier, "TEAM.com.example.app");
        assert!(profile.matches("com.example.app", Some("MAC"), &identity));
        assert!(!profile.matches("com.example.app", Some("OTHER-MAC"), &identity));
        assert!(parse_profile(Platform::Ios, path, &bytes).is_err());
        Ok(())
    }

    #[test]
    fn gives_profiles_without_uuids_content_ids() -> Result<(), Box<dyn std::error::Error>> {
        let mut first = profile_value("TEAM.com.example.first", "DEVICE");
        let mut second = profile_value("TEAM.com.example.second", "DEVICE");
        for value in [&mut first, &mut second] {
            let Value::Dictionary(root) = value else {
                unreachable!("profile fixture is a dictionary");
            };
            root.remove("UUID");
        }
        let mut first_bytes = Vec::new();
        let mut second_bytes = Vec::new();
        first.to_writer_xml(&mut first_bytes)?;
        second.to_writer_xml(&mut second_bytes)?;

        let first_profile = parse_profile(
            Platform::Ios,
            Path::new("embedded.mobileprovision"),
            &first_bytes,
        )?;
        let second_profile = parse_profile(
            Platform::Ios,
            Path::new("embedded.mobileprovision"),
            &second_bytes,
        )?;
        assert_eq!(first_profile.id, fingerprint(&first_bytes));
        assert_eq!(second_profile.id, fingerprint(&second_bytes));
        assert_ne!(first_profile.id, second_profile.id);
        Ok(())
    }

    #[test]
    fn fingerprints_are_stable() {
        assert_eq!(
            fingerprint(b"tokamak"),
            "1d86b373a704b11db2b725352c9d5115a3f4099ece1a3820bc39a80b5d6c8521"
        );
        assert_eq!(
            sha1_fingerprint(b"tokamak"),
            "f58ab5abd0dd05d290400f9bd22730cf5563a47a"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_team_ranking_ignores_profiles_without_registered_devices() -> anyhow::Result<()> {
        let identities = [
            development_identity("PERSONAL", Some("Personal Developer"))?,
            development_identity("COMPANY", Some("Example Company"))?,
        ];
        let mut profiles = Vec::new();
        for (identity, team_id, bundle_id) in [
            (&identities[0], "PERSONAL", "com.tokamak.old-app"),
            (&identities[1], "COMPANY", "com.example.other"),
        ] {
            let mut value = profile_value(&format!("{team_id}.{bundle_id}"), "DEVICE");
            let root = value.as_dictionary_mut().context("profile dictionary")?;
            root.insert(
                "TeamIdentifier".to_owned(),
                Value::Array(vec![Value::String(team_id.to_owned())]),
            );
            root.insert(
                "DeveloperCertificates".to_owned(),
                Value::Array(vec![Value::Data(identity.certificate_der.clone())]),
            );
            let mut bytes = Vec::new();
            value.to_writer_xml(&mut bytes)?;
            profiles.push(parse_profile(
                Platform::Ios,
                Path::new("profile.mobileprovision"),
                &bytes,
            )?);
        }
        let project = tempfile::tempdir()?;
        let mut deviceless = profiles[1].clone();
        deviceless.devices.clear();
        deviceless.application_identifier = "COMPANY.com.tokamak.new-app".to_owned();
        profiles.push(deviceless);
        let eligible = super::development_profiles(profiles.into_iter().map(Ok).collect());
        for device_id in [Some("DEVICE"), None] {
            assert_eq!(
                automatic_team_id(
                    project.path(),
                    "com.tokamak.new-app",
                    device_id,
                    &identities,
                    &eligible
                )?,
                "PERSONAL"
            );
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_signing_prefers_an_existing_tokamak_team() -> Result<(), Box<dyn std::error::Error>>
    {
        let identities = vec![
            Identity {
                name: "Apple Development: Personal".to_owned(),
                fingerprint: "personal".to_owned(),
                selector: "personal".to_owned(),
                certificate_der: vec![1, 2, 3],
            },
            Identity {
                name: "Apple Development: Company".to_owned(),
                fingerprint: "company".to_owned(),
                selector: "company".to_owned(),
                certificate_der: vec![4, 5, 6],
            },
        ];
        let profiles = vec![
            Profile {
                path: Path::new("personal.mobileprovision").to_owned(),
                id: "personal".to_owned(),
                name: "Personal".to_owned(),
                team_id: "PERSONAL".to_owned(),
                application_identifier: "PERSONAL.com.tokamak.old-app".to_owned(),
                expiration: SystemTime::now() - Duration::from_secs(1),
                expiration_label: String::new(),
                devices: vec!["DEVICE".to_owned()],
                developer_certificates: vec![vec![1, 2, 3]],
                entitlements: Dictionary::new(),
            },
            Profile {
                path: Path::new("company.mobileprovision").to_owned(),
                id: "company".to_owned(),
                name: "Company".to_owned(),
                team_id: "COMPANY".to_owned(),
                application_identifier: "COMPANY.com.example.other".to_owned(),
                expiration: SystemTime::now() - Duration::from_secs(1),
                expiration_label: String::new(),
                devices: vec!["DEVICE".to_owned()],
                developer_certificates: vec![vec![4, 5, 6]],
                entitlements: Dictionary::new(),
            },
        ];

        assert_eq!(
            automatic_team_id(
                Path::new("/automatic-team-test"),
                "com.tokamak.new-app",
                Some("DEVICE"),
                &identities,
                &profiles,
            )?,
            "PERSONAL"
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signing_probe_uses_the_requested_bundle_id() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let project =
            write_signing_probe(Platform::Ios, temporary.path(), "com.example.probe", None)?;
        let contents = std::fs::read_to_string(project.join("project.pbxproj"))?;
        assert!(contents.contains("PRODUCT_BUNDLE_IDENTIFIER = \"com.example.probe\";"));
        assert!(
            contents.contains("buildConfigurationList = AAAAAAAAAAAAAAAAAAAAAAAA; buildPhases")
        );
        assert_eq!(
            pbx_escape(r#"com.example.\"probe"#),
            r#"com.example.\\\"probe"#
        );
        assert!(
            project
                .join("xcshareddata/xcschemes/TokamakSigningProbe.xcscheme")
                .is_file()
        );
        assert_eq!(
            std::fs::read_to_string(temporary.path().join("main.m"))?,
            "int main(void) { return 0; }\n"
        );
        assert!(contents.contains("CODE_SIGN_ENTITLEMENTS = TokamakSigningProbe.entitlements;"));
        assert_eq!(
            Value::from_file(temporary.path().join("TokamakSigningProbe.entitlements"))?,
            Value::Dictionary(Dictionary::new())
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signing_probe_declares_the_app_entitlements() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let mut declared = Dictionary::new();
        declared.insert(
            "com.apple.developer.aps-environment".to_owned(),
            Value::String("development".to_owned()),
        );

        write_signing_probe(
            Platform::Macos,
            temporary.path(),
            "com.example.probe",
            Some(&declared),
        )?;

        let entitlements =
            Value::from_file(temporary.path().join("TokamakSigningProbe.entitlements"))?;
        let entitlements = entitlements
            .as_dictionary()
            .ok_or("probe entitlements are not a dictionary")?;
        assert!(entitlements.contains_key("keychain-access-groups"));
        assert_eq!(
            entitlements.get("com.apple.developer.aps-environment"),
            Some(&Value::String("development".to_owned()))
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_signing_probe_requests_a_provisioned_keychain_entitlement()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let project =
            write_signing_probe(Platform::Macos, temporary.path(), "com.example.probe", None)?;
        let contents = std::fs::read_to_string(project.join("project.pbxproj"))?;
        assert!(contents.contains("PRODUCT_BUNDLE_IDENTIFIER = \"com.example.probe\";"));
        assert!(contents.contains("SDKROOT = macosx;"));
        assert!(contents.contains("CODE_SIGN_ENTITLEMENTS = TokamakSigningProbe.entitlements;"));
        let entitlements =
            Value::from_file(temporary.path().join("TokamakSigningProbe.entitlements"))?;
        assert!(
            entitlements
                .as_dictionary()
                .is_some_and(|entitlements| entitlements.contains_key("keychain-access-groups"))
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_signing_errors_name_the_mac() {
        let cause = anyhow::anyhow!("Your team has no devices");
        let error =
            automatic_signing_error(Platform::Macos, "com.example.app", Some("MAC"), &cause);
        assert_eq!(
            error.to_string(),
            "automatic macOS signing failed for bundle com.example.app and macOS device MAC:\n\nYour team has no devices"
        );
    }
}
