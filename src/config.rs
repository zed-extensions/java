use std::path::Path;

use zed_extension_api::{Os, Worktree, current_platform, serde_json::Value};

use crate::util::expand_home_path;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum CheckUpdates {
    #[default]
    Always,
    Once,
    Never,
}

pub fn get_java_home(configuration: &Option<Value>, worktree: &Worktree) -> Option<String> {
    // try to read the value from settings
    if let Some(configuration) = configuration
        && let Some(java_home) = configuration
            .pointer("/java_home")
            .or_else(|| configuration.pointer("/java/home")) // legacy support
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, java_home.to_string()) {
            Ok(home_path) => return Some(home_path),
            Err(err) => {
                println!("{err}");
            }
        };
    }

    // try to read the value from env
    match worktree
        .shell_env()
        .into_iter()
        .find(|(k, _)| k == "JAVA_HOME")
    {
        Some((_, value)) if !value.is_empty() => Some(value),
        _ => None,
    }
}

fn configured_jdtls_data_directory(configuration: &Option<Value>) -> Result<Option<&str>, String> {
    let Some(value) = configuration
        .as_ref()
        .and_then(|configuration| configuration.pointer("/data_directory"))
    else {
        return Ok(None);
    };

    let path = value
        .as_str()
        .ok_or_else(|| "JDTLS data_directory must be a string".to_string())?;
    if path.trim().is_empty() {
        return Err("JDTLS data_directory must not be empty".to_string());
    }

    Ok(Some(path))
}

/// macOS and Linux rely on [`Path::is_absolute`].
///
/// Windows requires custom logic to recognize:
/// - Drive paths: C:\... or C:/...
/// - UNC paths: \\server\share\... or //server/share/..
fn is_absolute_data_directory(path: &str, os: Os) -> bool {
    match os {
        Os::Windows => {
            let bytes = path.as_bytes();
            path.starts_with(r"\\")
                || path.starts_with("//")
                || (bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'/' | b'\\'))
        }
        Os::Mac | Os::Linux => Path::new(path).is_absolute(),
    }
}

fn validate_jdtls_data_directory(path: String, os: Os) -> Result<String, String> {
    if is_absolute_data_directory(&path, os) {
        Ok(path)
    } else {
        Err(format!(
            "JDTLS data_directory must be an absolute path: {path}"
        ))
    }
}

/// Returns the parent directory where per-worktree JDTLS data directories are stored.
pub fn get_jdtls_data_directory(
    configuration: &Option<Value>,
    worktree: &Worktree,
) -> Result<Option<String>, String> {
    let Some(data_directory) = configured_jdtls_data_directory(configuration)? else {
        return Ok(None);
    };
    let path = expand_home_path(worktree, data_directory.to_string())
        .map_err(|err| format!("Failed to expand JDTLS data_directory: {err}"))?;

    validate_jdtls_data_directory(path, current_platform().0).map(Some)
}

pub fn is_java_autodownload(configuration: &Option<Value>) -> bool {
    configuration
        .as_ref()
        .and_then(|configuration| {
            configuration
                .pointer("/jdk_auto_download")
                .and_then(|enabled| enabled.as_bool())
        })
        .unwrap_or(false)
}

pub fn is_lombok_enabled(configuration: &Option<Value>) -> bool {
    configuration
        .as_ref()
        .and_then(|configuration| {
            configuration
                .pointer("/lombok_support")
                .or_else(|| configuration.pointer("/java/jdt/ls/lombokSupport/enabled")) // legacy support
                .and_then(|enabled| enabled.as_bool())
        })
        .unwrap_or(true)
}

pub fn get_check_updates(configuration: &Option<Value>) -> CheckUpdates {
    if let Some(configuration) = configuration
        && let Some(mode_str) = configuration
            .pointer("/check_updates")
            .and_then(|x| x.as_str())
            .map(|s| s.to_lowercase())
    {
        return match mode_str.as_str() {
            "once" => CheckUpdates::Once,
            "never" => CheckUpdates::Never,
            "always" => CheckUpdates::Always,
            _ => CheckUpdates::default(),
        };
    }
    CheckUpdates::default()
}

pub fn get_jdtls_launcher(configuration: &Option<Value>, worktree: &Worktree) -> Option<String> {
    if let Some(configuration) = configuration
        && let Some(launcher_path) = configuration
            .pointer("/jdtls_launcher")
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, launcher_path.to_string()) {
            Ok(path) => return Some(path),
            Err(err) => {
                println!("{err}");
            }
        }
    }

    None
}

/// Returns the max heap size for jdtls (e.g. "2G", "4096m").
/// Maps to the `-Xmx` JVM argument.
pub fn get_max_memory(configuration: &Option<Value>) -> Option<String> {
    configuration
        .as_ref()
        .and_then(|c| c.pointer("/max_memory").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// Returns the initial heap size for jdtls (e.g. "512m", "1G").
/// Maps to the `-Xms` JVM argument. Defaults to "1G".
pub fn get_min_memory(configuration: &Option<Value>) -> Option<String> {
    configuration
        .as_ref()
        .and_then(|c| c.pointer("/min_memory").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

pub fn get_lombok_jar(configuration: &Option<Value>, worktree: &Worktree) -> Option<String> {
    if let Some(configuration) = configuration
        && let Some(jar_path) = configuration
            .pointer("/lombok_jar")
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, jar_path.to_string()) {
            Ok(path) => return Some(path),
            Err(err) => {
                println!("{err}");
            }
        }
    }

    None
}

pub fn get_java_debug_jar(configuration: &Option<Value>, worktree: &Worktree) -> Option<String> {
    if let Some(configuration) = configuration
        && let Some(jar_path) = configuration
            .pointer("/java_debug_jar")
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, jar_path.to_string()) {
            Ok(path) => return Some(path),
            Err(err) => {
                println!("{err}");
            }
        }
    }

    None
}

pub fn get_lsp_proxy_path(configuration: &Option<Value>, worktree: &Worktree) -> Option<String> {
    if let Some(configuration) = configuration
        && let Some(lsp_proxy_path) = configuration
            .pointer("/lsp_proxy_path")
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, lsp_proxy_path.to_string()) {
            Ok(path) => return Some(path),
            Err(err) => {
                println!("{err}");
            }
        }
    }

    None
}

/// Path to a local `gradle-lsp-bridge` binary, overriding the downloaded one.
/// Parallel to [`get_lsp_proxy_path`] for the JDTLS proxy; primarily used to
/// test a locally-built bridge before a release ships the asset.
pub fn get_gradle_bridge_path(
    configuration: &Option<Value>,
    worktree: &Worktree,
) -> Option<String> {
    if let Some(configuration) = configuration
        && let Some(bridge_path) = configuration
            .pointer("/gradle_bridge_path")
            .and_then(|x| x.as_str())
    {
        match expand_home_path(worktree, bridge_path.to_string()) {
            Ok(path) => return Some(path),
            Err(err) => {
                println!("{err}");
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use zed_extension_api::serde_json::json;

    use zed_extension_api::Os;

    use super::{
        configured_jdtls_data_directory, is_absolute_data_directory, validate_jdtls_data_directory,
    };

    #[test]
    fn configured_data_directory_distinguishes_absent_configuration() {
        assert_eq!(configured_jdtls_data_directory(&None), Ok(None));
        assert_eq!(configured_jdtls_data_directory(&Some(json!({}))), Ok(None));
    }

    #[test]
    fn configured_data_directory_accepts_non_empty_strings() {
        let configuration = Some(json!({ "data_directory": "/tmp/jdtls" }));

        assert_eq!(
            configured_jdtls_data_directory(&configuration),
            Ok(Some("/tmp/jdtls"))
        );
    }

    #[test]
    fn configured_data_directory_rejects_empty_values() {
        let empty_error =
            configured_jdtls_data_directory(&Some(json!({ "data_directory": "" }))).unwrap_err();
        let whitespace_error =
            configured_jdtls_data_directory(&Some(json!({ "data_directory": "  " }))).unwrap_err();

        assert_eq!(empty_error, "JDTLS data_directory must not be empty");
        assert_eq!(whitespace_error, "JDTLS data_directory must not be empty");
    }

    #[test]
    fn configured_data_directory_rejects_non_string_values() {
        let error =
            configured_jdtls_data_directory(&Some(json!({ "data_directory": true }))).unwrap_err();

        assert_eq!(error, "JDTLS data_directory must be a string");
    }

    #[test]
    fn data_directory_validation_rejects_relative_paths() {
        assert_eq!(
            validate_jdtls_data_directory("tmp/jdtls".to_string(), Os::Linux),
            Err("JDTLS data_directory must be an absolute path: tmp/jdtls".to_string())
        );
        assert_eq!(
            validate_jdtls_data_directory(r"Opt\zed-jdtls".to_string(), Os::Windows),
            Err(r"JDTLS data_directory must be an absolute path: Opt\zed-jdtls".to_string())
        );
    }

    #[test]
    fn data_directory_requires_platform_absolute_paths() {
        assert!(is_absolute_data_directory("/tmp/jdtls", Os::Linux));
        assert!(!is_absolute_data_directory("tmp/jdtls", Os::Linux));
        assert!(is_absolute_data_directory(r"C:\Opt\zed-jdtls", Os::Windows));
        assert!(is_absolute_data_directory("C:/Opt/zed-jdtls", Os::Windows));
        assert!(is_absolute_data_directory(
            r"\\server\share\zed-jdtls",
            Os::Windows
        ));
        assert!(!is_absolute_data_directory(r"Opt\zed-jdtls", Os::Windows));
    }
}
