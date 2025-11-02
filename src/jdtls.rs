use std::{
    env::current_dir,
    fs::{create_dir, metadata, read_dir},
    path::{Path, PathBuf},
};

use sha1::{Digest, Sha1};
use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Os,
    Worktree, current_platform, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    make_file_executable,
    serde_json::Value,
    set_language_server_installation_status,
};

use crate::{
    config::is_java_autodownload,
    jdk::try_to_fetch_and_install_latest_jdk,
    util::{
        get_curr_dir, get_java_exec_name, get_java_executable, get_java_major_version,
        path_to_string, remove_all_files_except,
    },
};

const JDTLS_INSTALL_PATH: &str = "jdtls";
const JDTLS_REPO: &str = "";
const LOMBOK_INSTALL_PATH: &str = "lombok";
const LOMBOK_REPO: &str = "";

// Errors

const JAVA_VERSION_ERROR: &str = "JDTLS requires at least Java version 21 to run. You can either specify a different JDK to use by configuring lsp.jdtls.settings.java_home to point to a different JDK, or set lsp.jdtls.settings.jdk_auto_download to true to let the extension automatically download one for you.";

pub fn build_jdtls_launch_args(
    jdtls_path: &PathBuf,
    configuration: &Option<Value>,
    worktree: &Worktree,
    jvm_args: Vec<String>,
    language_server_id: &LanguageServerId,
) -> zed::Result<Vec<String>> {
    if let Some(jdtls_launcher) = get_jdtls_launcher_from_path(worktree) {
        return Ok(vec![jdtls_launcher]);
    }

    let mut java_executable = get_java_executable(configuration, worktree, language_server_id)?;
    let java_major_version = get_java_major_version(&java_executable)?;
    if java_major_version < 21 {
        if is_java_autodownload(configuration) {
            java_executable =
                try_to_fetch_and_install_latest_jdk(language_server_id)?.join(get_java_exec_name());
        } else {
            return Err(JAVA_VERSION_ERROR.to_string());
        }
    }

    let extension_workdir = get_curr_dir()?;

    let jdtls_base_path = extension_workdir.join(jdtls_path);

    let shared_config_path = get_shared_config_path(&jdtls_base_path);
    let jar_path = find_equinox_launcher(&jdtls_base_path)?;
    let jdtls_data_path = get_jdtls_data_path(worktree)?;

    let mut args = vec![
        path_to_string(java_executable)?,
        "-Declipse.application=org.eclipse.jdt.ls.core.id1".to_string(),
        "-Dosgi.bundles.defaultStartLevel=4".to_string(),
        "-Declipse.product=org.eclipse.jdt.ls.core.product".to_string(),
        "-Dosgi.checkConfiguration=true".to_string(),
        format!(
            "-Dosgi.sharedConfiguration.area={}",
            path_to_string(shared_config_path)?
        ),
        "-Dosgi.sharedConfiguration.area.readOnly=true".to_string(),
        "-Dosgi.configuration.cascaded=true".to_string(),
        "-Xms1G".to_string(),
        "--add-modules=ALL-SYSTEM".to_string(),
        "--add-opens".to_string(),
        "java.base/java.util=ALL-UNNAMED".to_string(),
        "--add-opens".to_string(),
        "java.base/java.lang=ALL-UNNAMED".to_string(),
    ];
    args.extend(jvm_args);
    args.extend(vec![
        "-jar".to_string(),
        path_to_string(jar_path)?,
        "-data".to_string(),
        path_to_string(jdtls_data_path)?,
    ]);
    if java_major_version >= 24 {
        args.push("-Djdk.xml.maxGeneralEntitySizeLimit=0".to_string());
        args.push("-Djdk.xml.totalEntitySizeLimit=0".to_string());
    }
    Ok(args)
}

pub fn find_latest_local_jdtls() -> Option<PathBuf> {
    let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
    // walk the dir where we install jdtls
    read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                // get the most recently created subdirectory
                .filter_map(|path| {
                    let created_time = metadata(&path).and_then(|meta| meta.created()).ok()?;
                    Some((path, created_time))
                })
                .max_by_key(|&(_, time)| time)
                // and return it
                .map(|(path, _)| path)
        })
        .ok()
        .flatten()
}

pub fn find_latest_local_lombok() -> Option<PathBuf> {
    let prefix = PathBuf::from(LOMBOK_INSTALL_PATH);
    // walk the dir where we install lombok
    read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                // get the most recently created jar file
                .filter(|path| {
                    path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jar")
                })
                .filter_map(|path| {
                    let created_time = metadata(&path).and_then(|meta| meta.created()).ok()?;
                    Some((path, created_time))
                })
                .max_by_key(|&(_, time)| time)
                .map(|(path, _)| path)
        })
        .ok()
        .flatten()
}

pub fn get_jdtls_launcher_from_path(worktree: &Worktree) -> Option<String> {
    let jdtls_executable_filename = match current_platform().0 {
        Os::Windows => "jdtls.bat",
        _ => "jdtls",
    };

    worktree.which(jdtls_executable_filename)
}

pub fn try_to_fetch_and_install_latest_jdtls(
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    let latest_version = get_latest_version_from_tag(JDTLS_REPO)?;

    let latest_version_build = String::from_utf8(
        fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(format!(
                    "https://download.eclipse.org/jdtls/milestones/{latest_version}/latest.txt"
                ))
                .build()?,
        )
        .map_err(|err| format!("failed to get latest version's build: {err}"))?
        .body,
    )
    .map_err(|err| {
        format!("attempt to get latest version's build resulted in a malformed response: {err}")
    })?;

    let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
    let build_directory = latest_version_build
        .replace("tar.gz", "")
        .trim()
        .to_string();
    let build_path = prefix.join(&build_directory);
    let binary_path = build_path.join("bin").join(get_binary_name());

    // If latest version isn't installed,
    if !metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
        // then download it...

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        download_file(
            &format!(
                "https://www.eclipse.org/downloads/download.php?file=/jdtls/milestones/{latest_version}/{latest_version_build}",
            ),
            path_to_string(build_path.clone())?.as_str(),
            DownloadedFileType::GzipTar,
        )?;
        make_file_executable(path_to_string(binary_path)?.as_str())?;

        // ...and delete other versions
        let _ = remove_all_files_except(prefix, build_directory.as_str());
    }

    // return jdtls base path
    Ok(build_path)
}

pub fn try_to_fetch_and_install_latest_lombok(
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    set_language_server_installation_status(
        language_server_id,
        &LanguageServerInstallationStatus::CheckingForUpdate,
    );

    let latest_version = get_latest_version_from_tag(LOMBOK_REPO)?;
    let prefix = LOMBOK_INSTALL_PATH;
    let jar_name = format!("lombok-{latest_version}.jar");
    let jar_path = Path::new(prefix).join(&jar_name);

    // If latest version isn't installed,
    if !metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
        // then download it...

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        create_dir(prefix).map_err(|err| err.to_string())?;
        download_file(
            &format!("https://projectlombok.org/downloads/{jar_name}"),
            path_to_string(jar_path.clone())?.as_str(),
            DownloadedFileType::Uncompressed,
        )?;

        // ...and delete other versions

        let _ = remove_all_files_except(prefix, jar_name.as_str());
    }

    // else use it
    Ok(jar_path)
}

fn find_equinox_launcher(jdtls_base_directory: &Path) -> Result<PathBuf, String> {
    let plugins_dir = jdtls_base_directory.join("plugins");

    // if we have `org.eclipse.equinox.launcher.jar` use that
    let specific_launcher = plugins_dir.join("org.eclipse.equinox.launcher.jar");
    if specific_launcher.is_file() {
        return Ok(specific_launcher);
    }

    // else get the first file that matches the glob 'org.eclipse.equinox.launcher_*.jar'
    let entries =
        read_dir(&plugins_dir).map_err(|e| format!("Failed to read plugins directory: {}", e))?;

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    s.starts_with("org.eclipse.equinox.launcher_") && s.ends_with(".jar")
                })
        })
        .ok_or_else(|| "Cannot find equinox launcher".to_string())
}

fn get_jdtls_data_path(worktree: &Worktree) -> zed::Result<PathBuf> {
    // Note: the JDTLS data path is where JDTLS stores its own caches.
    // In the unlikely event we can't find the canonical OS-Level cache-path,
    // we fall back to the the extension's workdir, which may never get cleaned up.
    // In future we may want to deliberately manage caches to be able to force-clean them.

    let mut env_iter = worktree.shell_env().into_iter();
    let base_cachedir = match current_platform().0 {
        Os::Mac => env_iter
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| PathBuf::from(v).join("Library").join("Caches")),
        Os::Linux => env_iter
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| PathBuf::from(v).join(".cache")),
        Os::Windows => env_iter
            .find(|(k, _)| k == "APPDATA")
            .map(|(_, v)| PathBuf::from(v)),
    }
    .unwrap_or_else(|| {
        current_dir()
            .expect("should be able to get extension workdir")
            .join("caches")
    });

    // caches are unique per worktree-root-path
    let cache_key = worktree.root_path();

    let hex_digest = get_sha1_hex(&cache_key);
    let unique_dir_name = format!("jdtls-{}", hex_digest);
    Ok(base_cachedir.join(unique_dir_name))
}

fn get_binary_name() -> &'static str {
    match current_platform().0 {
        Os::Windows => "jdtls.bat",
        _ => "jdtls",
    }
}

fn get_sha1_hex(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn get_shared_config_path(jdtls_base_directory: &Path) -> PathBuf {
    // Note: JDTLS also provides config_linux_arm and config_mac_arm (and others),
    // but does not use them in their own launch script. It may be worth investigating if we should use them when appropriate.
    let config_to_use = match current_platform().0 {
        Os::Linux => "config_linux",
        Os::Mac => "config_mac",
        Os::Windows => "config_win",
    };
    jdtls_base_directory.join(config_to_use)
}
