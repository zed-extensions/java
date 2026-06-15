use std::{fs::metadata, path::PathBuf};

use serde_json::Value;
use zed_extension_api::{
    self as zed, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, Worktree, serde_json,
    set_language_server_installation_status,
};

use crate::util::{mark_checked_once, should_use_local_or_download};

pub(crate) const PROXY_INSTALL_PATH: &str = "proxy-bin";
pub(crate) const GITHUB_REPO: &str = "zed-extensions/java";

pub(crate) fn asset_name(binary: &str) -> zed::Result<(String, DownloadedFileType)> {
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
    Ok((format!("{binary}-{os_str}-{arch_str}.{ext}"), file_type))
}

pub(crate) fn binary_exec(binary: &str) -> String {
    let (os, _arch) = zed::current_platform();

    match os {
        zed::Os::Linux | zed::Os::Mac => binary.to_string(),
        zed::Os::Windows => format!("{binary}.exe"),
    }
}

pub(crate) fn find_latest_local(binary: &str) -> Option<PathBuf> {
    let exec = binary_exec(binary);
    let local_binary = PathBuf::from(PROXY_INSTALL_PATH).join(&exec);
    if metadata(&local_binary).is_ok_and(|m| m.is_file()) {
        return Some(local_binary);
    }

    // Check versioned downloads (e.g. proxy-bin/v6.8.12/java-lsp-proxy)
    std::fs::read_dir(PROXY_INSTALL_PATH)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join(&exec))
        .filter(|p| metadata(p).is_ok_and(|m| m.is_file()))
        .last()
}

pub(crate) fn download_binary(
    cached: &mut Option<String>,
    configuration: &Option<Value>,
    language_server_id: &LanguageServerId,
    worktree: &Worktree,
    binary: &str,
) -> zed::Result<String> {
    // 1. Respect check_updates setting (Never/Once/Always)
    match should_use_local_or_download(configuration, find_latest_local(binary), PROXY_INSTALL_PATH)
    {
        Ok(Some(path)) => {
            let s = path.to_string_lossy().to_string();
            *cached = Some(s.clone());
            return Ok(s);
        }
        Ok(None) => { /* policy allows download, continue */ }
        Err(_) => {
            // Never/Once with no managed install — fall through to PATH as last resort
        }
    }

    // 2. Auto-download from GitHub releases
    if let Ok((name, file_type)) = asset_name(binary)
        && let Ok(release) = zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )
    {
        let bin_path = format!(
            "{}/{}/{}",
            PROXY_INSTALL_PATH,
            release.version,
            binary_exec(binary)
        );

        if metadata(&bin_path).is_ok() {
            *cached = Some(bin_path.clone());
            return Ok(bin_path);
        }

        if let Some(asset) = release.assets.iter().find(|a| a.name == name) {
            let version_dir = format!("{PROXY_INSTALL_PATH}/{}", release.version);

            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );

            if zed::download_file(&asset.download_url, &version_dir, file_type).is_ok() {
                let _ = zed::make_file_executable(&bin_path);
                set_language_server_installation_status(
                    language_server_id,
                    &LanguageServerInstallationStatus::None,
                );
                // Do not remove other files if we are downloading one of multiple binaries
                // but for now they are in the same version dir.
                // let _ = remove_all_files_except(PROXY_INSTALL_PATH, &release.version);
                let _ = mark_checked_once(PROXY_INSTALL_PATH, &release.version);
                *cached = Some(bin_path.clone());
                return Ok(bin_path);
            }
        }
    }

    // 3. Fallback: local install (covers "always" mode when download fails)
    if let Some(path) = find_latest_local(binary) {
        let s = path.to_string_lossy().to_string();
        *cached = Some(s.clone());
        return Ok(s);
    }

    // 4. Fallback: binary on $PATH
    if let Some(path) = worktree.which(binary_exec(binary).as_str()) {
        return Ok(path);
    }

    // 5. Stale cache fallback
    if let Some(path) = cached.as_deref()
        && metadata(path).is_ok()
    {
        return Ok(path.to_string());
    }

    Err(format!("'{binary}' not found"))
}
