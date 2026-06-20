use std::{fs::metadata, path::PathBuf};

use zed_extension_api::{
    self as zed, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, Worktree, serde_json::Value,
    set_language_server_installation_status,
};

use crate::{
    config::{get_gradle_bridge_path, get_lsp_proxy_path},
    downloadable::Downloadable,
    util::{mark_checked_once, remove_all_files_except, should_use_local_or_download},
};

const BRIDGE_BINARY: &str = "gradle-lsp-bridge";
const BRIDGE_INSTALL_PATH: &str = "gradle-bridge-bin";
/// The bridge ships as a separate per-platform asset on the same GitHub release
/// as the JDTLS proxy (the extension version's release tag).
const GITHUB_REPO: &str = "zed-extensions/java";

/// Downloads and locates the `gradle-lsp-bridge` binary — the native process
/// that bridges Zed to the Gradle Language Server and drives the real
/// `gradle-server.jar` over gRPC. Mirrors [`crate::proxy::Proxy`], but resolves
/// its own asset name so the two binaries can be downloaded independently from
/// the shared release.
pub struct GradleBridge {
    cached_path: Option<String>,
}

impl GradleBridge {
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

impl Downloadable for GradleBridge {
    const INSTALL_PATH: &'static str = BRIDGE_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let local_binary = PathBuf::from(BRIDGE_INSTALL_PATH).join(bridge_exec());
        if metadata(&local_binary).is_ok_and(|m| m.is_file()) {
            return Some(local_binary);
        }

        std::fs::read_dir(BRIDGE_INSTALL_PATH)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path().join(bridge_exec()))
            .filter(|p| metadata(p).is_ok_and(|m| m.is_file()))
            .last()
    }

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        Ok(zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )
        .map_err(|err| format!("Failed to fetch latest bridge release from {GITHUB_REPO}: {err}"))?
        .version)
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let (name, file_type) = asset_name()?;
        let bin_path = format!("{BRIDGE_INSTALL_PATH}/{version}/{}", bridge_exec());

        if metadata(&bin_path).is_ok() {
            self.cached_path = Some(bin_path.clone());
            return Ok(PathBuf::from(bin_path));
        }

        let release = zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )
        .map_err(|err| format!("Failed to fetch bridge release: {err}"))?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("No asset found matching {name:?}"))?;

        let version_dir = format!("{BRIDGE_INSTALL_PATH}/{version}");

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        zed::download_file(&asset.download_url, &version_dir, file_type)
            .map_err(|err| format!("Failed to download bridge: {err}"))?;

        let _ = zed::make_file_executable(&bin_path);
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );
        let _ = remove_all_files_except(BRIDGE_INSTALL_PATH, version);
        let _ = mark_checked_once(BRIDGE_INSTALL_PATH, version);

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

        if let Ok(version) = self.fetch_latest_version()
            && let Ok(path) = self.download(&version, language_server_id)
        {
            return Ok(path);
        }

        if let Some(path) = worktree.which(bridge_exec().as_str()) {
            return Ok(PathBuf::from(path));
        }

        Err(format!("'{}' not found", bridge_exec()))
    }

    fn user_configured_path(
        &self,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> Option<String> {
        // A dedicated override always wins.
        if let Some(path) = get_gradle_bridge_path(configuration, worktree) {
            return Some(path);
        }

        // Otherwise, if the user points `lsp_proxy_path` at a local build, look
        // for the bridge as a sibling — but only adopt it when it actually
        // exists, so users running a custom proxy without a co-located bridge
        // still fall through to the normal download path.
        let proxy_path = get_lsp_proxy_path(configuration, worktree)?;
        let p = PathBuf::from(&proxy_path);
        let dir = if p.is_dir() {
            p
        } else {
            p.parent().map(PathBuf::from).unwrap_or(p)
        };
        let sibling = dir.join(bridge_exec());
        metadata(&sibling)
            .is_ok_and(|m| m.is_file())
            .then(|| sibling.to_string_lossy().to_string())
    }
}

fn asset_name() -> zed::Result<(String, DownloadedFileType)> {
    let (os, arch) = zed::current_platform();
    let (os_str, file_type) = match os {
        zed::Os::Mac => ("darwin", DownloadedFileType::GzipTar),
        zed::Os::Linux => ("linux", DownloadedFileType::GzipTar),
        zed::Os::Windows => ("windows", DownloadedFileType::Zip),
    };
    let arch_str = match arch {
        zed::Architecture::Aarch64 => "aarch64",
        zed::Architecture::X8664 => "x86_64",
        _ => return Err("Unsupported architecture".into()),
    };
    let ext = if matches!(file_type, DownloadedFileType::Zip) {
        "zip"
    } else {
        "tar.gz"
    };
    Ok((
        format!("{BRIDGE_BINARY}-{os_str}-{arch_str}.{ext}"),
        file_type,
    ))
}

fn bridge_exec() -> String {
    let (os, _arch) = zed::current_platform();
    match os {
        zed::Os::Linux | zed::Os::Mac => BRIDGE_BINARY.to_string(),
        zed::Os::Windows => format!("{BRIDGE_BINARY}.exe"),
    }
}
