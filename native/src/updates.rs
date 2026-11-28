use crate::{METADATA_LIMIT, read_local_file};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    fs,
    io::{self, Write},
    path::Path,
    time::Duration,
};

#[cfg(feature = "windows-updater")]
mod windows;
#[cfg(feature = "windows-updater")]
pub use windows::{PreparedUpdate, WindowsUpdater};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const SETTINGS: &str = "update-settings.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateTrack {
    #[default]
    Stable,
    Beta,
}

#[derive(Deserialize, Serialize)]
struct UpdateSettings {
    track: UpdateTrack,
}

impl UpdateTrack {
    pub fn load(profile: &Path) -> Self {
        read_local_file(profile, &profile.join(SETTINGS), METADATA_LIMIT)
            .ok()
            .and_then(|contents| serde_json::from_slice::<UpdateSettings>(&contents).ok())
            .map_or(Self::default(), |settings| settings.track)
    }

    pub fn save(self, profile: &Path) -> io::Result<()> {
        fs::create_dir_all(profile)?;
        let mut stage = tempfile::NamedTempFile::new_in(profile)?;
        serde_json::to_writer(&mut stage, &UpdateSettings { track: self })?;
        stage.write_all(b"\n")?;
        stage.as_file().sync_all()?;
        stage
            .persist(profile.join(SETTINGS))
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

pub fn installed_version() -> Version {
    let package: serde_json::Value =
        serde_json::from_str(include_str!("../../package.json")).expect("valid package manifest");
    Version::parse(package["version"].as_str().expect("app version"))
        .expect("app version follows semver")
}

#[derive(Deserialize)]
pub struct Release {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
}

impl Asset {
    fn sha256(&self) -> Option<&str> {
        let digest = self.digest.as_deref()?.strip_prefix("sha256:")?;
        (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then_some(digest)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct UpdateCandidate {
    tag: String,
    version: Version,
    asset_name: String,
    size: u64,
    sha256: String,
}

impl UpdateCandidate {
    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

pub fn select_windows_update(
    releases: &[Release],
    track: UpdateTrack,
    current: &Version,
) -> Option<UpdateCandidate> {
    releases
        .iter()
        .filter_map(|release| {
            let tag = &release.tag_name;
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
            let pre = version.pre.as_str();
            let eligible = !release.draft
                && if pre.is_empty() {
                    !release.prerelease
                } else {
                    track == UpdateTrack::Beta && release.prerelease && supported_prerelease(pre)
                };
            if !eligible || version_order(&version, current) != Ordering::Greater {
                return None;
            }
            let (asset, sha256) = release.native_windows_installer(&version)?;
            Some(UpdateCandidate {
                tag: tag.clone(),
                version,
                asset_name: asset.name.clone(),
                size: asset.size,
                sha256,
            })
        })
        .max_by(|a, b| version_order(&a.version, &b.version))
}

impl Release {
    fn unique_asset(&self, name: &str) -> Option<&Asset> {
        let mut matching = self.assets.iter().filter(|asset| asset.name == name);
        let asset = matching.next()?;
        matching.next().is_none().then_some(asset)
    }

    fn verified_installer(&self, name: &str) -> Option<(&Asset, String)> {
        let asset = self.unique_asset(name)?;
        if !(1..=MAX_INSTALLER_BYTES).contains(&asset.size) {
            return None;
        }
        Some((asset, asset.sha256()?.to_ascii_lowercase()))
    }

    fn native_windows_installer(&self, version: &Version) -> Option<(&Asset, String)> {
        let canonical = format!("Refract-Setup-{version}.exe");
        self.verified_installer(&canonical)
            .filter(|(_, digest)| {
                // The checksum emitted by package.cts distinguishes a native
                // installer from an Electron release with the same asset name.
                let marker = format!("Refract-Native-Setup-{version}.{digest}.sha256");
                let contents = format!("{digest}  {canonical}\n");
                self.unique_asset(&marker).is_some_and(|asset| {
                    asset.size == contents.len() as u64
                        && asset.sha256().is_some_and(|checksum| {
                            checksum.eq_ignore_ascii_case(&format!(
                                "{:x}",
                                Sha256::digest(contents.as_bytes())
                            ))
                        })
                })
            })
            .or_else(|| self.verified_installer(&format!("Refract-Native-Setup-{version}.exe")))
    }
}

fn supported_prerelease(pre: &str) -> bool {
    numbered_pre_release(pre).is_some()
        || pre
            .strip_prefix("beta.")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

fn numbered_pre_release(pre: &str) -> Option<&str> {
    let number = pre.strip_prefix("pre-release-")?;
    (!number.is_empty() && !number.starts_with('0') && number.bytes().all(|b| b.is_ascii_digit()))
        .then_some(number)
}

fn version_order(a: &Version, b: &Version) -> Ordering {
    if (a.major, a.minor, a.patch) == (b.major, b.minor, b.patch)
        && let (Some(a), Some(b)) = (
            numbered_pre_release(a.pre.as_str()),
            numbered_pre_release(b.pre.as_str()),
        )
    {
        return a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    }
    a.cmp_precedence(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, prerelease: bool, draft: bool, native: bool) -> Release {
        let version = tag.strip_prefix('v').unwrap_or(tag);
        let prefix = if native {
            "Refract-Native-Setup"
        } else {
            "Refract-Setup"
        };
        serde_json::from_value(json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "assets": [{
                "name": format!("{prefix}-{version}.exe"),
                "digest": format!("sha256:{}", "A".repeat(64)),
                "size": 1000,
            }],
        }))
        .unwrap()
    }

    #[test]
    fn updates_only_to_verified_native_assets_on_the_selected_track() {
        let mut bad_digest = release("9.0.0", false, false, true);
        bad_digest.assets[0].digest = None;
        let releases = [
            release("v1.1.0", false, false, true),
            release("v1.2.0-beta.7", true, false, true),
            release("v1.2.0-beta.10", true, false, true),
            release("v1.3.0-pre-release-9", true, false, true),
            release("v1.3.0-pre-release-10", true, false, true),
            release("v2.0.0", false, false, false),
            release("v3.0.0", false, true, true),
            release("v4.0.0-alpha.1", true, false, true),
            release("4.1.0-pre-release-01", true, false, true),
            release("4.2.0-beta.1", false, false, true),
            bad_digest,
        ];
        let current = Version::new(1, 0, 0);
        let stable = select_windows_update(&releases, UpdateTrack::Stable, &current).unwrap();
        assert_eq!(stable.tag, "v1.1.0");
        assert_eq!(stable.sha256, "a".repeat(64));
        let beta = select_windows_update(&releases, UpdateTrack::Beta, &current).unwrap();
        assert_eq!(beta.tag, "v1.3.0-pre-release-10");
        assert!(select_windows_update(&releases, UpdateTrack::Beta, beta.version()).is_none());
        assert!(select_windows_update(&releases, UpdateTrack::Stable, beta.version()).is_none());
    }

    #[test]
    fn requires_a_checksum_marker_that_binds_the_canonical_installer_to_its_version() {
        let current = Version::new(1, 0, 0);
        let mut canonical = release("v2.0.0", false, false, false);
        let digest = "a".repeat(64);
        let marker = format!("Refract-Native-Setup-2.0.0.{digest}.sha256");
        let contents = format!("{digest}  Refract-Setup-2.0.0.exe\n");
        canonical.assets.push(Asset {
            name: marker.clone(),
            size: contents.len() as u64,
            digest: Some(format!("sha256:{:x}", Sha256::digest(b"unrelated asset"))),
        });
        let pick = |release: &Release| {
            select_windows_update(std::slice::from_ref(release), UpdateTrack::Stable, &current)
        };
        assert!(pick(&canonical).is_none());

        let checksum = format!("sha256:{:x}", Sha256::digest(contents.as_bytes()));
        canonical.assets[1].digest = Some(checksum.clone());
        let update = pick(&canonical).unwrap();
        assert_eq!(update.asset_name, "Refract-Setup-2.0.0.exe");
        assert_eq!(update.sha256, digest);

        canonical.assets.push(Asset {
            name: marker,
            size: contents.len() as u64,
            digest: Some(checksum),
        });
        assert!(pick(&canonical).is_none());
        canonical.assets.pop();
        canonical.assets[0].digest = Some(format!("sha256:{}", "b".repeat(64)));
        assert!(pick(&canonical).is_none());
    }

    #[test]
    fn preserves_the_electron_update_track_format_and_stable_default() {
        let profile = tempfile::tempdir().unwrap();
        let settings = profile.path().join(SETTINGS);
        assert_eq!(UpdateTrack::load(profile.path()), UpdateTrack::Stable);
        fs::write(&settings, b"{\"track\":\"beta\"}\n").unwrap();
        assert_eq!(UpdateTrack::load(profile.path()), UpdateTrack::Beta);
        UpdateTrack::Stable.save(profile.path()).unwrap();
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "{\"track\":\"stable\"}\n"
        );
        fs::write(&settings, b"{\"track\":\"invalid\"}").unwrap();
        assert_eq!(UpdateTrack::load(profile.path()), UpdateTrack::Stable);
    }
}
