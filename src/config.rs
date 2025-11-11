use zed_extension_api::{Worktree, serde_json::Value};

use crate::util::expand_home_path;

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
                println!("{}", err);
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

pub fn is_check_updates_enabled(configuration: &Option<Value>) -> bool {
    configuration
        .as_ref()
        .and_then(|configuration| {
            configuration
                .pointer("/check_updates_on_startup")
                .and_then(|enabled| enabled.as_bool())
        })
        .unwrap_or(true)
}

pub fn get_jdtls_download_url(configuration: &Option<Value>) -> Option<String> {
    configuration.as_ref().and_then(|configuration| {
        configuration
            .pointer("/jdtls_download_url")
            .and_then(|url| url.as_str())
            .map(|s| s.to_string())
    })
}

pub fn get_lombok_download_url(configuration: &Option<Value>) -> Option<String> {
    configuration.as_ref().and_then(|configuration| {
        configuration
            .pointer("/lombok_download_url")
            .and_then(|url| url.as_str())
            .map(|s| s.to_string())
    })
}

pub fn get_debugger_download_url(configuration: &Option<Value>) -> Option<String> {
    configuration.as_ref().and_then(|configuration| {
        configuration
            .pointer("/debugger_download_url")
            .and_then(|url| url.as_str())
            .map(|s| s.to_string())
    })
}
