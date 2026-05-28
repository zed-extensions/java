use zed_extension_api::{Worktree, serde_json::Value};

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
