use regex::Regex;
use std::{
    env::current_dir,
    fs,
    path::{Path, PathBuf},
};
use zed_extension_api::{
    self as zed, Command, LanguageServerId, Os, Worktree, current_platform,
    http_client::{HttpMethod, HttpRequest, fetch},
    serde_json::Value,
};

use crate::{
    config::{CheckUpdates, get_check_updates, get_java_home, is_java_autodownload},
    jdk::try_to_fetch_and_install_latest_jdk,
};

// Errors
const EXPAND_ERROR: &str = "Failed to expand ~";
const CURR_DIR_ERROR: &str = "Could not get current dir";
const DIR_ENTRY_LOAD_ERROR: &str = "Failed to load directory entry";
const DIR_ENTRY_RM_ERROR: &str = "Failed to remove directory entry";
const ENTRY_TYPE_ERROR: &str = "Could not determine entry type";
const FILE_ENTRY_RM_ERROR: &str = "Failed to remove file entry";
const PATH_TO_STR_ERROR: &str = "Failed to convert path to string";
const JAVA_EXEC_ERROR: &str = "Failed to convert Java executable path to string";
const JAVA_VERSION_ERROR: &str = "Failed to determine Java major version";
const JAVA_EXEC_NOT_FOUND_ERROR: &str = "Could not find Java executable in JAVA_HOME or on PATH";
const TAG_RETRIEVAL_ERROR: &str = "Failed to fetch GitHub tags";
const TAG_RESPONSE_ERROR: &str = "Failed to deserialize GitHub tags response";
const TAG_UNEXPECTED_FORMAT_ERROR: &str = "Malformed GitHub tags response";
const PATH_IS_NOT_DIR: &str = "File exists but is not a path";
const NO_LOCAL_INSTALL_NEVER_ERROR: &str =
    "Update checks disabled (never) and no local installation found";
const NO_LOCAL_INSTALL_ONCE_ERROR: &str =
    "Update check already performed once and no local installation found";

const ONCE_CHECK_MARKER: &str = ".update_checked";

/// Create a Path if it does not exist
///
/// **Errors** if a file that is not a path exists at the location or read/write access failed for the location
///
///# Arguments
/// * [`path`] the path to create
///
///# Returns
///
/// Ok(()) if the path exists or was created successfully
pub fn create_path_if_not_exists<P: AsRef<Path>>(path: P) -> zed::Result<()> {
    let path_ref = path.as_ref();
    match fs::metadata(path_ref) {
        Ok(metadata) => {
            if metadata.is_dir() {
                Ok(())
            } else {
                Err(format!("{PATH_IS_NOT_DIR}: {path_ref:?}"))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path_ref).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Check if update check has been performed once for a component
///
/// # Arguments
///
/// * [`component_name`] - The component directory name (e.g., "jdtls", "lombok")
///
/// # Returns
///
/// Returns true if the marker file exists, indicating a check was already performed
pub fn has_checked_once(component_name: &str) -> bool {
    PathBuf::from(component_name)
        .join(ONCE_CHECK_MARKER)
        .exists()
}

/// Mark that an update check has been performed for a component
///
/// # Arguments
///
/// * [`component_name`] - The component directory name (e.g., "jdtls", "lombok")
/// * [`version`] - The version that was downloaded
///
/// # Returns
///
/// Returns Ok(()) if the marker was created successfully
///
/// # Errors
///
/// Returns an error if the directory or marker file could not be created
pub fn mark_checked_once(component_name: &str, version: &str) -> zed::Result<()> {
    let marker_path = PathBuf::from(component_name).join(ONCE_CHECK_MARKER);
    create_path_if_not_exists(PathBuf::from(component_name))
        .map_err(|err| format!("Failed to create directory for {component_name}: {err}"))?;
    fs::write(&marker_path, version)
        .map_err(|err| format!("Failed to write marker file {marker_path:?}: {err}"))
}

/// Expand ~ on Unix-like systems
///
/// # Arguments
///
/// * [`worktree`] Zed extension worktree with access to ENV
/// * [`path`] path to expand
///
/// # Returns
///
/// On Unix-like systems ~ is replaced with the value stored in HOME
///
/// On Windows systems [`path`] is returned untouched
pub fn expand_home_path(worktree: &Worktree, path: String) -> zed::Result<String> {
    match zed::current_platform() {
        (Os::Windows, _) => Ok(path),
        (_, _) => worktree
            .shell_env()
            .iter()
            .find(|&(key, _)| key == "HOME")
            .map_or_else(
                || Err(EXPAND_ERROR.to_string()),
                |(_, value)| Ok(path.replace("~", value)),
            ),
    }
}

/// Get the extension current directory
///
/// # Returns
///
/// The [`PathBuf`] of the extension directory
///
/// # Errors
///
/// This functoin will return an error if it was not possible to retrieve the current directory
pub fn get_curr_dir() -> zed::Result<PathBuf> {
    current_dir().map_err(|_| CURR_DIR_ERROR.to_string())
}

/// Retrieve the path to a java exec either:
/// - defined by the user in `settings.json` under option `java_home`
/// - from PATH
/// - from JAVA_HOME
/// - from the bundled OpenJDK if option `jdk_auto_download` is true
///
/// # Arguments
///
/// * [`configuration`] a JSON object representing the user configuration
/// * [`worktree`] Zed extension worktree
///
/// # Returns
///
/// Returns the path to the java exec file
///
/// # Errors
///
/// This function will return an error if neither PATH or JAVA_HOME led
/// to a java exec file
pub fn get_java_executable(
    configuration: &Option<Value>,
    worktree: &Worktree,
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    let java_executable_filename = get_java_exec_name();

    // Get executable from $JAVA_HOME
    if let Some(java_home) = get_java_home(configuration, worktree) {
        let java_executable = PathBuf::from(java_home)
            .join("bin")
            .join(java_executable_filename);
        return Ok(java_executable);
    }
    // If we can't, try to get it from $PATH
    if let Some(java_home) = worktree.which(java_executable_filename.as_str()) {
        return Ok(PathBuf::from(java_home));
    }

    // If the user has set the option, retrieve the latest version of Corretto (OpenJDK)
    if is_java_autodownload(configuration) {
        return Ok(
            try_to_fetch_and_install_latest_jdk(language_server_id, configuration)?
                .join(java_executable_filename),
        );
    }

    Err(JAVA_EXEC_NOT_FOUND_ERROR.to_string())
}

/// Retrieve the executable name for Java on this platform
///
/// # Returns
///
/// Returns the executable java name
pub fn get_java_exec_name() -> String {
    match current_platform().0 {
        Os::Windows => "java.exe".to_string(),
        _ => "java".to_string(),
    }
}

/// Retrieve the java major version accessible by the extension
///
/// # Arguments
///
/// * [`java_executable`] the path to a java exec file
///
/// # Returns
///
/// Returns the java major version
///
/// # Errors
///
/// This function will return an error if:
///
/// * [`java_executable`] can't be converted into a String
/// * No major version can be determined
pub fn get_java_major_version(java_executable: &PathBuf) -> zed::Result<u32> {
    let program = path_to_string(java_executable)
        .map_err(|err| format!("{JAVA_EXEC_ERROR} '{java_executable:?}': {err}"))?;
    let output_bytes = Command::new(&program)
        .arg("-version")
        .output()
        .map_err(|err| format!("Failed to execute '{program} -version': {err}"))?
        .stderr;
    let output = String::from_utf8(output_bytes)
        .map_err(|err| format!("Invalid UTF-8 in java version output: {err}"))?;

    let major_version_regex = Regex::new(r#"version\s"(?P<major>\d+)(\.\d+\.\d+(_\d+)?)?"#)
        .map_err(|err| format!("Invalid regex for Java version parsing: {err}"))?;
    let major_version = major_version_regex
        .captures_iter(&output)
        .find_map(|c| c.name("major").and_then(|m| m.as_str().parse::<u32>().ok()));

    if let Some(major_version) = major_version {
        Ok(major_version)
    } else {
        Err(JAVA_VERSION_ERROR.to_string())
    }
}

/// Retrieve the latest and second latest versions from the repo tags
///
/// # Arguments
///
/// * [`repo`] The GitHub repository from which to retrieve the tags
///
/// # Returns
///
/// A tuple containing the latest version, and optionally, the second latest version if available
///
/// # Errors
///
/// This function will return an error if:
/// * Could not fetch tags from Github
/// * Failed to deserialize response
/// * Unexpected Github response format
pub fn get_latest_versions_from_tag(repo: &str) -> zed::Result<(String, Option<String>)> {
    let tags_response_body = serde_json::from_slice::<Value>(
        &fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(format!("https://api.github.com/repos/{repo}/tags"))
                .build()?,
        )
        .map_err(|err| format!("{TAG_RETRIEVAL_ERROR}: {err}"))?
        .body,
    )
    .map_err(|err| format!("{TAG_RESPONSE_ERROR}: {err}"))?;

    let latest_version = get_tag_at(&tags_response_body, 0);
    let second_version = get_tag_at(&tags_response_body, 1);

    if latest_version.is_none() {
        return Err(TAG_UNEXPECTED_FORMAT_ERROR.to_string());
    }

    Ok((
        latest_version.unwrap().to_string(),
        second_version.map(|second| second.to_string()),
    ))
}

fn get_tag_at(github_tags: &Value, index: usize) -> Option<&str> {
    github_tags.as_array().and_then(|tag| {
        tag.get(index).and_then(|latest_tag| {
            latest_tag
                .get("name")
                .and_then(|tag_name| tag_name.as_str())
                .map(|val| &val[1..])
        })
    })
}

/// Converts a [`Path`] into a [`String`].
///
/// # Arguments
///
/// * `path` - The path of type `AsRef<Path>` to be converted.
///
/// # Returns
///
/// Returns a `String` representing the path.
///
/// # Errors
///
/// This function will return an error when the string conversion fails.
pub fn path_to_string<P: AsRef<Path>>(path: P) -> zed::Result<String> {
    path.as_ref()
        .to_path_buf()
        .into_os_string()
        .into_string()
        .map_err(|_| PATH_TO_STR_ERROR.to_string())
}

/// Remove all files or directories that aren't equal to [`filename`].
///
/// This function scans the directory given by [`prefix`] and removes any
/// file or directory whose name does not exactly match [`filename`].
///
/// # Arguments
///
/// * [`prefix`] - The path to the directory to clean. See [`AsRef<Path>`] for supported types.
/// * [`filename`] - The name of the file to keep.
///
/// # Returns
///
/// Returns `Ok(())` on success, even if some removals fail (errors are printed to stdout).
pub fn remove_all_files_except<P: AsRef<Path>>(prefix: P, filename: &str) -> zed::Result<()> {
    let entries: Vec<_> = match fs::read_dir(prefix) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(err) => {
            println!("{DIR_ENTRY_LOAD_ERROR}: {err}");
            return Err(format!("{DIR_ENTRY_LOAD_ERROR}: {err}"));
        }
    };

    for entry in entries {
        if entry.file_name().to_str() != Some(filename) {
            match entry.file_type() {
                Ok(t) => {
                    if t.is_dir()
                        && let Err(err) = fs::remove_dir_all(entry.path())
                    {
                        println!("{DIR_ENTRY_RM_ERROR}: {err}");
                    } else if t.is_file()
                        && let Err(err) = fs::remove_file(entry.path())
                    {
                        println!("{FILE_ENTRY_RM_ERROR}: {err}");
                    }
                }
                Err(type_err) => println!("{ENTRY_TYPE_ERROR}: {type_err}"),
            }
        }
    }

    Ok(())
}

/// Determine whether to use local component or download based on update mode
///
/// This function handles the common logic for all components (JDTLS, Lombok, Debugger):
/// 1. Apply update check mode (Never/Once/Always)
/// 2. Find local installation if applicable
///
/// # Arguments
/// * `configuration` - User configuration JSON
/// * `local` - Optional path to local installation
/// * `component_name` - Component name for error messages (e.g., "jdtls", "lombok", "debugger")
///
/// # Returns
/// * `Ok(Some(PathBuf))` - Local installation should be used
/// * `Ok(None)` - Should download
/// * `Err(String)` - Error message if resolution failed
///
/// # Errors
/// - Update mode is Never but no local installation found
/// - Update mode is Once and already checked but no local installation found
pub fn should_use_local_or_download(
    configuration: &Option<Value>,
    local: Option<PathBuf>,
    component_name: &str,
) -> zed::Result<Option<PathBuf>> {
    match get_check_updates(configuration) {
        CheckUpdates::Never => match local {
            Some(path) => Ok(Some(path)),
            None => Err(format!(
                "{NO_LOCAL_INSTALL_NEVER_ERROR} for {component_name}"
            )),
        },
        CheckUpdates::Once => {
            // If we have a local installation, use it
            if let Some(path) = local {
                return Ok(Some(path));
            }

            // If we've already checked once, don't check again
            if has_checked_once(component_name) {
                return Err(format!(
                    "{NO_LOCAL_INSTALL_ONCE_ERROR} for {component_name}"
                ));
            }

            // First time checking - allow download
            Ok(None)
        }
        CheckUpdates::Always => Ok(None),
    }
}
