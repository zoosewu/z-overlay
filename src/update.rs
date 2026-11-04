//! Finding a newer release on GitHub and swapping it in for the running executable.

use crate::Result;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const REPOSITORY: &str = "zoosewu/z-overlay";
pub const EXECUTABLE_ASSET: &str = "z-overlay.exe";
pub const CHECKSUM_ASSET: &str = "z-overlay.exe.sha256";

pub fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
}

/// A `major.minor.patch` version; a leading `v` and any suffix after `-` or `+` are ignored.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().trim_start_matches('v');
        let core = value.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is major.minor.patch")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A published release that carries the executable and its checksum.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub page: String,
    pub executable_url: String,
    pub checksum_url: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Reads GitHub's "latest release" response. A release that is not a usable
/// update — a draft, a prerelease, an odd tag or missing files — is `None`.
pub fn parse_release(json: &str) -> Result<Option<Release>> {
    let release: GitHubRelease = serde_json::from_str(json)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let asset = |name: &str| {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.clone())
    };
    Ok((|| {
        Some(Release {
            version: Version::parse(&release.tag_name)?,
            page: release.html_url.clone(),
            executable_url: asset(EXECUTABLE_ASSET)?,
            checksum_url: asset(CHECKSUM_ASSET)?,
        })
    })())
}

/// The release, if it is newer than the running version.
pub fn newer(release: Option<Release>, current: Version) -> Option<Release> {
    release.filter(|release| release.version > current)
}

/// Checks `data` against a `sha256sum` line: the hex digest, then the file name.
pub fn verify_checksum(data: &[u8], checksum_file: &str) -> Result<()> {
    let expected = checksum_file
        .split_whitespace()
        .next()
        .ok_or("the checksum file is empty")?
        .to_ascii_lowercase();
    let actual: String = Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual == expected {
        Ok(())
    } else {
        Err("the download does not match its checksum".into())
    }
}

/// Where the replaced executable is kept until the next start removes it.
pub fn old_executable(executable: &Path) -> PathBuf {
    executable.with_extension("exe.old")
}

/// Puts `data` in place of `executable`. Windows lets a running executable be
/// renamed but not overwritten, so the current one is moved aside first.
pub fn replace_executable(executable: &Path, data: &[u8]) -> Result<()> {
    let new = executable.with_extension("exe.new");
    let old = old_executable(executable);
    std::fs::write(&new, data)?;
    let result = (|| -> Result<()> {
        match std::fs::remove_file(&old) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        std::fs::rename(executable, &old)?;
        if let Err(error) = std::fs::rename(&new, executable) {
            // Put the running version back so the folder still has an executable.
            let _ = std::fs::rename(&old, executable);
            return Err(error.into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(tag: &str, draft: bool, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|name| {
                format!(
                    r#"{{"name":"{name}","browser_download_url":"https://example.com/{tag}/{name}"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://example.com/{tag}","draft":{draft},"prerelease":false,"assets":[{}],"body":"notes"}}"#,
            assets.join(",")
        )
    }

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("v0.1.2"), Some(Version(0, 1, 2)));
        assert_eq!(Version::parse("1.10.0-beta.1"), Some(Version(1, 10, 0)));
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("vx.1.2"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(0, 1, 2).to_string(), "0.1.2");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_release_needs_a_version_tag_and_both_files() {
        let both = [EXECUTABLE_ASSET, CHECKSUM_ASSET];
        let release = parse_release(&release_json("v0.2.0", false, &both))
            .unwrap()
            .unwrap();
        assert_eq!(release.version, Version(0, 2, 0));
        assert_eq!(release.page, "https://example.com/v0.2.0");
        assert_eq!(
            release.executable_url,
            "https://example.com/v0.2.0/z-overlay.exe"
        );
        assert_eq!(
            release.checksum_url,
            "https://example.com/v0.2.0/z-overlay.exe.sha256"
        );

        assert_eq!(
            parse_release(&release_json("v0.2.0", true, &both)).unwrap(),
            None
        );
        assert_eq!(
            parse_release(&release_json("latest", false, &both)).unwrap(),
            None
        );
        let missing = [EXECUTABLE_ASSET];
        assert_eq!(
            parse_release(&release_json("v0.2.0", false, &missing)).unwrap(),
            None
        );
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn only_a_higher_version_is_offered() {
        let both = [EXECUTABLE_ASSET, CHECKSUM_ASSET];
        let release = parse_release(&release_json("v0.2.0", false, &both)).unwrap();
        assert!(newer(release.clone(), Version(0, 1, 9)).is_some());
        assert!(newer(release.clone(), Version(0, 2, 0)).is_none());
        assert!(newer(release, Version(0, 3, 0)).is_none());
        assert!(newer(None, Version(0, 1, 0)).is_none());
    }

    #[test]
    fn checksum_matches_sha256sum_output() {
        // sha256("abc")
        let line =
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD *z-overlay.exe\n";
        assert!(verify_checksum(b"abc", line).is_ok());
        assert!(verify_checksum(b"abd", line).is_err());
        assert!(verify_checksum(b"abc", "").is_err());
    }

    #[test]
    fn replacing_keeps_the_old_executable_aside() {
        let dir = std::env::temp_dir().join(format!("z-overlay-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("z-overlay.exe");
        std::fs::write(&executable, b"one").unwrap();

        replace_executable(&executable, b"two").unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"two");
        assert_eq!(std::fs::read(old_executable(&executable)).unwrap(), b"one");

        // A leftover from the previous update is replaced.
        replace_executable(&executable, b"three").unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"three");
        assert_eq!(std::fs::read(old_executable(&executable)).unwrap(), b"two");
        assert!(!dir.join("z-overlay.exe.new").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
