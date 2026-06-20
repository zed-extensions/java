use std::{
    fs::{metadata, read_dir},
    path::PathBuf,
};

use zed_extension_api::{
    self as zed, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, set_language_server_installation_status,
};

use crate::{
    downloadable::Downloadable,
    util::{create_path_if_not_exists, mark_checked_once, remove_all_files_except},
};

const INSTALL_PATH: &str = "gradle-ls";
const GITHUB_REPO: &str = "microsoft/vscode-gradle";
const VSIX_PUBLISHER: &str = "vscjava";
const VSIX_EXTENSION: &str = "vscode-gradle";

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
        let prefix = PathBuf::from(INSTALL_PATH);
        read_dir(&prefix)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .find(|path| path.join("lib").is_dir())
    }

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        let release = zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: false,
                pre_release: false,
            },
        )
        .map_err(|err| {
            format!("Failed to fetch latest Gradle LS release from {GITHUB_REPO}: {err}")
        })?;
        Ok(release.version)
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
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
            "https://{VSIX_PUBLISHER}.gallery.vsassets.io/_apis/public/gallery/publisher/{VSIX_PUBLISHER}/extension/{VSIX_EXTENSION}/{version}/assetbyname/Microsoft.VisualStudio.Services.VSIXPackage"
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
        let _ = mark_checked_once(INSTALL_PATH, version);

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );

        self.cached_path = Some(version_dir.clone());
        Ok(version_dir)
    }
}
