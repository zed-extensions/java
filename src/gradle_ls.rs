use std::{
    fs::metadata,
    path::{Path, PathBuf},
};

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Worktree,
    serde_json::Value, set_language_server_installation_status,
};

use crate::{
    downloadable::Downloadable,
    util::{create_path_if_not_exists, remove_all_files_except},
};

const INSTALL_PATH: &str = "gradle-ls";
const SUPPORTED_VERSION: &str = "3.18.0";

pub struct GradleLs {
    cached_path: Option<PathBuf>,
}

impl GradleLs {
    pub fn new() -> Self {
        Self { cached_path: None }
    }
}

impl Downloadable for GradleLs {
    const INSTALL_PATH: &'static str = INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        find_supported_installation(Path::new(INSTALL_PATH))
    }

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self, _worktree: &Worktree) -> zed::Result<String> {
        Ok(SUPPORTED_VERSION.to_string())
    }

    fn version_for_download(
        &self,
        _language_server_id: &LanguageServerId,
        _configuration: &Option<Value>,
        _worktree: &Worktree,
    ) -> zed::Result<(String, bool)> {
        // The bridge protocol is coupled to vscode-gradle 3.18. Do not allow
        // the shared update-check cache to substitute an incompatible version.
        Ok((SUPPORTED_VERSION.to_string(), false))
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        let version_dir = PathBuf::from(INSTALL_PATH).join(version);
        let lib_dir = version_dir.join("lib");

        if metadata(&lib_dir).is_ok_and(|m| m.is_dir()) {
            self.cached_path = Some(version_dir.clone());
            return Ok(version_dir);
        }

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        create_path_if_not_exists(&version_dir)
            .map_err(|err| format!("Failed to create Gradle LS directory: {err}"))?;

        let download_url = format!(
            "https://open-vsx.org/api/vscjava/vscode-gradle/{SUPPORTED_VERSION}/file/vscjava.vscode-gradle-{SUPPORTED_VERSION}.vsix"
        );

        // The VSIX is a zip file. We download and extract it into a temp location,
        // then move the lib/ directory to our version directory.
        let vsix_dir = PathBuf::from(INSTALL_PATH).join("_vsix_temp");
        let vsix_dir_str = vsix_dir.to_string_lossy().to_string();

        zed::download_file(&download_url, &vsix_dir_str, DownloadedFileType::Zip)
            .map_err(|err| format!("Failed to download Gradle LS VSIX: {err}"))?;

        // The VSIX extracts with extension/lib/ containing the JARs
        let extracted_lib = vsix_dir.join("extension").join("lib");
        if !metadata(&extracted_lib).is_ok_and(|m| m.is_dir()) {
            let _ = std::fs::remove_dir_all(&vsix_dir);
            return Err(
                "Downloaded VSIX does not contain expected extension/lib/ directory".to_string(),
            );
        }

        // Move extension/lib/ to our version directory
        std::fs::rename(&extracted_lib, &lib_dir)
            .map_err(|err| format!("Failed to move lib directory: {err}"))?;

        // Cleanup VSIX temp
        let _ = std::fs::remove_dir_all(&vsix_dir);

        // Remove old versions
        let _ = remove_all_files_except(INSTALL_PATH, version);

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );

        self.cached_path = Some(version_dir.clone());
        Ok(version_dir)
    }
}

fn find_supported_installation(install_path: &Path) -> Option<PathBuf> {
    let version_dir = install_path.join(SUPPORTED_VERSION);
    version_dir.join("lib").is_dir().then_some(version_dir)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_install_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("gradle-ls-version-test-{nonce:x}"))
    }

    #[test]
    fn local_installation_only_accepts_supported_version() {
        let install_path = temp_install_path();
        std::fs::create_dir_all(install_path.join("3.17.0/lib")).unwrap();
        assert_eq!(find_supported_installation(&install_path), None);

        let supported = install_path.join(SUPPORTED_VERSION);
        std::fs::create_dir_all(supported.join("lib")).unwrap();
        assert_eq!(find_supported_installation(&install_path), Some(supported));

        std::fs::remove_dir_all(install_path).unwrap();
    }
}
