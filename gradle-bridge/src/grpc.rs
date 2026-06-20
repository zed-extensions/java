//! Drives the real shipped `gradle-server.jar` over gRPC, exactly as the VS Code
//! `vscode-gradle` extension does.
//!
//! A single long-lived `gradle-server` JVM is spawned once and kept alive for
//! the bridge's lifetime: it holds the Gradle Tooling-API connection open and
//! keeps the Gradle daemon warm, so re-syncs after the first are near-instant.
//! This is the whole point of driving the real server rather than forking a cold
//! JVM per save.
//!
//! The bridge calls the server-streaming `GetBuild` RPC, then maps the resulting
//! model into the `gradle.setPlugins` / `gradle.setClosures` /
//! `gradle.setScriptClasspaths` `executeCommand` arguments the Gradle Language
//! Server understands — the same forwarding the VS Code TypeScript client does.

use std::env;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tonic::transport::{Channel, Endpoint};

use crate::proto::gradle::{
    gradle_client::GradleClient, get_build_reply::Kind, output::OutputType, GetBuildRequest,
    GradleConfig, GradleProject,
};

/// The Gradle distribution configuration forwarded by the extension via
/// environment variables and threaded into the gRPC `GradleConfig`. Mirrors the
/// knobs the VS Code `gradle-server` honors.
#[derive(Clone, Default)]
pub struct DistributionConfig {
    pub gradle_user_home: String,
    pub gradle_home: String,
    pub version: String,
    pub jvm_arguments: String,
    pub java_home: String,
    pub wrapper_enabled: bool,
}

impl DistributionConfig {
    /// Read the configuration the extension exported as `GRADLE_SYNC_*` vars.
    /// Absence of `GRADLE_SYNC_WRAPPER_ENABLED=false` means wrapper-enabled (the
    /// default), matching the helper's previous behavior and the LS settings.
    pub fn from_env() -> Self {
        let wrapper_enabled = env::var("GRADLE_SYNC_WRAPPER_ENABLED")
            .map(|v| !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        Self {
            gradle_user_home: env_or_empty("GRADLE_SYNC_USER_HOME"),
            gradle_home: env_or_empty("GRADLE_SYNC_GRADLE_HOME"),
            version: env_or_empty("GRADLE_SYNC_VERSION"),
            jvm_arguments: env_or_empty("GRADLE_SYNC_JVM_ARGS"),
            java_home: env_or_empty("GRADLE_SYNC_JAVA_HOME"),
            wrapper_enabled,
        }
    }

    fn to_gradle_config(&self) -> GradleConfig {
        GradleConfig {
            gradle_home: self.gradle_home.clone(),
            user_home: self.gradle_user_home.clone(),
            jvm_arguments: self.jvm_arguments.clone(),
            wrapper_enabled: self.wrapper_enabled,
            version: self.version.clone(),
            // The Gradle Language Server checks the extension version against a
            // minimum; the value is otherwise opaque, so report a recent one.
            java_extension_version: String::new(),
            java_home: self.java_home.clone(),
        }
    }
}

fn env_or_empty(key: &str) -> String {
    env::var(key).unwrap_or_default()
}

/// The outcome of a `GetBuild` call: either the resolved root project model, or
/// a build-evaluation failure to surface as a diagnostic on the build file.
pub enum BuildOutcome {
    Model(GradleProject),
    /// `(error, causes)` — already flattened from the gRPC status / reply.
    Error { error: String, causes: Vec<String> },
}

/// Manages the long-lived `gradle-server` process and gRPC channel. Cloneable
/// (cheap `Arc` clone) so it can be shared with the sync worker.
#[derive(Clone)]
pub struct GradleServer {
    inner: Arc<Mutex<ServerState>>,
    java: String,
    classpath: String,
    java_home: Option<String>,
    config: DistributionConfig,
}

struct ServerState {
    /// The running server process + connected channel, if started.
    running: Option<RunningServer>,
}

struct RunningServer {
    child: Child,
    channel: Channel,
}

impl GradleServer {
    pub fn new(
        java: String,
        classpath: String,
        java_home: Option<String>,
        config: DistributionConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ServerState { running: None })),
            java,
            classpath,
            java_home,
            config,
        }
    }

    /// Run `GetBuild` for `project_dir`, starting the server on first use and
    /// reusing it thereafter. `cancellation_key` is echoed in the request so a
    /// superseding sync can cancel this build via [`Self::cancel`].
    pub async fn get_build(&self, project_dir: &str, cancellation_key: &str) -> BuildOutcome {
        let channel = match self.ensure_channel().await {
            Ok(c) => c,
            Err(e) => {
                return BuildOutcome::Error {
                    error: format!("Failed to start gradle-server: {e}"),
                    causes: Vec::new(),
                };
            }
        };

        let mut client = GradleClient::new(channel)
            // Multi-project models can exceed the 4 MB default; the VS Code
            // client sets this to unlimited.
            .max_decoding_message_size(usize::MAX);

        let request = GetBuildRequest {
            project_dir: project_dir.to_string(),
            cancellation_key: cancellation_key.to_string(),
            gradle_config: Some(self.config.to_gradle_config()),
            show_output_colors: false,
        };

        let mut stream = match client.get_build(request).await {
            Ok(resp) => resp.into_inner(),
            Err(status) => {
                return BuildOutcome::Error {
                    error: status.message().to_string(),
                    causes: Vec::new(),
                };
            }
        };

        // The gRPC error status carries only the outermost exception message
        // (`ErrorMessageBuilder` sets `Status.INTERNAL.withDescription(e.getMessage())`)
        // — e.g. "The supplied build action failed with an exception." The
        // actionable detail (the offending build file, line/column, and the root
        // cause) is what Gradle writes to standard error, which the server
        // streams back as `Output` messages with `output_type = STDERR`. We
        // accumulate that here and attach it to the failure so it reaches the
        // editor diagnostic, instead of discarding it.
        let mut model: Option<GradleProject> = None;
        let mut stderr = String::new();
        loop {
            match stream.message().await {
                Ok(Some(reply)) => match reply.kind {
                    Some(Kind::GetBuildResult(result)) => {
                        model = result.build.and_then(|b| b.project);
                    }
                    Some(Kind::CompatibilityCheckError(msg)) => {
                        return BuildOutcome::Error {
                            error: msg,
                            causes: stderr_causes(&stderr),
                        };
                    }
                    Some(Kind::Output(output)) => {
                        if output.output_type == OutputType::Stderr as i32 {
                            stderr.push_str(&String::from_utf8_lossy(&output.output_bytes));
                        }
                    }
                    // Progress/Environment/Cancelled are informational.
                    _ => {}
                },
                Ok(None) => break,
                Err(status) => {
                    return BuildOutcome::Error {
                        error: status.message().to_string(),
                        causes: stderr_causes(&stderr),
                    };
                }
            }
        }

        match model {
            Some(project) => BuildOutcome::Model(project),
            None => BuildOutcome::Error {
                error: "gradle-server returned no build model".to_string(),
                causes: stderr_causes(&stderr),
            },
        }
    }

    /// Cancel an in-flight build identified by `cancellation_key`. Best-effort:
    /// errors (including the server not running) are ignored.
    pub async fn cancel(&self, cancellation_key: &str) {
        let channel = {
            let state = self.inner.lock().await;
            state.running.as_ref().map(|r| r.channel.clone())
        };
        let Some(channel) = channel else {
            return;
        };
        let mut client = GradleClient::new(channel);
        let _ = client
            .cancel_build(crate::proto::gradle::CancelBuildRequest {
                cancellation_key: cancellation_key.to_string(),
            })
            .await;
    }

    /// Kill the server process if running. Called on bridge shutdown.
    pub async fn shutdown(&self) {
        let mut state = self.inner.lock().await;
        if let Some(mut running) = state.running.take() {
            let _ = running.child.start_kill();
        }
    }

    /// Ensure a connected channel exists, (re)starting the server if needed.
    async fn ensure_channel(&self) -> Result<Channel, String> {
        let mut state = self.inner.lock().await;

        // Reuse a healthy running server.
        if let Some(running) = state.running.as_mut() {
            // If the JVM died, drop it and restart below.
            match running.child.try_wait() {
                Ok(None) => return Ok(running.channel.clone()),
                _ => {
                    state.running = None;
                }
            }
        }

        let port = free_port().await?;
        let child = self.spawn_server(port)?;
        let channel = connect_with_retry(port).await?;
        state.running = Some(RunningServer { child, channel: channel.clone() });
        Ok(channel)
    }

    /// Spawn `java -cp <classpath> com.github.badsyntax.gradle.GradleServer <port>`.
    fn spawn_server(&self, port: u16) -> Result<Child, String> {
        let mut cmd = Command::new(&self.java);
        // GradleServer.main parses only `--key=value` args (Utils.parseArgs); a
        // bare positional port is ignored. `port` is required; `startBuildServer`
        // is also validated as required — we set it false because we only need
        // the gRPC build-model server, not the BSP build server (which would in
        // turn require `pipeName`/`bundleDir`). The LS pipe path is omitted: the
        // bridge launches and talks to the language server itself.
        cmd.args([
            "-Dfile.encoding=UTF-8",
            "-cp",
            &self.classpath,
            "com.github.badsyntax.gradle.GradleServer",
            &format!("--port={port}"),
            "--startBuildServer=false",
        ])
        .stdin(Stdio::null())
        // The server logs readiness to stderr; inherit so it lands in the
        // bridge's own stderr (Zed's language server log) for debugging.
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

        if let Some(home) = &self.java_home {
            // The Gradle start script honors VSCODE_JAVA_HOME; set JAVA_HOME too
            // so the directly-launched server uses the same JDK.
            cmd.env("JAVA_HOME", home);
            cmd.env("VSCODE_JAVA_HOME", home);
        }

        cmd.spawn()
            .map_err(|e| format!("failed to spawn gradle-server: {e}"))
    }
}

/// Turn the captured Gradle standard-error text into a list of cause lines.
///
/// The diagnostics builder joins these onto the top-level error message and
/// scans the combined text for Gradle's `build file '…': N:` and
/// `@ line N, column C` markers, so preserving the raw lines keeps both the
/// human-readable detail and the location parsing intact. Returns empty when no
/// stderr was captured (a successful build, or a failure that wrote nothing).
fn stderr_causes(stderr: &str) -> Vec<String> {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Pick a free TCP port on the loopback interface. We bind, read the assigned
/// port, then drop the listener so the JVM can bind it. (A brief race window
/// exists, but the loopback ephemeral range makes a collision very unlikely; the
/// connect-retry below also absorbs a transient failure.)
async fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to reserve a port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to read reserved port: {e}"))?
        .port();
    Ok(port)
}

/// Connect a plaintext h2c channel to the server, retrying while the JVM starts
/// up and binds its port. Mirrors the VS Code client's `waitForReady` deadline.
async fn connect_with_retry(port: u16) -> Result<Channel, String> {
    let uri = format!("http://127.0.0.1:{port}");
    let endpoint = Endpoint::from_shared(uri)
        .map_err(|e| format!("invalid gradle-server endpoint: {e}"))?
        .connect_timeout(Duration::from_secs(2));

    // Up to ~30s total, matching the VS Code client's readiness deadline.
    let mut last_err = String::new();
    for _ in 0..150 {
        match endpoint.connect().await {
            Ok(channel) => return Ok(channel),
            Err(e) => {
                last_err = e.to_string();
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(format!("gradle-server did not become ready: {last_err}"))
}

/// Build the ordered list of `workspace/executeCommand` argument tuples to send
/// to the language server for `root` and every subproject, recursively.
///
/// Each entry is `(command, arguments)` where `arguments` is the JSON array the
/// LS expects. The `projectPath` argument is normalized to match the key the LS
/// derives from a document URI (`Paths.get(uri).getParent().toString()`), i.e.
/// the absolute project directory — which is exactly what the model reports.
pub fn model_to_commands(root: &GradleProject) -> Vec<(&'static str, Value)> {
    let mut commands = Vec::new();
    collect_commands(root, &mut commands);
    commands
}

fn collect_commands(project: &GradleProject, out: &mut Vec<(&'static str, Value)>) {
    let project_path = normalize_project_path(&project.project_path);

    // gradle.setPlugins [projectPath, plugins[]]
    out.push((
        "gradle.setPlugins",
        json!([project_path, project.plugins]),
    ));

    // gradle.setClosures [projectPath, closures[]]
    let closures: Vec<Value> = project
        .plugin_closures
        .iter()
        .map(|closure| {
            let methods: Vec<Value> = closure
                .methods
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "parameterTypes": m.parameter_types,
                        "deprecated": m.deprecated,
                    })
                })
                .collect();
            let fields: Vec<Value> = closure
                .fields
                .iter()
                .map(|f| json!({ "name": f.name, "deprecated": f.deprecated }))
                .collect();
            json!({ "name": closure.name, "methods": methods, "fields": fields })
        })
        .collect();
    out.push(("gradle.setClosures", json!([project_path, closures])));

    // gradle.setScriptClasspaths [projectPath, scriptClasspaths[]]
    out.push((
        "gradle.setScriptClasspaths",
        json!([project_path, project.script_classpaths]),
    ));

    for sub in &project.projects {
        collect_commands(sub, out);
    }
}

/// Normalize an absolute project path so it matches the key the language server
/// derives via `Paths.get(uri).getParent().toString()` — collapsing redundant
/// separators and `.`/`..` segments without resolving symlinks.
fn normalize_project_path(path: &str) -> String {
    use std::path::{Component, PathBuf};

    let mut normalized = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::gradle::{GrpcGradleClosure, GrpcGradleField, GrpcGradleMethod};

    fn sample_project() -> GradleProject {
        GradleProject {
            is_root: true,
            tasks: vec![],
            projects: vec![GradleProject {
                is_root: false,
                project_path: "/p/sub".to_string(),
                plugins: vec!["java".to_string()],
                ..Default::default()
            }],
            project_path: "/p".to_string(),
            dependency_item: None,
            plugins: vec!["java".to_string(), "application".to_string()],
            plugin_closures: vec![GrpcGradleClosure {
                name: "java".to_string(),
                methods: vec![GrpcGradleMethod {
                    name: "sourceCompatibility".to_string(),
                    parameter_types: vec!["String".to_string()],
                    deprecated: false,
                }],
                fields: vec![GrpcGradleField {
                    name: "sourceSets".to_string(),
                    deprecated: false,
                }],
            }],
            script_classpaths: vec!["/p/.gradle/x.jar".to_string()],
        }
    }

    #[test]
    fn emits_three_commands_per_project_recursively() {
        let cmds = model_to_commands(&sample_project());
        // root + 1 subproject, 3 commands each.
        assert_eq!(cmds.len(), 6);
        assert_eq!(cmds[0].0, "gradle.setPlugins");
        assert_eq!(cmds[1].0, "gradle.setClosures");
        assert_eq!(cmds[2].0, "gradle.setScriptClasspaths");
        // Root projectPath is arg 0 of setPlugins.
        assert_eq!(cmds[0].1[0], "/p");
        assert_eq!(cmds[0].1[1][0], "java");
        // Subproject follows.
        assert_eq!(cmds[3].1[0], "/p/sub");
    }

    #[test]
    fn closure_shape_matches_ls_contract() {
        let cmds = model_to_commands(&sample_project());
        let closures = &cmds[1].1[1];
        assert_eq!(closures[0]["name"], "java");
        assert_eq!(closures[0]["methods"][0]["name"], "sourceCompatibility");
        assert_eq!(closures[0]["methods"][0]["parameterTypes"][0], "String");
        assert_eq!(closures[0]["methods"][0]["deprecated"], false);
        assert_eq!(closures[0]["fields"][0]["name"], "sourceSets");
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize_project_path("/p/./sub/../sub"), "/p/sub");
    }
}
