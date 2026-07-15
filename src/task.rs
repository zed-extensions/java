use std::{fs::metadata, path::PathBuf};

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Worktree,
    serde_json::Value, set_language_server_installation_status,
};

use crate::{
    downloadable::Downloadable,
    util::{mark_checked_once, remove_all_files_except, should_use_local_or_download},
};

const TASK_HELPER_BINARY: &str = "java-task-helper";
const TASK_HELPER_INSTALL_PATH: &str = "bin";
const GITHUB_REPO: &str = "zed-extensions/java";

pub struct TaskHelper {
    cached_path: Option<String>,
}

impl TaskHelper {
    pub fn new() -> Self {
        Self { cached_path: None }
    }
}

impl Downloadable for TaskHelper {
    const INSTALL_PATH: &'static str = TASK_HELPER_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let local_binary = PathBuf::from(TASK_HELPER_INSTALL_PATH).join(task_helper_exec());
        if metadata(&local_binary).is_ok_and(|m| m.is_file()) {
            return Some(local_binary);
        }

        std::fs::read_dir(TASK_HELPER_INSTALL_PATH)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path().join(task_helper_exec()))
            .filter(|p| metadata(p).is_ok_and(|m| m.is_file()))
            .last()
    }

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        // The task helper is built and released together with the extension, so
        // the matching release is the one tagged with the extension's own version.
        Ok(format!("v{}", env!("CARGO_PKG_VERSION")))
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let (name, file_type) = asset_name()?;
        let bin_path = format!(
            "{TASK_HELPER_INSTALL_PATH}/{version}/{}",
            task_helper_exec()
        );

        if metadata(&bin_path).is_ok() {
            self.cached_path = Some(bin_path.clone());
            return Ok(PathBuf::from(bin_path));
        }

        let release = zed::github_release_by_tag_name(GITHUB_REPO, version)
            .map_err(|err| format!("Failed to fetch task helper release {version}: {err}"))?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("No asset found matching {name:?}"))?;

        let version_dir = format!("{TASK_HELPER_INSTALL_PATH}/{version}");

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        zed::download_file(&asset.download_url, &version_dir, file_type)
            .map_err(|err| format!("Failed to download task helper: {err}"))?;

        let _ = zed::make_file_executable(&bin_path);
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );
        let _ = remove_all_files_except(TASK_HELPER_INSTALL_PATH, version);
        let _ = mark_checked_once(TASK_HELPER_INSTALL_PATH, version);

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
            println!(
                "Failed to update task helper, falling back to local installation: {download_err}"
            );
            let s = path.to_string_lossy().to_string();
            self.cached_path = Some(s);
            return Ok(path);
        }

        if let Some(path) = worktree.which(task_helper_exec().as_str()) {
            return Ok(PathBuf::from(path));
        }

        Err(format!("'{}' not found: {download_err}", task_helper_exec()))
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
        format!("{TASK_HELPER_BINARY}-{os_str}-{arch_str}.{ext}"),
        file_type,
    ))
}

fn task_helper_exec() -> String {
    let (os, _arch) = zed::current_platform();
    match os {
        zed::Os::Linux | zed::Os::Mac => TASK_HELPER_BINARY.to_string(),
        zed::Os::Windows => format!("{TASK_HELPER_BINARY}.exe"),
    }
}
