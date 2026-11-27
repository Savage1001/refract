use crate::{METADATA_LIMIT, read_local_file};
use semver::Version;
use serde::{Deserialize, Serialize};
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
            let asset_name = format!("Refract-Native-Setup-{version}.exe");
            let mut matching = release
                .assets
                .iter()
                .filter(|asset| asset.name == asset_name);
            let asset = matching.next()?;
            if matching.next().is_some() || !(1..=MAX_INSTALLER_BYTES).contains(&asset.size) {
                return None;
            }
            let digest = asset.digest.as_deref()?.strip_prefix("sha256:")?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            Some(UpdateCandidate {
                tag: tag.clone(),
                version,
                asset_name,
                size: asset.size,
                sha256: digest.to_ascii_lowercase(),
            })
        })
        .max_by(|a, b| version_order(&a.version, &b.version))
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
