use std::{fs::metadata, path::PathBuf};

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Worktree,
    serde_json::Value, set_language_server_installation_status,
};

use crate::{
    config::get_lsp_proxy_path,
    downloadable::Downloadable,
    util::{
        NATIVE_BIN_DIR, mark_checked_once, platform_asset_name, platform_exec_name,
        remove_all_files_except, should_use_local_or_download,
    },
};

const PROXY_BINARY: &str = "java-lsp-proxy";
const PROXY_INSTALL_PATH: &str = NATIVE_BIN_DIR;
const GITHUB_REPO: &str = "zed-extensions/java";

pub struct Proxy {
    cached_path: Option<String>,
}

impl Proxy {
    pub fn new() -> Self {
        Self { cached_path: None }
    }

    pub fn binary_path(
        &mut self,
        configuration: &Option<Value>,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<String> {
        let path = self.get_or_download(language_server_id, configuration, worktree)?;
        Ok(path.to_string_lossy().to_string())
    }
}

impl Downloadable for Proxy {
    const INSTALL_PATH: &'static str = PROXY_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let local_binary = PathBuf::from(PROXY_INSTALL_PATH).join(proxy_exec());
        if metadata(&local_binary).is_ok_and(|m| m.is_file()) {
            return Some(local_binary);
        }

        std::fs::read_dir(PROXY_INSTALL_PATH)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path().join(proxy_exec()))
            .filter(|p| metadata(p).is_ok_and(|m| m.is_file()))
            .last()
    }

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        // The proxy is built and released together with the extension, so the
        // matching release is the one tagged with the extension's own version.
        Ok(format!("v{}", env!("CARGO_PKG_VERSION")))
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let (name, file_type) = asset_name()?;
        let bin_path = format!("{PROXY_INSTALL_PATH}/{version}/{}", proxy_exec());

        if metadata(&bin_path).is_ok() {
            self.cached_path = Some(bin_path.clone());
            return Ok(PathBuf::from(bin_path));
        }

        let release = zed::github_release_by_tag_name(GITHUB_REPO, version)
            .map_err(|err| format!("Failed to fetch proxy release {version}: {err}"))?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("No asset found matching {name:?}"))?;

        let version_dir = format!("{PROXY_INSTALL_PATH}/{version}");

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        zed::download_file(&asset.download_url, &version_dir, file_type)
            .map_err(|err| format!("Failed to download proxy: {err}"))?;

        let _ = zed::make_file_executable(&bin_path);
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );
        let _ = remove_all_files_except(PROXY_INSTALL_PATH, version);
        let _ = mark_checked_once(PROXY_INSTALL_PATH, version);

        self.cached_path = Some(bin_path.clone());
        Ok(PathBuf::from(bin_path))
    }

    fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        if let Some(path) = self.user_configured_path(configuration, worktree) {
            self.cached_path = Some(path.clone());
            return Ok(PathBuf::from(path));
        }

        if let Some(path) =
            should_use_local_or_download(configuration, self.find_local(), Self::INSTALL_PATH)
                .unwrap_or(None)
        {
            let s = path.to_string_lossy().to_string();
            self.cached_path = Some(s);
            return Ok(path);
        }

        let downloaded = self
            .fetch_latest_version()
            .and_then(|version| self.download(&version, language_server_id));

        let download_err = match downloaded {
            Ok(path) => return Ok(path),
            Err(err) => err,
        };

        // The version check or download failed (e.g. GitHub API rate
        // limiting) — an existing local installation is better than none.
        if let Some(path) = self.find_local() {
            println!("Failed to update proxy, falling back to local installation: {download_err}");
            let s = path.to_string_lossy().to_string();
            self.cached_path = Some(s);
            return Ok(path);
        }

        if let Some(path) = worktree.which(proxy_exec().as_str()) {
            return Ok(PathBuf::from(path));
        }

        Err(format!("'{}' not found: {download_err}", proxy_exec()))
    }

    fn user_configured_path(
        &self,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> Option<String> {
        get_lsp_proxy_path(configuration, worktree)
    }
}

fn asset_name() -> zed::Result<(String, DownloadedFileType)> {
    platform_asset_name(PROXY_BINARY)
}

fn proxy_exec() -> String {
    platform_exec_name(PROXY_BINARY)
}
