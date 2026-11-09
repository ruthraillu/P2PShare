use semver::Version;
use serde_json::Value;

const APK: &str = "P2PShare-android-arm64.apk";
const RELEASES: &str = "https://github.com/ruthraillu/P2PShare/releases/tag";

pub(crate) fn available(metadata: &str, installed: &Version) -> Option<Version> {
    let release: Value = serde_json::from_str(metadata).ok()?;
    if release["draft"].as_bool() != Some(false) || release["prerelease"].as_bool() != Some(false) {
        return None;
    }
    let version = Version::parse(release["tag_name"].as_str()?.strip_prefix('v')?).ok()?;
    if !version.pre.is_empty() || !version.build.is_empty() || &version <= installed {
        return None;
    }
    release["assets"]
        .as_array()?
        .iter()
        .any(|asset| {
            asset["name"].as_str() == Some(APK)
                && asset["state"].as_str() == Some("uploaded")
                && asset["size"].as_u64().is_some_and(|size| size > 0)
        })
        .then_some(version)
}

#[cfg(target_os = "android")]
pub(crate) fn installed() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("valid workspace version")
}

pub(crate) fn page(version: &Version) -> String {
    format!("{RELEASES}/v{version}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({
            "tag_name": "v0.14.0", "draft": false, "prerelease": false,
            "html_url": "https://example.com/untrusted",
            "assets": [{"name": APK, "state": "uploaded", "size": 123}]
        })
    }

    #[test]
    fn newer_apk_release_uses_the_workspace_version_and_fixed_github_page() {
        let current = Version::new(0, 13, 0);
        let version = available(&fixture().to_string(), &current).unwrap();
        assert_eq!(version, Version::new(0, 14, 0));
        assert_eq!(
            page(&version),
            "https://github.com/ruthraillu/P2PShare/releases/tag/v0.14.0"
        );
        assert!(available(&fixture().to_string(), &version).is_none());
        assert!(available(&fixture().to_string(), &Version::new(0, 15, 0)).is_none());
    }

    #[test]
    fn ignores_missing_apks_prereleases_and_bad_metadata() {
        let current = Version::new(0, 13, 0);
        let rejected =
            |release: Value| assert!(available(&release.to_string(), &current).is_none());
        for key in ["draft", "prerelease"] {
            let mut release = fixture();
            release[key] = json!(true);
            rejected(release);
        }
        for tag in ["v0.14.0-rc.1", "v0.14.0+other", "v/../../other", "0.14.0"] {
            let mut release = fixture();
            release["tag_name"] = json!(tag);
            rejected(release);
        }
        for assets in [
            json!([]),
            json!([{"name": "P2PShare.exe", "state": "uploaded", "size": 10}]),
            json!([{"name": APK, "state": "new", "size": 10}]),
        ] {
            let mut release = fixture();
            release["assets"] = assets;
            rejected(release);
        }
        assert!(available("invalid JSON", &current).is_none());
    }
}
