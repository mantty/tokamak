#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Apple platform-pack plist generation and signing asset discovery.

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

#[cfg(any(target_os = "macos", test))]
use anyhow::Context;
use anyhow::{Result, bail};
#[cfg(any(target_os = "macos", test))]
use plist::{Dictionary, Value as PlistValue};
#[cfg(target_os = "macos")]
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "macos", test))]
use sha1::{Digest as Sha1Digest, Sha1};
#[cfg(any(target_os = "macos", test))]
use sha2::{Digest as Sha2Digest, Sha256};
/// A signing identity and provisioning profile selected for an iOS app.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct Selection {
    identity: String,
    profile: PathBuf,
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
        signing_inventory(&identities, &discover_profiles())
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
        let selection = resolve_macos(project, bundle_id, device_id)?;
        sign_bundle(bundle, &selection)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (project, bundle, bundle_id, device_id);
        bail!("iOS signing requires a macOS host with Xcode");
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
}

#[cfg(any(target_os = "macos", test))]
impl Profile {
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
const SIGNING_PROBE_PROJECT: &str = include_str!("../resources/ios-signing-probe.pbxproj");
#[cfg(target_os = "macos")]
const SIGNING_PROBE_SCHEME: &str = include_str!("../resources/ios-signing-probe.xcscheme");
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

#[cfg(target_os = "macos")]
fn resolve_macos(project: &Path, bundle_id: &str, device_id: Option<&str>) -> Result<Selection> {
    let environment_team = env::var("TOKAMAK_IOS_TEAM_ID").ok();
    let configured_team = configured_team_id(environment_team.as_deref())?;
    if let Some(selection) = explicit_selection(configured_team.is_some(), device_id)? {
        return Ok(selection);
    }

    let profiles = development_profiles(discover_profiles());
    let identities = discover_identities()?;
    let team_id = match configured_team {
        Some(team_id) => team_id,
        None => automatic_team_id(project, bundle_id, device_id, &identities, &profiles)
            .map_err(|error| automatic_signing_error(bundle_id, device_id, &error))?,
    };
    let selected_team = team_id.as_str();
    let mut candidates = identities
        .iter()
        .flat_map(|identity| {
            profiles
                .iter()
                .filter(move |profile| {
                    profile.team_id == selected_team
                        && profile.matches(bundle_id, device_id, identity)
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
        return automatic_selection(project, bundle_id, device_id, &team_id)
            .map_err(|error| automatic_signing_error(bundle_id, device_id, &error));
    }

    let key = cache_key(project, bundle_id, device_id);
    let mut cache = load_cache();
    let candidate = match cache.entries.get(&key).and_then(|cached| {
        candidates.iter().find(|candidate| {
            candidate.identity.fingerprint == cached.identity_fingerprint
                && candidate.profile.id == cached.profile_id
        })
    }) {
        Some(candidate) => candidate.clone(),
        None => choose_candidate(&candidates)?,
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
    save_cache(&cache);
    Ok(selection)
}

#[cfg(target_os = "macos")]
fn sign_bundle(bundle: &Path, selection: &Selection) -> Result<()> {
    let profile = fs::read(&selection.profile).with_context(|| {
        format!(
            "read iOS provisioning profile {}",
            selection.profile.display()
        )
    })?;
    fs::write(bundle.join("embedded.mobileprovision"), &profile)
        .context("embed the iOS provisioning profile")?;

    let profile = decode_profile(&profile).context("decode the iOS provisioning profile")?;
    let entitlements = profile
        .as_dictionary()
        .and_then(|root| root.get("Entitlements"))
        .context("provisioning profile entitlements are missing")?;
    let temporary = tempfile::tempdir().context("create iOS signing directory")?;
    let entitlements_path = temporary.path().join("entitlements.plist");
    let mut file = fs::File::create(&entitlements_path).context("create iOS entitlements")?;
    entitlements
        .to_writer_xml(&mut file)
        .context("write iOS signing entitlements")?;

    let output = Command::new("codesign")
        .args([
            "--force",
            "--timestamp=none",
            "--sign",
            &selection.identity,
            "--generate-entitlement-der",
            "--entitlements",
        ])
        .arg(&entitlements_path)
        .arg(bundle)
        .output()
        .context("sign iOS application bundle")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!("codesign failed: {}", command_output_detail(&output));
    }
}

#[cfg(target_os = "macos")]
fn automatic_selection(
    project: &Path,
    bundle_id: &str,
    device_id: Option<&str>,
    team_id: &str,
) -> Result<Selection> {
    println!(
        "No matching iOS signing profile found; asking Xcode to provision {bundle_id} automatically"
    );

    let temporary = tempfile::tempdir().context("create Xcode signing probe directory")?;
    let project_path = write_signing_probe(temporary.path(), bundle_id)?;
    let derived_data = temporary.path().join("DerivedData");
    let destination = device_id.map_or_else(
        || "generic/platform=iOS".to_owned(),
        |device_id| format!("id={device_id}"),
    );
    let mut command = Command::new("xcodebuild");
    command
        .current_dir(temporary.path())
        .arg("-project")
        .arg(&project_path)
        .args([
            "-scheme",
            "TokamakSigningProbe",
            "-configuration",
            "Debug",
            "-sdk",
            "iphoneos",
        ])
        .arg("-destination")
        .arg(destination)
        .arg("-derivedDataPath")
        .arg(&derived_data)
        .arg("-allowProvisioningUpdates");
    if device_id.is_some() {
        command.arg("-allowProvisioningDeviceRegistration");
    }
    let output = command
        .args([
            "CODE_SIGN_STYLE=Automatic",
            "CODE_SIGN_IDENTITY=Apple Development",
        ])
        .arg(format!("DEVELOPMENT_TEAM={team_id}"))
        .arg("build")
        .stdin(Stdio::inherit())
        .output()
        .context("run xcodebuild for automatic iOS provisioning")?;
    if !output.status.success() {
        bail!("xcodebuild failed: {}", command_output_detail(&output));
    }

    let embedded_profile = derived_data
        .join("Build/Products/Debug-iphoneos/TokamakSigningProbe.app/embedded.mobileprovision");
    let bytes = fs::read(&embedded_profile).with_context(|| {
        format!(
            "Xcode did not produce a provisioning profile at {}",
            embedded_profile.display()
        )
    })?;
    let profile = parse_profile(&embedded_profile, &bytes)
        .context("parse the provisioning profile generated by Xcode")?;
    let identities = discover_identities()?;
    let identity = identities
        .iter()
        .find(|identity| {
            profile.team_id == team_id && profile.matches(bundle_id, device_id, identity)
        })
        .context(
            "Xcode generated a profile that does not match an installed Apple Development identity",
        )?;
    let profile_path = persist_profile(&profile.id, &bytes)?;
    let key = cache_key(project, bundle_id, device_id);
    let mut cache = load_cache();
    cache.entries.insert(
        key,
        CachedSelection {
            identity_fingerprint: identity.fingerprint.clone(),
            profile_id: profile.id,
        },
    );
    save_cache(&cache);
    Ok(Selection {
        identity: identity.selector.clone(),
        profile: profile_path,
    })
}

#[cfg(target_os = "macos")]
fn automatic_team_id(
    project: &Path,
    bundle_id: &str,
    device_id: Option<&str>,
    identities: &[Identity],
    profiles: &[Profile],
) -> Result<String> {
    if let Some(cached) = load_cache()
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

#[cfg(target_os = "macos")]
fn write_signing_probe(root: &Path, bundle_id: &str) -> Result<PathBuf> {
    let project = root.join("TokamakSigningProbe.xcodeproj");
    let schemes = project.join("xcshareddata/xcschemes");
    fs::create_dir_all(&schemes).context("create Xcode signing probe project")?;
    fs::write(
        project.join("project.pbxproj"),
        SIGNING_PROBE_PROJECT.replace("__TOKAMAK_BUNDLE_ID__", &pbx_escape(bundle_id)),
    )?;
    fs::write(root.join("main.m"), SIGNING_PROBE_SOURCE)?;
    fs::write(
        schemes.join("TokamakSigningProbe.xcscheme"),
        SIGNING_PROBE_SCHEME,
    )?;
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
    bundle_id: &str,
    device_id: Option<&str>,
    error: &anyhow::Error,
) -> anyhow::Error {
    let target = device_id.map_or_else(
        || "a generic iOS device".to_owned(),
        |device_id| format!("iOS device {device_id}"),
    );
    anyhow::anyhow!(
        "automatic iOS signing failed for bundle {bundle_id} and {target}:\n\n{error:#}"
    )
}

#[cfg(target_os = "macos")]
fn team_command(device_id: Option<&str>) -> String {
    device_id.map_or_else(
        || "tok build ios --set ios-team-id=TEAM_ID".to_owned(),
        |device_id| format!("tok dev {device_id} --set ios-team-id=TEAM_ID -- <dev-command>"),
    )
}

#[cfg(target_os = "macos")]
fn manual_command(device_id: Option<&str>) -> String {
    device_id.map_or_else(
        || {
            "tok build ios --set ios-signing-identity=IDENTITY_SHA1 \\\n    --set ios-provisioning-profile=/path/to/profile.mobileprovision"
                .to_owned()
        },
        |device_id| {
            format!(
                "tok dev {device_id} \\\n    --set ios-signing-identity=IDENTITY_SHA1 \\\n    --set ios-provisioning-profile=/path/to/profile.mobileprovision \\\n    -- <dev-command>"
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
                     Automatic: set TOKAMAK_IOS_TEAM_ID or use --set ios-team-id=TEAM_ID\n  \
                     Manual:    set BOTH TOKAMAK_IOS_SIGNING_IDENTITY and\n             \
                     TOKAMAK_IOS_PROVISIONING_PROFILE, or use the matching --set options\n\n  \
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
fn configured_team_id(environment: Option<&str>) -> Result<Option<String>> {
    let Some(team_id) = environment else {
        return Ok(None);
    };
    let team_id = team_id.trim();
    if team_id.is_empty() {
        bail!("TOKAMAK_IOS_TEAM_ID must not be empty");
    }
    Ok(Some(team_id.to_owned()))
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
fn discover_profiles() -> Vec<Result<Profile>> {
    let mut paths = profile_paths();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| parse_profile(&path, &bytes))
                .with_context(|| path.display().to_string())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn profile_paths() -> Vec<PathBuf> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut directories = vec![
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ];
    if let Some(directory) = profile_cache_directory() {
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
                        .is_some_and(|extension| extension == "mobileprovision")
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn persist_profile(profile_id: &str, bytes: &[u8]) -> Result<PathBuf> {
    let directory = profile_cache_directory().context("resolve tokamak iOS signing cache")?;
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "create tokamak iOS signing profile cache: {}",
            directory.display()
        )
    })?;
    let path = directory.join(format!("{profile_id}.mobileprovision"));
    fs::write(&path, bytes)
        .with_context(|| format!("save Xcode provisioning profile: {}", path.display()))?;
    fs::canonicalize(path).context("resolve saved Xcode provisioning profile")
}

#[cfg(any(target_os = "macos", test))]
fn parse_profile(path: &Path, bytes: &[u8]) -> Result<Profile> {
    let value = decode_profile(bytes)?;
    let root = value
        .as_dictionary()
        .context("provisioning profile root is not a dictionary")?;
    let entitlements = root
        .get("Entitlements")
        .and_then(PlistValue::as_dictionary)
        .context("provisioning profile entitlements are missing")?;
    let application_identifier = string_value(entitlements, "application-identifier")
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
    if !platforms.is_empty()
        && !platforms
            .iter()
            .any(|platform| matches!(platform.as_str(), "iOS" | "iPhoneOS"))
    {
        bail!("provisioning profile is not for iOS");
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
fn cache_path() -> Option<PathBuf> {
    let base = env::var_os("HOME")
        .map(PathBuf::from)?
        .join("Library/Application Support");
    Some(base.join("tokamak/ios-signing.json"))
}

#[cfg(target_os = "macos")]
fn profile_cache_directory() -> Option<PathBuf> {
    cache_path()?
        .parent()
        .map(|parent| parent.join("ios-signing-profiles"))
}

#[cfg(target_os = "macos")]
fn load_cache() -> SelectionCache {
    let Some(path) = cache_path() else {
        return SelectionCache::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn save_cache(cache: &SelectionCache) {
    let Some(path) = cache_path() else {
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
        eprintln!("warning: could not save iOS signing selection: {error}");
    }
}

#[cfg(target_os = "macos")]
fn choose_candidate(candidates: &[Candidate]) -> Result<Candidate> {
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "multiple valid iOS signing profiles match this app and device; set TOKAMAK_IOS_SIGNING_IDENTITY and TOKAMAK_IOS_PROVISIONING_PROFILE for non-interactive use"
        );
    }

    println!("Multiple valid iOS signing profiles match this app and device:");
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
            bail!("no iOS signing profile was selected");
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
        Identity, app_identifier_matches, configured_team_id, fingerprint, parse_profile,
        sha1_fingerprint,
    };

    #[test]
    fn configured_team_id_reads_the_environment() -> anyhow::Result<()> {
        assert_eq!(configured_team_id(Some(" ENV "))?.as_deref(), Some("ENV"));
        assert_eq!(configured_team_id(None)?, None);
        Ok(())
    }

    #[test]
    fn empty_team_selection_reports_the_environment() -> anyhow::Result<()> {
        let error = configured_team_id(Some(" "))
            .err()
            .context("expected an empty-team error")?;
        assert_eq!(error.to_string(), "TOKAMAK_IOS_TEAM_ID must not be empty");
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
        let profile = parse_profile(Path::new("/profiles/app.mobileprovision"), &bytes)?;
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
        let profile = parse_profile(Path::new("/profiles/dev.mobileprovision"), &bytes)?;
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
        let error = automatic_signing_error("com.example.app", Some("DEVICE"), &error);
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
                "    tok dev DEVICE --set ios-team-id=TEAM_ID -- <dev-command>\n\n",
                "Tokamak will select the signing identity and provisioning profile automatically.\n\n",
                "Alternatively, use manual signing:\n\n",
                "  First, list installed identities and provisioning profiles:\n",
                "    tok certs\n\n",
                "  Both values are required and must belong together.\n\n",
                "  Set them for this shell:\n",
                "    export TOKAMAK_IOS_SIGNING_IDENTITY=\"IDENTITY_SHA1\"\n",
                "    export TOKAMAK_IOS_PROVISIONING_PROFILE=\"/path/to/profile.mobileprovision\"\n\n",
                "  Or set them for a single command:\n",
                "    tok dev DEVICE \\\n    --set ios-signing-identity=IDENTITY_SHA1 \\\n    --set ios-provisioning-profile=/path/to/profile.mobileprovision \\\n    -- <dev-command>"
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
            profiles.push(parse_profile(Path::new("profile.mobileprovision"), &bytes)?);
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
        assert!(error.contains("tok build ios --set ios-team-id=TEAM_ID"));
        assert!(!error.contains("tok dev"));
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_signing_errors_preserve_details_without_unrelated_manual_instructions() {
        let cause =
            anyhow::anyhow!("No devices are registered").context("Xcode provisioning failed");
        let error = automatic_signing_error("com.example.app", None, &cause);
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
        let profile = parse_profile(Path::new("PROFILE-1.mobileprovision"), &bytes)?;
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
    fn parses_profiles_without_devices_but_excludes_them_from_automatic_signing()
    -> anyhow::Result<()> {
        let mut value = profile_value("TEAM.com.example.app", "DEVICE");
        value
            .as_dictionary_mut()
            .context("profile dictionary")?
            .remove("ProvisionedDevices");
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes)?;
        let profile = parse_profile(Path::new("distribution.mobileprovision"), &bytes)?;
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

        assert!(parse_profile(Path::new("PROFILE-1.mobileprovision"), &bytes).is_err());
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

        let first_profile = parse_profile(Path::new("embedded.mobileprovision"), &first_bytes)?;
        let second_profile = parse_profile(Path::new("embedded.mobileprovision"), &second_bytes)?;
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
            profiles.push(parse_profile(Path::new("profile.mobileprovision"), &bytes)?);
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
        let project = write_signing_probe(temporary.path(), "com.example.probe")?;
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
        Ok(())
    }
}
