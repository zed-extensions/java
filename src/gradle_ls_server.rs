use std::{env, fs};

use zed_extension_api::{
    self as zed, LanguageServerId, Os, Worktree, current_platform,
    serde_json::{Value, json},
    settings::LspSettings,
};

use crate::{
    config::get_java_home,
    downloadable::Downloadable,
    gradle_bridge::GradleBridge,
    gradle_ls::GradleLs,
    language_server::LanguageServer,
    util::{get_java_executable, path_to_string},
};

pub struct GradleLsServer {
    pub gradle_ls: GradleLs,
    pub bridge: GradleBridge,
}

impl GradleLsServer {
    pub fn new() -> Self {
        Self {
            gradle_ls: GradleLs::new(),
            bridge: GradleBridge::new(),
        }
    }
}

impl LanguageServer for GradleLsServer {
    const SERVER_ID: &'static str = "gradle-language-server";

    fn command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        let configuration = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.settings);

        let current_dir =
            env::current_dir().map_err(|err| format!("Failed to get current directory: {err}"))?;

        let gradle_ls_path = self
            .gradle_ls
            .get_or_download(language_server_id, &configuration, worktree)
            .map_err(|err| format!("Failed to get Gradle Language Server: {err}"))?;

        let lib_path = current_dir.join(&gradle_ls_path).join("lib");
        let classpath = build_classpath(&lib_path)?;

        let bridge_path = self
            .bridge
            .binary_path(&configuration, language_server_id, worktree)
            .map_err(|err| format!("Failed to get gradle-lsp-bridge binary path: {err}"))?;

        let java_executable = get_java_executable(&configuration, worktree, language_server_id)
            .map_err(|err| format!("Failed to locate Java executable: {err}"))?;

        let java_home = get_java_home(&configuration, worktree);

        let mut env = Vec::new();
        if let Some(java_home) = &java_home {
            env.push(("JAVA_HOME".to_string(), java_home.clone()));
        }

        // Forward Gradle distribution settings to the bridge (read from the
        // process environment, threaded into the gRPC GradleConfig the bridge
        // sends to gradle-server). Mirrors the knobs the VS Code gradle-server
        // applies to its Tooling API connection. Sourced from the LSP `settings`
        // block (the single config source); init options are left empty.
        env.extend(gradle_config_env(&configuration, java_home.as_deref()));

        let java_path = path_to_string(&java_executable)
            .map_err(|err| format!("Failed to convert Java path: {err}"))?;

        Ok(zed::Command {
            command: bridge_path,
            args: vec![
                java_path,
                "-cp".to_string(),
                classpath,
                "com.microsoft.gradle.GradleLanguageServer".to_string(),
            ],
            env,
        })
    }

    fn initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        let options = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options)
            .map_err(|err| format!("Failed to get LSP settings: {err}"))?
            .unwrap_or_else(|| json!({}));

        Ok(Some(options))
    }

    fn workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        Ok(
            LspSettings::for_worktree(language_server_id.as_ref(), worktree)
                .ok()
                .and_then(|lsp_settings| lsp_settings.settings),
        )
    }
}

/// Build the environment that conveys Gradle distribution settings to the
/// `gradle-lsp-bridge`. The keys mirror the language server's own settings
/// schema (`gradleUserHome`, `gradleVersion`, `gradleWrapperEnabled`,
/// `gradleHome`), read from the LSP `settings` block, and are mapped to the
/// `GRADLE_SYNC_*` variables the bridge reads into the gRPC `GradleConfig` it
/// sends to `gradle-server`. `gradle_jvm_arguments` (a string) and the resolved
/// JDK home are also forwarded if present.
fn gradle_config_env(
    configuration: &Option<Value>,
    java_home: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();

    // The JDK the bridge should ask gradle-server to build with. Threaded into
    // the gRPC GradleConfig's java_home; mirrors VS Code passing VSCODE_JAVA_HOME.
    if let Some(java_home) = java_home
        && !java_home.is_empty()
    {
        env.push(("GRADLE_SYNC_JAVA_HOME".to_string(), java_home.to_string()));
    }

    let Some(settings) = configuration else {
        return env;
    };

    let mut push_str = |key: &str, var: &str| {
        if let Some(value) = settings.get(key).and_then(|v| v.as_str())
            && !value.is_empty()
        {
            env.push((var.to_string(), value.to_string()));
        }
    };

    push_str("gradleUserHome", "GRADLE_SYNC_USER_HOME");
    push_str("gradleVersion", "GRADLE_SYNC_VERSION");
    push_str("gradleHome", "GRADLE_SYNC_GRADLE_HOME");
    push_str("gradle_jvm_arguments", "GRADLE_SYNC_JVM_ARGS");

    // Only forward the wrapper flag when explicitly disabled; the bridge treats
    // its absence as "wrapper enabled" (the default).
    if settings
        .get("gradleWrapperEnabled")
        .and_then(|v| v.as_bool())
        == Some(false)
    {
        env.push((
            "GRADLE_SYNC_WRAPPER_ENABLED".to_string(),
            "false".to_string(),
        ));
    }

    env
}

fn build_classpath(lib_path: &std::path::Path) -> zed::Result<String> {
    let separator = match current_platform().0 {
        Os::Windows => ";",
        _ => ":",
    };

    let entries: Vec<String> = fs::read_dir(lib_path)
        .map_err(|err| format!("Failed to read lib directory {}: {err}", lib_path.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "jar")
        })
        .map(|path| path.to_string_lossy().to_string())
        .collect();

    if entries.is_empty() {
        return Err(format!("No JAR files found in {}", lib_path.display()));
    }

    Ok(entries.join(separator))
}
