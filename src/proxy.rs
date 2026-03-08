use std::{fs::metadata, path::PathBuf};

use serde_json::Value;
use zed_extension_api::{
    self as zed, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, Worktree, serde_json,
    set_language_server_installation_status,
};

use crate::util::{mark_checked_once, remove_all_files_except, should_use_local_or_download};

const PROXY_BINARY: &str = "java-lsp-proxy";
const PROXY_INSTALL_PATH: &str = "proxy-bin";
const GITHUB_REPO: &str = "zed-extensions/java";

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
        format!("java-lsp-proxy-{os_str}-{arch_str}.{ext}"),
        file_type,
    ))
}

fn find_latest_local() -> Option<PathBuf> {
    let local_binary = PathBuf::from(PROXY_INSTALL_PATH).join(PROXY_BINARY);
    if metadata(&local_binary).is_ok_and(|m| m.is_file()) {
        return Some(local_binary);
    }

    // Check versioned downloads (e.g. proxy-bin/v6.8.12/java-lsp-proxy)
    std::fs::read_dir(PROXY_INSTALL_PATH)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join(PROXY_BINARY))
        .filter(|p| metadata(p).is_ok_and(|m| m.is_file()))
        .last()
}

pub fn binary_path(
    cached: &mut Option<String>,
    configuration: &Option<Value>,
    language_server_id: &LanguageServerId,
    worktree: &Worktree,
) -> zed::Result<String> {
    // 1. Respect check_updates setting (Never/Once/Always)
    //    Returns Some(path) when local install exists and policy says use it.
    //    Returns None when policy allows downloading.
    //    Returns Err when policy is Never/Once-exhausted with no local install.
    match should_use_local_or_download(configuration, find_latest_local(), PROXY_INSTALL_PATH) {
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
    if let Ok((name, file_type)) = asset_name() {
        if let Ok(release) = zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        ) {
            let bin_path = format!("{PROXY_INSTALL_PATH}/{}/java-lsp-proxy", release.version);

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
                    let _ = remove_all_files_except(PROXY_INSTALL_PATH, &release.version);
                    let _ = mark_checked_once(PROXY_INSTALL_PATH, &release.version);
                    *cached = Some(bin_path.clone());
                    return Ok(bin_path);
                }
            }
        }
    }

    // 3. Fallback: local install (covers "always" mode when download fails)
    if let Some(path) = find_latest_local() {
        let s = path.to_string_lossy().to_string();
        *cached = Some(s.clone());
        return Ok(s);
    }

    // 4. Fallback: binary on $PATH
    if let Some(path) = worktree.which(PROXY_BINARY) {
        return Ok(path);
    }

    // 5. Stale cache fallback
    if let Some(path) = cached.as_deref() {
        if metadata(path).is_ok() {
            return Ok(path.to_string());
        }
    }

    Err(format!("'{PROXY_BINARY}' not found"))
}
