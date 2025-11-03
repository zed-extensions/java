use std::path::{Path, PathBuf};

use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, LanguageServerId,
    LanguageServerInstallationStatus, Os, current_platform, download_file,
    set_language_server_installation_status,
};

use crate::util::{get_curr_dir, path_to_string, remove_all_files_except};

// Errors
const JDK_DIR_ERROR: &str = "Failed to read into JDK install directory";
const NO_JDK_DIR_ERROR: &str = "No match for jdk or corretto in the extracted directory";

const CORRETTO_REPO: &str = "corretto/corretto-25";
const CORRETTO_UNIX_URL_TEMPLATE: &str = "https://corretto.aws/downloads/resources/{version}/amazon-corretto-{version}-{platform}-{arch}.tar.gz";
const CORRETTO_WINDOWS_URL_TEMPLATE: &str = "https://corretto.aws/downloads/resources/{version}/amazon-corretto-{version}-{platform}-{arch}-jdk.zip";

fn build_corretto_url(version: &str, platform: &str, arch: &str) -> String {
    let template = match zed::current_platform().0 {
        Os::Windows => CORRETTO_WINDOWS_URL_TEMPLATE,
        _ => CORRETTO_UNIX_URL_TEMPLATE,
    };

    template
        .replace("{version}", version)
        .replace("{platform}", platform)
        .replace("{arch}", arch)
}

// For now keep in this file as they are not used anywhere else
// otherwise move to util
fn get_architecture() -> zed::Result<String> {
    match zed::current_platform() {
        (_, Architecture::Aarch64) => Ok("aarch64".to_string()),
        (_, Architecture::X86) => Ok("x86".to_string()),
        (_, Architecture::X8664) => Ok("x64".to_string()),
    }
}

fn get_platform() -> zed::Result<String> {
    match zed::current_platform() {
        (Os::Mac, _) => Ok("macosx".to_string()),
        (Os::Linux, _) => Ok("linux".to_string()),
        (Os::Windows, _) => Ok("windows".to_string()),
    }
}

pub fn try_to_fetch_and_install_latest_jdk(
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    let version = zed::latest_github_release(
        CORRETTO_REPO,
        zed_extension_api::GithubReleaseOptions {
            require_assets: false,
            pre_release: false,
        },
    )?
    .version;

    let jdk_path = get_curr_dir()?.join("jdk");
    let install_path = jdk_path.join(&version);

    // Check for updates, if same version is already downloaded skip download

    set_language_server_installation_status(
        language_server_id,
        &LanguageServerInstallationStatus::CheckingForUpdate,
    );

    if !install_path.exists() {
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        let platform = get_platform()?;
        let arch = get_architecture()?;

        download_file(
            build_corretto_url(&version, &platform, &arch).as_str(),
            path_to_string(install_path.clone())?.as_str(),
            match zed::current_platform().0 {
                Os::Windows => DownloadedFileType::Zip,
                _ => DownloadedFileType::GzipTar,
            },
        )?;

        // Remove older versions
        let _ = remove_all_files_except(jdk_path, version.as_str());
    }

    // Depending on the platform the name of the extracted dir might differ
    // Rather than hard coding, extract it dynamically
    let extracted_dir = get_extracted_dir(&install_path)?;

    Ok(install_path
        .join(extracted_dir)
        .join(match current_platform().0 {
            Os::Mac => "Contents/Home/bin",
            _ => "bin",
        }))
}

fn get_extracted_dir(path: &Path) -> zed::Result<String> {
    let Ok(mut entries) = path.read_dir() else {
        return Err(JDK_DIR_ERROR.to_string());
    };

    match entries.find_map(|entry| {
        let entry = entry.ok()?;
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy().to_string();

        if name_str.contains("jdk") || name_str.contains("corretto") {
            Some(name_str)
        } else {
            None
        }
    }) {
        Some(dir_path) => Ok(dir_path),
        None => Err(NO_JDK_DIR_ERROR.to_string()),
    }
}
