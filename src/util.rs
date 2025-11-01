use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
};
use zed_extension_api::{self as zed, Command, Os, Worktree, current_platform, serde_json::Value};

use crate::config::get_java_home;

// Errors
const EXPAND_ERROR: &str = "Failed to expand ~";
const DIR_ENTRY_LOAD_ERROR: &str = "Failed to load directory entry";
const DIR_ENTRY_RM_ERROR: &str = "Failed to remove directory entry";
const DIR_ENTRY_LS_ERROR: &str = "Failed to list prefix directory";
const PATH_TO_STR_ERROR: &str = "Failed to convert path to string";
const JAVA_EXEC_ERROR: &str = "Failed to convert Java executable path to string";
const JAVA_VERSION_ERROR: &str = "Failed to determine Java major version";
const JAVA_EXEC_NOT_FOUND_ERROR: &str = "Could not find Java executable in JAVA_HOME or on PATH";

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
    match fs::read_dir(prefix) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        if entry.file_name().to_str() != Some(filename)
                            && let Err(err) = fs::remove_dir_all(entry.path())
                        {
                            println!("{msg}: {err}", msg = DIR_ENTRY_RM_ERROR, err = err);
                        }
                    }
                    Err(err) => println!("{msg}: {err}", msg = DIR_ENTRY_LOAD_ERROR, err = err),
                }
            }
        }
        Err(err) => println!("{msg}: {err}", msg = DIR_ENTRY_LS_ERROR, err = err),
    }

    Ok(())
}

/// Retrieve the path to a java exec either:
/// - defined by the user in `settings.json`
/// - from PATH
/// - from JAVA_HOME
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
) -> zed::Result<PathBuf> {
    let java_executable_filename = match current_platform().0 {
        Os::Windows => "java.exe",
        _ => "java",
    };

    // Get executable from $JAVA_HOME
    if let Some(java_home) = get_java_home(configuration, worktree) {
        let java_executable = PathBuf::from(java_home)
            .join("bin")
            .join(java_executable_filename);
        return Ok(java_executable);
    }
    // If we can't, try to get it from $PATH
    worktree
        .which(java_executable_filename)
        .map(PathBuf::from)
        .ok_or_else(|| JAVA_EXEC_NOT_FOUND_ERROR.to_string())
}

pub fn path_to_string<P: AsRef<Path>>(path: P) -> zed::Result<String> {
    path.as_ref()
        .to_path_buf()
        .into_os_string()
        .into_string()
        .map_err(|_| PATH_TO_STR_ERROR.to_string())
}

pub fn get_java_major_version(java_executable: &PathBuf) -> zed::Result<u32> {
    let program = path_to_string(java_executable).map_err(|_| JAVA_EXEC_ERROR.to_string())?;
    let output_bytes = Command::new(program).arg("-version").output()?.stderr;
    let output = String::from_utf8(output_bytes).map_err(|e| e.to_string())?;

    let major_version_regex =
        Regex::new(r#"version\s"(?P<major>\d+)(\.\d+\.\d+(_\d+)?)?"#).map_err(|e| e.to_string())?;
    let major_version = major_version_regex
        .captures_iter(&output)
        .find_map(|c| c.name("major").and_then(|m| m.as_str().parse::<u32>().ok()));

    if let Some(major_version) = major_version {
        Ok(major_version)
    } else {
        Err(JAVA_VERSION_ERROR.to_string())
    }
}
