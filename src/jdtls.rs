use std::{
    env::current_dir,
    fs::{metadata, read_dir},
    path::{Path, PathBuf},
};

use sha1::{Digest, Sha1};
use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, LanguageServerId,
    LanguageServerInstallationStatus, Os, Worktree, current_platform, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    make_file_executable,
    serde_json::Value,
    set_language_server_installation_status,
};

use crate::{
    component::Component,
    config::{get_lombok_jar, is_java_autodownload},
    jdk::Jdk,
    util::{
        create_path_if_not_exists, get_curr_dir, get_java_exec_name, get_java_executable,
        get_java_major_version, get_latest_versions_from_tag, mark_checked_once, path_to_string,
        remove_all_files_except,
    },
};

const JDTLS_INSTALL_PATH: &str = "jdtls";
const JDTLS_REPO: &str = "eclipse-jdtls/eclipse.jdt.ls";
const LOMBOK_INSTALL_PATH: &str = "lombok";
const LOMBOK_REPO: &str = "projectlombok/lombok";

const JAVA_VERSION_ERROR: &str = "JDTLS requires at least Java version 21 to run. You can either specify a different JDK to use by configuring lsp.jdtls.settings.java_home to point to a different JDK, or set lsp.jdtls.settings.jdk_auto_download to true to let the extension automatically download one for you.";
const JDTLS_VERSION_ERROR: &str = "No version to fallback to";

// --- Jdtls Component ---

pub struct Jdtls {
    cached_path: Option<PathBuf>,
}

impl Jdtls {
    pub fn new() -> Self {
        Self { cached_path: None }
    }
}

impl Component for Jdtls {
    const INSTALL_PATH: &'static str = JDTLS_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
        read_dir(&prefix)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
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

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        let (last, _) = get_latest_versions_from_tag(JDTLS_REPO)
            .map_err(|err| format!("Failed to fetch JDTLS versions from {JDTLS_REPO}: {err}"))?;
        Ok(last)
    }

    fn download(
        &mut self,
        _version: &str,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let (last, second_last) = get_latest_versions_from_tag(JDTLS_REPO)
            .map_err(|err| format!("Failed to fetch JDTLS versions from {JDTLS_REPO}: {err}"))?;

        let (latest_version, latest_version_build) = download_jdtls_milestone(last.as_ref())
            .map_or_else(
                |_| {
                    second_last
                        .ok_or(JDTLS_VERSION_ERROR.to_string())
                        .and_then(|fallback| {
                            download_jdtls_milestone(&fallback)
                                .map(|milestone| (fallback, milestone.trim_end().to_string()))
                        })
                },
                |milestone| Ok((last, milestone.trim_end().to_string())),
            )?;

        let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
        let build_directory = latest_version_build.replace(".tar.gz", "");
        let build_path = prefix.join(&build_directory);
        let binary_path = build_path.join("bin").join(get_binary_name());

        if !metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            let download_url = format!(
                "https://www.eclipse.org/downloads/download.php?file=/jdtls/milestones/{latest_version}/{latest_version_build}"
            );
            download_file(
                &download_url,
                path_to_string(build_path.clone())
                    .map_err(|err| format!("Invalid JDTLS build path {build_path:?}: {err}"))?
                    .as_str(),
                DownloadedFileType::GzipTar,
            )
            .map_err(|err| format!("Failed to download JDTLS from {download_url}: {err}"))?;
            make_file_executable(
                path_to_string(&binary_path)
                    .map_err(|err| format!("Invalid JDTLS binary path {binary_path:?}: {err}"))?
                    .as_str(),
            )
            .map_err(|err| format!("Failed to make JDTLS executable at {binary_path:?}: {err}"))?;

            let _ = remove_all_files_except(prefix, build_directory.as_str());
            let _ = mark_checked_once(JDTLS_INSTALL_PATH, &latest_version);
        }

        self.cached_path = Some(build_path.clone());
        Ok(build_path)
    }
}

// --- Lombok Component ---

pub struct Lombok {
    cached_path: Option<PathBuf>,
}

impl Lombok {
    pub fn new() -> Self {
        Self { cached_path: None }
    }
}

impl Component for Lombok {
    const INSTALL_PATH: &'static str = LOMBOK_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let prefix = PathBuf::from(LOMBOK_INSTALL_PATH);
        read_dir(&prefix)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.is_file()
                            && path.extension().and_then(|ext| ext.to_str()) == Some("jar")
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

    fn loaded(&self) -> bool {
        self.cached_path.is_some()
    }

    fn fetch_latest_version(&self) -> zed::Result<String> {
        let (latest_version, _) = get_latest_versions_from_tag(LOMBOK_REPO)
            .map_err(|err| format!("Failed to fetch Lombok versions from {LOMBOK_REPO}: {err}"))?;
        Ok(latest_version)
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = LOMBOK_INSTALL_PATH;
        let jar_name = format!("lombok-{version}.jar");
        let jar_path = Path::new(prefix).join(&jar_name);

        if !metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            create_path_if_not_exists(prefix)
                .map_err(|err| format!("Failed to create Lombok directory '{prefix}': {err}"))?;
            let download_url = format!("https://projectlombok.org/downloads/{jar_name}");
            download_file(
                &download_url,
                path_to_string(jar_path.clone())
                    .map_err(|err| format!("Invalid Lombok jar path {jar_path:?}: {err}"))?
                    .as_str(),
                DownloadedFileType::Uncompressed,
            )
            .map_err(|err| format!("Failed to download Lombok from {download_url}: {err}"))?;

            let _ = remove_all_files_except(prefix, jar_name.as_str());
            let _ = mark_checked_once(LOMBOK_INSTALL_PATH, version);
        }

        self.cached_path = Some(jar_path.clone());
        Ok(jar_path)
    }

    fn user_configured_path(
        &self,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> Option<String> {
        get_lombok_jar(configuration, worktree)
    }
}

// --- JDTLS launch utilities ---

/// Parse a JVM memory string (e.g. "2G", "512m", "1024k") into bytes.
fn parse_memory_value(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, multiplier) = match s.as_bytes().last()? {
        b'g' | b'G' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        b'm' | b'M' => (&s[..s.len() - 1], 1024 * 1024),
        b'k' | b'K' => (&s[..s.len() - 1], 1024),
        _ => (s, 1),
    };
    num.parse::<u64>().ok().map(|n| n * multiplier)
}

pub fn build_jdtls_launch_args(
    jdtls_path: &PathBuf,
    configuration: &Option<Value>,
    worktree: &Worktree,
    jvm_args: Vec<String>,
    language_server_id: &LanguageServerId,
    jdk: &mut Jdk,
) -> zed::Result<Vec<String>> {
    if let Some(jdtls_launcher) = get_jdtls_launcher_from_path(worktree) {
        return Ok(vec![jdtls_launcher]);
    }

    let mut java_executable = get_java_executable(configuration, worktree, language_server_id)
        .map_err(|err| format!("Failed to locate Java executable for JDTLS: {err}"))?;
    let java_major_version = get_java_major_version(&java_executable)
        .map_err(|err| format!("Failed to determine Java version: {err}"))?;
    if java_major_version < 21 {
        if is_java_autodownload(configuration) {
            java_executable = jdk
                .get_bin_path(language_server_id, configuration, worktree)
                .map_err(|err| format!("Failed to auto-download JDK for JDTLS: {err}"))?
                .join(get_java_exec_name());
        } else {
            return Err(JAVA_VERSION_ERROR.to_string());
        }
    }

    let extension_workdir = get_curr_dir()
        .map_err(|err| format!("Failed to get extension working directory: {err}"))?;

    let jdtls_base_path = extension_workdir.join(jdtls_path);

    let shared_config_path = get_shared_config_path(&jdtls_base_path);
    let jar_path = find_equinox_launcher(&jdtls_base_path).map_err(|err| {
        format!("Failed to find JDTLS equinox launcher in {jdtls_base_path:?}: {err}")
    })?;
    let jdtls_data_path = get_jdtls_data_path(worktree)
        .map_err(|err| format!("Failed to determine JDTLS data path: {err}"))?;

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
        "-Djava.import.generatesMetadataFilesAtProjectRoot=false".to_string(),
    ];
    {
        let mut min =
            crate::config::get_min_memory(configuration).unwrap_or_else(|| "1G".to_string());
        let mut max = crate::config::get_max_memory(configuration);
        if let Some(ref max_val) = max
            && let (Some(min_bytes), Some(max_bytes)) =
                (parse_memory_value(&min), parse_memory_value(max_val))
            && min_bytes > max_bytes
            && let Some(max) = max.as_mut()
        {
            std::mem::swap(&mut min, max);
        }
        args.push(format!("-Xms{min}"));
        if let Some(max_val) = max {
            args.push(format!("-Xmx{max_val}"));
        }
    }
    args.extend(vec![
        "--add-modules=ALL-SYSTEM".to_string(),
        "--add-opens".to_string(),
        "java.base/java.util=ALL-UNNAMED".to_string(),
        "--add-opens".to_string(),
        "java.base/java.lang=ALL-UNNAMED".to_string(),
    ]);
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

pub fn get_jdtls_launcher_from_path(worktree: &Worktree) -> Option<String> {
    let jdtls_executable_filename = match current_platform().0 {
        Os::Windows => "jdtls.bat",
        _ => "jdtls",
    };

    worktree.which(jdtls_executable_filename)
}

fn download_jdtls_milestone(version: &str) -> zed::Result<String> {
    String::from_utf8(
        fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(format!(
                    "https://download.eclipse.org/jdtls/milestones/{version}/latest.txt"
                ))
                .build()?,
        )
        .map_err(|err| format!("Failed to get latest version's build: {err}"))?
        .body,
    )
    .map_err(|err| format!("Failed to get latest version's build (malformed response): {err}"))
}

fn find_equinox_launcher(jdtls_base_directory: &Path) -> Result<PathBuf, String> {
    let plugins_dir = jdtls_base_directory.join("plugins");

    let specific_launcher = plugins_dir.join("org.eclipse.equinox.launcher.jar");
    if specific_launcher.is_file() {
        return Ok(specific_launcher);
    }

    let entries =
        read_dir(&plugins_dir).map_err(|err| format!("Failed to read plugins directory: {err}"))?;

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
    let env = worktree.shell_env();
    let base_cachedir = match current_platform().0 {
        Os::Mac => env
            .iter()
            .find(|(k, _)| k == "XDG_CACHE_HOME")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| {
                env.iter()
                    .find(|(k, _)| k == "HOME")
                    .map(|(_, v)| PathBuf::from(v).join("Library").join("Caches"))
            }),
        Os::Linux => env
            .iter()
            .find(|(k, _)| k == "XDG_CACHE_HOME")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| {
                env.iter()
                    .find(|(k, _)| k == "HOME")
                    .map(|(_, v)| PathBuf::from(v).join(".cache"))
            }),
        Os::Windows => env
            .iter()
            .find(|(k, _)| k == "APPDATA")
            .map(|(_, v)| PathBuf::from(v)),
    }
    .map(Ok)
    .unwrap_or_else(|| {
        current_dir()
            .map_err(|err| format!("Failed to get current directory: {err}"))
            .map(|path| path.join("caches"))
    })?;

    let cache_key = worktree.root_path();
    let hex_digest = get_sha1_hex(&cache_key);
    let unique_dir_name = format!("jdtls-{hex_digest}");
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
    let config_to_use = match current_platform() {
        (Os::Mac, Architecture::Aarch64) => "config_mac_arm",
        (Os::Mac, _) => "config_mac",
        (Os::Linux, Architecture::Aarch64) => "config_linux_arm",
        (Os::Linux, _) => "config_linux",
        (Os::Windows, _) => "config_win",
    };
    jdtls_base_directory.join(config_to_use)
}
