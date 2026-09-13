use std::{
    env::current_dir,
    fs::{self, metadata, read_dir},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
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
    config::{get_jdtls_data_directory, get_lombok_jar, is_java_autodownload},
    downloadable::Downloadable,
    jdk::Jdk,
    util::{
        create_path_if_not_exists, get_curr_dir, get_java_exec_name, get_java_executable,
        get_java_major_version, get_latest_versions_from_tag, path_to_string,
        remove_all_files_except,
    },
};

const JDTLS_INSTALL_PATH: &str = "jdtls";
const JDTLS_REPO: &str = "eclipse-jdtls/eclipse.jdt.ls";
const LOMBOK_INSTALL_PATH: &str = "lombok";
const LOMBOK_REPO: &str = "projectlombok/lombok";

const JAVA_VERSION_ERROR: &str = "JDTLS requires at least Java version 21 to run. You can either specify a different JDK to use by configuring lsp.jdtls.settings.java_home to point to a different JDK, or set lsp.jdtls.settings.jdk_auto_download to true to let the extension automatically download one for you.";
// --- Jdtls Component ---

pub struct Jdtls {
    cached_path: Option<PathBuf>,
}

impl Jdtls {
    pub fn new() -> Self {
        Self { cached_path: None }
    }
}

impl Downloadable for Jdtls {
    const INSTALL_PATH: &'static str = JDTLS_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
        let binary_name = get_binary_name();
        let config_directory = get_shared_config_directory_name();
        read_dir(&prefix)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .filter(|path| is_complete_jdtls_install(path, binary_name, config_directory))
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

    fn fetch_latest_version(&self, worktree: &Worktree) -> zed::Result<String> {
        let (last, _) = get_latest_versions_from_tag(JDTLS_REPO, worktree)
            .map_err(|err| format!("Failed to fetch JDTLS versions from {JDTLS_REPO}: {err}"))?;
        Ok(last)
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
        let binary_name = get_binary_name();
        let config_directory = get_shared_config_directory_name();
        if let Some(build_path) =
            installed_jdtls_version(&prefix, version, binary_name, config_directory)
        {
            self.cached_path = Some(build_path.clone());
            return Ok(build_path);
        }

        let build_path = prefix.join(version);

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        let latest_version_build = download_jdtls_milestone(version)?.trim_end().to_string();

        let download_url = format!(
            "https://www.eclipse.org/downloads/download.php?file=/jdtls/milestones/{version}/{latest_version_build}"
        );
        let staging_path = jdtls_staging_path(&prefix, version)?;
        let staging_binary_path = jdtls_binary_path(&staging_path, binary_name);
        let staged = (|| {
            download_file(
                &download_url,
                path_to_string(staging_path.clone())
                    .map_err(|err| format!("Invalid JDTLS staging path {staging_path:?}: {err}"))?
                    .as_str(),
                DownloadedFileType::GzipTar,
            )
            .map_err(|err| format!("Failed to download JDTLS from {download_url}: {err}"))?;
            make_file_executable(
                path_to_string(&staging_binary_path)
                    .map_err(|err| {
                        format!("Invalid JDTLS binary path {staging_binary_path:?}: {err}")
                    })?
                    .as_str(),
            )
            .map_err(|err| {
                format!("Failed to make JDTLS executable at {staging_binary_path:?}: {err}")
            })?;

            promote_staged_jdtls(&staging_path, &build_path, binary_name, config_directory)
        })();
        if let Err(err) = staged {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(err);
        }

        let _ = remove_old_jdtls_installations(&prefix, version);

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

impl Downloadable for Lombok {
    const INSTALL_PATH: &'static str = LOMBOK_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        let prefix = get_curr_dir().ok()?.join(LOMBOK_INSTALL_PATH);
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

    fn fetch_latest_version(&self, worktree: &Worktree) -> zed::Result<String> {
        let (latest_version, _) = get_latest_versions_from_tag(LOMBOK_REPO, worktree)
            .map_err(|err| format!("Failed to fetch Lombok versions from {LOMBOK_REPO}: {err}"))?;
        Ok(latest_version)
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        let prefix = get_curr_dir()
            .map_err(|err| format!("Failed to get extension directory for Lombok: {err}"))?
            .join(LOMBOK_INSTALL_PATH);
        let jar_name = format!("lombok-{version}.jar");
        let jar_path = prefix.join(&jar_name);

        if !metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            create_path_if_not_exists(&prefix)
                .map_err(|err| format!("Failed to create Lombok directory '{prefix:?}': {err}"))?;
            let download_url = format!("https://projectlombok.org/downloads/{jar_name}");
            download_file(
                &download_url,
                path_to_string(jar_path.clone())
                    .map_err(|err| format!("Invalid Lombok jar path {jar_path:?}: {err}"))?
                    .as_str(),
                DownloadedFileType::Uncompressed,
            )
            .map_err(|err| format!("Failed to download Lombok from {download_url}: {err}"))?;
            let _ = remove_all_files_except(&prefix, jar_name.as_str());
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
    jdtls_path: &Path,
    jdtls_data_path: &Path,
    configuration: &Option<Value>,
    worktree: &Worktree,
    jvm_args: Vec<String>,
    language_server_id: &LanguageServerId,
    jdk: &mut Jdk,
) -> zed::Result<Vec<String>> {
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
        "-Djava.net.preferIPv4Stack=true".to_string(),
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
    args.extend(vec!["-jar".to_string(), path_to_string(jar_path)?]);
    append_jdtls_data_args(&mut args, jdtls_data_path)?;
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

/// Return the launcher path expected inside an extracted JDTLS installation.
fn jdtls_binary_path(install_path: &Path, binary_name: &str) -> PathBuf {
    install_path.join("bin").join(binary_name)
}

/// Return the tag-specific installation path only when all required JDTLS files are present.
fn installed_jdtls_version(
    prefix: &Path,
    version: &str,
    binary_name: &str,
    config_directory: &str,
) -> Option<PathBuf> {
    let install_path = prefix.join(version);
    is_complete_jdtls_install(&install_path, binary_name, config_directory).then_some(install_path)
}

/// Validate the launcher, platform configuration, and Equinox launcher needed to start JDTLS.
fn is_complete_jdtls_install(
    install_path: &Path,
    binary_name: &str,
    config_directory: &str,
) -> bool {
    jdtls_binary_path(install_path, binary_name).is_file()
        && install_path.join(config_directory).is_dir()
        && find_equinox_launcher(install_path).is_ok()
}

/// Select an unused hidden sibling directory for extracting a JDTLS archive
/// before it is promoted to the tag-specific installation directory.
fn jdtls_staging_path(prefix: &Path, version: &str) -> zed::Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("System clock is before the Unix epoch: {err}"))?
        .as_nanos();
    let version_hash = get_sha1_hex(version);

    (0..16)
        .map(|attempt| prefix.join(format!(".{version_hash}.{nonce}.{attempt}.tmp")))
        .find(|path| !path.exists())
        .ok_or_else(|| format!("Failed to allocate a JDTLS staging directory for {version}"))
}

/// Validate and atomically promote a staged JDTLS installation.
///
/// A concurrently completed installation wins; its redundant staging directory
/// is removed instead of replacing the valid final directory.
fn promote_staged_jdtls(
    staging_path: &Path,
    install_path: &Path,
    binary_name: &str,
    config_directory: &str,
) -> zed::Result<()> {
    if !is_complete_jdtls_install(staging_path, binary_name, config_directory) {
        return Err(format!(
            "Downloaded JDTLS installation is incomplete at {staging_path:?}"
        ));
    }

    if is_complete_jdtls_install(install_path, binary_name, config_directory) {
        fs::remove_dir_all(staging_path)
            .map_err(|err| format!("Failed to remove redundant JDTLS staging directory: {err}"))?;
        return Ok(());
    }

    if install_path.exists() {
        fs::remove_dir_all(install_path)
            .map_err(|err| format!("Failed to remove incomplete JDTLS installation: {err}"))?;
    }

    match fs::rename(staging_path, install_path) {
        Ok(()) => Ok(()),
        Err(_) if is_complete_jdtls_install(install_path, binary_name, config_directory) => {
            fs::remove_dir_all(staging_path)
                .map_err(|err| format!("Failed to remove redundant JDTLS staging directory: {err}"))
        }
        Err(err) => Err(format!(
            "Failed to promote JDTLS staging directory {staging_path:?}: {err}"
        )),
    }
}

/// Remove older completed JDTLS version directories while preserving the
/// selected version, update records, and hidden in-progress staging directories.
fn remove_old_jdtls_installations(prefix: &Path, version: &str) -> zed::Result<()> {
    let entries = fs::read_dir(prefix)
        .map_err(|err| format!("Failed to read JDTLS install directory {prefix:?}: {err}"))?;

    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name != version
            && !file_name.starts_with('.')
            && entry.file_type().is_ok_and(|file_type| file_type.is_dir())
            && let Err(err) = fs::remove_dir_all(entry.path())
        {
            println!("Failed to remove old JDTLS installation: {err}");
        }
    }

    Ok(())
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

pub fn get_configured_jdtls_data_path(
    configuration: &Option<Value>,
    worktree: &Worktree,
) -> zed::Result<Option<PathBuf>> {
    let base_directory = get_jdtls_data_directory(configuration, worktree)?;
    Ok(base_directory.map(|base_directory| {
        build_jdtls_data_path(Path::new(&base_directory), &worktree.root_path())
    }))
}

pub fn append_jdtls_data_args(args: &mut Vec<String>, data_path: &Path) -> zed::Result<()> {
    args.push("-data".to_string());
    args.push(path_to_string(data_path)?);
    Ok(())
}

pub fn get_default_jdtls_data_path(worktree: &Worktree) -> zed::Result<PathBuf> {
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

    Ok(build_jdtls_data_path(&base_cachedir, &worktree.root_path()))
}

fn build_jdtls_data_path(base_directory: &Path, cache_key: &str) -> PathBuf {
    let hex_digest = get_sha1_hex(cache_key);
    let unique_dir_name = format!("jdtls-{hex_digest}");
    base_directory.join(unique_dir_name)
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

/// Return the JDTLS configuration directory bundled for the current platform.
fn get_shared_config_directory_name() -> &'static str {
    match current_platform() {
        (Os::Mac, Architecture::Aarch64) => "config_mac_arm",
        (Os::Mac, _) => "config_mac",
        (Os::Linux, Architecture::Aarch64) => "config_linux_arm",
        (Os::Linux, _) => "config_linux",
        (Os::Windows, _) => "config_win",
    }
}

fn get_shared_config_path(jdtls_base_directory: &Path) -> PathBuf {
    jdtls_base_directory.join(get_shared_config_directory_name())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::util::{UPDATE_CHECK_MARKER, fresh_cached_version, record_successful_update_check};

    static NEXT_TEMP_PATH: AtomicU64 = AtomicU64::new(0);

    fn temporary_jdtls_prefix(test_name: &str) -> PathBuf {
        let id = NEXT_TEMP_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "zed-java-jdtls-{}-{id}-{test_name}",
            std::process::id()
        ))
    }

    fn create_complete_install(install_path: &Path) {
        let binary_path = jdtls_binary_path(install_path, "jdtls");
        fs::create_dir_all(binary_path.parent().unwrap()).unwrap();
        fs::write(binary_path, "").unwrap();
        fs::create_dir_all(install_path.join("config_linux")).unwrap();
        let launcher = install_path
            .join("plugins")
            .join("org.eclipse.equinox.launcher_1.0.0.jar");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        fs::write(launcher, "").unwrap();
    }

    #[test]
    fn incomplete_jdtls_install_is_not_reused() {
        let prefix = temporary_jdtls_prefix("incomplete");
        let version = "1.50.0";
        let install_path = prefix.join(version);

        let binary_path = jdtls_binary_path(&install_path, "jdtls");
        fs::create_dir_all(binary_path.parent().unwrap()).unwrap();
        fs::write(&binary_path, "").unwrap();

        assert_eq!(
            installed_jdtls_version(&prefix, version, "jdtls", "config_linux"),
            None
        );

        create_complete_install(&install_path);
        assert_eq!(
            installed_jdtls_version(&prefix, version, "jdtls", "config_linux"),
            Some(install_path)
        );

        let _ = fs::remove_dir_all(prefix);
    }

    #[test]
    fn complete_staging_install_replaces_incomplete_final_directory() {
        let prefix = temporary_jdtls_prefix("promotion");
        let install_path = prefix.join("1.50.0");
        let staging_path = prefix.join(".staging.tmp");
        fs::create_dir_all(&install_path).unwrap();
        fs::write(install_path.join("partial"), "").unwrap();
        create_complete_install(&staging_path);

        promote_staged_jdtls(&staging_path, &install_path, "jdtls", "config_linux").unwrap();

        assert!(!staging_path.exists());
        assert!(is_complete_jdtls_install(
            &install_path,
            "jdtls",
            "config_linux"
        ));
        assert!(!install_path.join("partial").exists());
        let _ = fs::remove_dir_all(prefix);
    }

    #[test]
    fn cached_tag_resolves_complete_install_without_build_metadata() {
        let prefix = temporary_jdtls_prefix("cache-flow");
        let version = "1.50.0";
        let update_check_path = prefix.join(UPDATE_CHECK_MARKER);
        record_successful_update_check(&update_check_path, version).unwrap();
        let cached_version = fresh_cached_version(&update_check_path).unwrap();
        let install_path = prefix.join(version);
        create_complete_install(&install_path);

        assert_eq!(cached_version, version);
        assert_eq!(
            installed_jdtls_version(&prefix, &cached_version, "jdtls", "config_linux"),
            Some(install_path)
        );
        let _ = fs::remove_dir_all(prefix);
    }

    #[test]
    fn jdtls_cleanup_preserves_active_staging_directories() {
        let prefix = temporary_jdtls_prefix("cleanup");
        let current = prefix.join("1.50.0");
        let old = prefix.join("1.49.0");
        let staging = prefix.join(".staging.tmp");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&staging).unwrap();

        remove_old_jdtls_installations(&prefix, "1.50.0").unwrap();

        assert!(current.exists());
        assert!(!old.exists());
        assert!(staging.exists());
        let _ = fs::remove_dir_all(prefix);
    }

    #[test]
    fn data_paths_are_stable_and_isolated_by_worktree() {
        let base_directory = Path::new("/tmp/custom-jdtls");
        let first = build_jdtls_data_path(base_directory, "/workspace/first");
        let first_again = build_jdtls_data_path(base_directory, "/workspace/first");
        let second = build_jdtls_data_path(base_directory, "/workspace/second");

        assert_eq!(first, first_again);
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(base_directory));
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("jdtls-"))
        );
    }

    #[test]
    fn data_path_arguments_are_appended_as_a_pair() {
        let mut args = vec!["jdtls".to_string()];
        let data_path = Path::new("/tmp/custom-jdtls/jdtls-workspace");

        append_jdtls_data_args(&mut args, data_path).unwrap();

        assert_eq!(
            args,
            vec![
                "jdtls".to_string(),
                "-data".to_string(),
                data_path.to_string_lossy().to_string()
            ]
        );
    }
}
