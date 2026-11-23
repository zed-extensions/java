//! Gradle task-server facade and reconnectable pipe listener.

use std::env;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use prost::Message;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

use crate::jsonrpc::{GradleJsonRpcClient, RpcError};
use crate::proto::gradle::{
    get_build_reply::Kind, output::OutputType, CancelBuildReply, CancelBuildRequest, GetBuildReply,
    GetBuildRequest, GradleConfig, GradleProject,
};

const JAVA_EXTENSION_VERSION: &str = "3.18.0";
const TASK_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
    pub fn from_env() -> Self {
        let wrapper_enabled = env::var("GRADLE_SYNC_WRAPPER_ENABLED")
            .map(|value| !value.eq_ignore_ascii_case("false"))
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
            java_extension_version: JAVA_EXTENSION_VERSION.to_string(),
            java_home: self.java_home.clone(),
        }
    }
}

fn env_or_empty(key: &str) -> String {
    env::var(key).unwrap_or_default()
}

pub enum BuildOutcome {
    Model(GradleProject),
    Cancelled,
    TransportError(String),
    Error { error: String, causes: Vec<String> },
}

#[derive(Clone)]
struct ConnectedClient {
    generation: u64,
    client: GradleJsonRpcClient,
}

/// Cloneable facade used by the sync scheduler.
#[derive(Clone)]
pub struct GradleServer {
    clients: watch::Receiver<Option<ConnectedClient>>,
    config: DistributionConfig,
}

impl GradleServer {
    pub async fn get_build(&self, project_dir: &str, cancellation_key: &str) -> BuildOutcome {
        let client = match self.wait_for_client().await {
            Ok(client) => client,
            Err(error) => return BuildOutcome::TransportError(error),
        };
        let request = GetBuildRequest {
            project_dir: project_dir.to_string(),
            cancellation_key: cancellation_key.to_string(),
            gradle_config: Some(self.config.to_gradle_config()),
            show_output_colors: false,
        };
        let (stream_id, mut stream) = client.register_get_build_stream().await;
        let terminal = client.request("gradle/getBuild", &request, Some(stream_id));
        tokio::pin!(terminal);
        let mut state = ReplyState::default();
        let mut protocol_error = None;
        let terminal = loop {
            tokio::select! {
                result = &mut terminal => break Some(result),
                payload = stream.recv() => match payload {
                    Some(payload) => {
                        if let Err(error) = state.apply_payload(&payload) {
                            protocol_error = Some(format!(
                                "Invalid Gradle getBuild protobuf reply: {error}"
                            ));
                            break None;
                        }
                    }
                    None => break Some(terminal.await),
                },
            }
        };
        while protocol_error.is_none() {
            let Ok(payload) = stream.try_recv() else {
                break;
            };
            if let Err(error) = state.apply_payload(&payload) {
                protocol_error = Some(format!("Invalid Gradle getBuild protobuf reply: {error}"));
            }
        }
        if protocol_error.is_none() {
            if let Some(Ok(Some(payload))) = &terminal {
                if let Err(error) = state.apply_payload(payload) {
                    protocol_error =
                        Some(format!("Invalid Gradle getBuild protobuf reply: {error}"));
                }
            }
        }
        client.remove_get_build_stream(stream_id).await;
        if let Some(error) = protocol_error {
            client.close(error.clone()).await;
            return BuildOutcome::TransportError(error);
        }

        match terminal.expect("terminal result is present without a protocol error") {
            Ok(Some(_)) => {}
            Ok(None) => {}
            Err(error) => {
                if let Some(compatibility_error) = state.compatibility_error {
                    return BuildOutcome::Error {
                        error: compatibility_error,
                        causes: stderr_causes(&state.stderr),
                    };
                }
                if error.is_connection() {
                    return BuildOutcome::TransportError(rpc_error_message(&error));
                }
                return BuildOutcome::Error {
                    error: rpc_error_message(&error),
                    causes: stderr_causes(&state.stderr),
                };
            }
        }

        if state.cancelled {
            return BuildOutcome::Cancelled;
        }
        if let Some(error) = state.compatibility_error {
            return BuildOutcome::Error {
                error,
                causes: stderr_causes(&state.stderr),
            };
        }
        match state.model {
            Some(model) => BuildOutcome::Model(model),
            None => BuildOutcome::Error {
                error: "gradle-server returned no build model".to_string(),
                causes: stderr_causes(&state.stderr),
            },
        }
    }

    /// Best-effort cancellation of the currently active build.
    pub async fn cancel(&self, cancellation_key: &str) {
        let client = match self.wait_for_client().await {
            Ok(client) => client,
            Err(_) => return,
        };
        // A superseding save can race just ahead of getBuild reaching the Java
        // handler. Retry a "not running" response briefly so that request is
        // cancelled as soon as its cancellation key is registered.
        for _ in 0..20 {
            let result = client
                .request(
                    "gradle/cancelBuild",
                    &CancelBuildRequest {
                        cancellation_key: cancellation_key.to_string(),
                    },
                    None,
                )
                .await;
            let Ok(Some(payload)) = result else {
                return;
            };
            let reply = match CancelBuildReply::decode(payload.as_slice()) {
                Ok(reply) => reply,
                Err(error) => {
                    client
                        .close(format!(
                            "Invalid Gradle cancelBuild protobuf response: {error}"
                        ))
                        .await;
                    return;
                }
            };
            if reply.build_running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_client(&self) -> Result<GradleJsonRpcClient, String> {
        let mut clients = self.clients.clone();
        tokio::time::timeout(TASK_CONNECT_TIMEOUT, async move {
            loop {
                if let Some(connected) = clients.borrow().clone() {
                    if !connected.client.is_closed() {
                        return Ok(connected.client);
                    }
                }
                clients
                    .changed()
                    .await
                    .map_err(|_| "Gradle task pipe listener stopped".to_string())?;
            }
        })
        .await
        .map_err(|_| "Timed out waiting for gradle-server task pipe".to_string())?
    }
}

#[derive(Default)]
struct ReplyState {
    model: Option<GradleProject>,
    stderr: String,
    compatibility_error: Option<String>,
    cancelled: bool,
}

impl ReplyState {
    fn apply_payload(&mut self, payload: &[u8]) -> Result<(), prost::DecodeError> {
        let reply = GetBuildReply::decode(payload)?;
        match reply.kind {
            Some(Kind::GetBuildResult(result)) => {
                self.model = result.build.and_then(|build| build.project);
            }
            Some(Kind::CompatibilityCheckError(error)) => {
                self.compatibility_error = Some(error);
            }
            Some(Kind::Output(output)) if output.output_type == OutputType::Stderr as i32 => {
                self.stderr
                    .push_str(&String::from_utf8_lossy(&output.output_bytes));
            }
            Some(Kind::Cancelled(_)) => {
                self.cancelled = true;
            }
            _ => {}
        }
        Ok(())
    }
}

fn rpc_error_message(error: &RpcError) -> String {
    match (&error.code, &error.data) {
        (Some(code), Some(data)) => format!("{} ({code}): {data}", error.message),
        (Some(code), None) => format!("{} ({code})", error.message),
        (None, _) => error.message.clone(),
    }
}

fn stderr_causes(stderr: &str) -> Vec<String> {
    stderr
        .trim()
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Owns the task-pipe accept loop and Unix socket path.
pub struct TaskPipeListener {
    pipe_path: String,
    accept_task: tokio::task::JoinHandle<()>,
    cleanup_path: Option<PathBuf>,
}

impl TaskPipeListener {
    pub fn pipe_path(&self) -> &str {
        &self.pipe_path
    }
}

impl Drop for TaskPipeListener {
    fn drop(&mut self) {
        self.accept_task.abort();
        if let Some(path) = &self.cleanup_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn bind_task_pipe(
    pipe_path: &str,
    config: DistributionConfig,
) -> Result<(GradleServer, TaskPipeListener), String> {
    let (clients_tx, clients_rx) = watch::channel(None);
    let generation = AtomicU64::new(1);

    #[cfg(unix)]
    let (accept_task, cleanup_path) = {
        let path = PathBuf::from(pipe_path);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|error| format!("Failed to remove stale task socket: {error}"))?;
        }
        let listener = tokio::net::UnixListener::bind(&path)
            .map_err(|error| format!("Failed to bind Gradle task socket: {error}"))?;
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let next_generation = generation.fetch_add(1, Ordering::Relaxed);
                publish_connection(stream, next_generation, clients_tx.clone()).await;
            }
        });
        (task, Some(path))
    };

    #[cfg(windows)]
    let (accept_task, cleanup_path) = {
        use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(true)
            .pipe_mode(PipeMode::Byte)
            .access_inbound(true)
            .access_outbound(true)
            .reject_remote_clients(true);
        let first = options
            .create(pipe_path)
            .map_err(|error| format!("Failed to create Gradle task pipe: {error}"))?;
        let path = pipe_path.to_string();
        let task = tokio::spawn(async move {
            let mut server = first;
            let mut first_instance = false;
            loop {
                if server.connect().await.is_err() {
                    return;
                }
                let connected = server;
                let mut options = ServerOptions::new();
                options
                    .first_pipe_instance(first_instance)
                    .pipe_mode(PipeMode::Byte)
                    .access_inbound(true)
                    .access_outbound(true)
                    .reject_remote_clients(true);
                server = match options.create(&path) {
                    Ok(server) => server,
                    Err(_) => return,
                };
                first_instance = false;
                let next_generation = generation.fetch_add(1, Ordering::Relaxed);
                publish_connection(connected, next_generation, clients_tx.clone()).await;
            }
        });
        (task, None)
    };

    Ok((
        GradleServer {
            clients: clients_rx,
            config,
        },
        TaskPipeListener {
            pipe_path: pipe_path.to_string(),
            accept_task,
            cleanup_path,
        },
    ))
}

async fn publish_connection<S>(
    stream: S,
    generation: u64,
    clients: watch::Sender<Option<ConnectedClient>>,
) where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let client = GradleJsonRpcClient::new(reader, writer);
    let mut closed = client.subscribe_closed();
    let connected = ConnectedClient { generation, client };
    if let Some(previous) = clients.send_replace(Some(connected)) {
        previous
            .client
            .close("Gradle task pipe connection replaced")
            .await;
    }

    tokio::spawn(async move {
        while !*closed.borrow() {
            if closed.changed().await.is_err() {
                break;
            }
        }
        clients.send_if_modified(|current| {
            if current
                .as_ref()
                .is_some_and(|client| client.generation == generation)
            {
                *current = None;
                true
            } else {
                false
            }
        });
    });
}

/// Generate a short, unique filesystem socket directory. Unix-domain socket
/// path limits are 103 bytes on macOS and 107 on Linux, so fall back to `/tmp`
/// when the platform temp directory is too long.
#[cfg(unix)]
pub fn socket_directory() -> Result<PathBuf, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_nanos();
    let name = format!("gradle-ls-{}-{nonce:x}", std::process::id());
    let mut directory = std::env::temp_dir().join(&name);
    if path_byte_len(&directory.join("task.sock")) > safe_socket_path_limit() {
        directory = Path::new("/tmp").join(name);
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Failed to create Gradle socket directory: {error}"))?;
    Ok(directory)
}

#[cfg(unix)]
fn path_byte_len(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().len()
}

#[cfg(target_os = "macos")]
const fn safe_socket_path_limit() -> usize {
    103
}

#[cfg(all(unix, not(target_os = "macos")))]
const fn safe_socket_path_limit() -> usize {
    107
}

#[cfg(windows)]
pub fn windows_pipe_name(kind: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        r"\\.\pipe\gradle-ls-{}-{nonce:x}-{kind}",
        std::process::id()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use proxy_common::{encode_lsp, parse_lsp_content, AsyncLspReader};
    use serde_json::{json, Value};
    use tokio::io::{duplex, split, AsyncWrite, AsyncWriteExt};

    use crate::proto::gradle::{get_build_reply, output, GetBuildResult, GradleBuild, Output};

    fn server_with_client(client: GradleJsonRpcClient) -> GradleServer {
        let (_, clients) = watch::channel(Some(ConnectedClient {
            generation: 1,
            client,
        }));
        GradleServer {
            clients,
            config: DistributionConfig::default(),
        }
    }

    async fn send_json(writer: &mut (impl AsyncWrite + Unpin), value: Value) {
        writer
            .write_all(encode_lsp(&value).as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
    }

    #[test]
    fn reply_state_collects_stderr_and_terminal_model() {
        let stderr = GetBuildReply {
            kind: Some(Kind::Output(Output {
                output_type: output::OutputType::Stderr as i32,
                output_bytes: b"broken build".to_vec(),
            })),
        };
        let model = GradleProject {
            project_path: "/project".to_string(),
            ..Default::default()
        };
        let terminal = GetBuildReply {
            kind: Some(get_build_reply::Kind::GetBuildResult(GetBuildResult {
                message: String::new(),
                build: Some(GradleBuild {
                    project: Some(model),
                }),
            })),
        };
        let mut state = ReplyState::default();
        state.apply_payload(&stderr.encode_to_vec()).unwrap();
        state.apply_payload(&terminal.encode_to_vec()).unwrap();
        assert_eq!(state.stderr, "broken build");
        assert_eq!(state.model.unwrap().project_path, "/project");
    }

    #[test]
    fn cancelled_reply_is_distinct_from_missing_model() {
        let reply = GetBuildReply {
            kind: Some(Kind::Cancelled(Default::default())),
        };
        let mut state = ReplyState::default();
        state.apply_payload(&reply.encode_to_vec()).unwrap();
        assert!(state.cancelled);
    }

    #[tokio::test]
    async fn unavailable_client_is_a_transport_error() {
        let (clients_tx, clients) = watch::channel::<Option<ConnectedClient>>(None);
        drop(clients_tx);
        let server = GradleServer {
            clients,
            config: DistributionConfig::default(),
        };

        match server.get_build("/project", "sync-1").await {
            BuildOutcome::TransportError(error) => {
                assert!(error.contains("listener stopped"));
            }
            _ => panic!("expected transport error"),
        }
    }

    #[tokio::test]
    async fn malformed_protobuf_reply_closes_connection() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);
        let server = server_with_client(client.clone());
        let build = tokio::spawn(async move { server.get_build("/project", "sync-1").await });

        let mut reader = AsyncLspReader::new(server_read);
        let raw = reader.read_message().await.unwrap().unwrap();
        let request = parse_lsp_content(&raw).unwrap();
        let stream_id = request["params"]["streamId"].as_u64().unwrap();
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "method": "gradle/getBuild/reply",
                "params": {
                    "streamId": stream_id,
                    "payload": BASE64.encode([0xff])
                }
            }),
        )
        .await;

        match build.await.unwrap() {
            BuildOutcome::TransportError(error) => {
                assert!(error.contains("Invalid Gradle getBuild protobuf reply"));
            }
            _ => panic!("expected transport error"),
        }
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn replacing_client_closes_in_flight_request() {
        let (clients_tx, clients) = watch::channel(None);
        let (first_bridge, first_peer) = duplex(64 * 1024);
        let (first_read, _first_write) = split(first_peer);
        publish_connection(first_bridge, 1, clients_tx.clone()).await;
        let first_client = clients
            .borrow()
            .as_ref()
            .expect("first client should be published")
            .client
            .clone();

        let request = tokio::spawn({
            let first_client = first_client.clone();
            async move {
                first_client
                    .request("gradle/cancelBuild", &CancelBuildRequest::default(), None)
                    .await
            }
        });
        let mut first_reader = AsyncLspReader::new(first_read);
        first_reader.read_message().await.unwrap().unwrap();

        let (second_bridge, _second_peer) = duplex(64 * 1024);
        publish_connection(second_bridge, 2, clients_tx).await;

        let error = request.await.unwrap().unwrap_err();
        assert!(error.message.contains("replaced"));
        assert!(first_client.is_closed());
        assert_eq!(
            clients.borrow().as_ref().map(|client| client.generation),
            Some(2)
        );
        let message = tokio::time::timeout(Duration::from_secs(1), first_reader.read_message())
            .await
            .expect("replaced peer should observe EOF")
            .unwrap();
        assert!(message.is_none());
    }

    #[tokio::test]
    async fn cancellation_retries_until_build_is_registered() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let server = server_with_client(GradleJsonRpcClient::new(client_read, client_write));
        let cancellation = tokio::spawn(async move {
            server.cancel("sync-1").await;
        });

        let mut reader = AsyncLspReader::new(server_read);
        for build_running in [false, true] {
            let raw = reader.read_message().await.unwrap().unwrap();
            let request = parse_lsp_content(&raw).unwrap();
            assert_eq!(request["method"], "gradle/cancelBuild");
            let reply = CancelBuildReply {
                message: String::new(),
                build_running,
            };
            send_json(
                &mut server_write,
                json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": { "reply": BASE64.encode(reply.encode_to_vec()) }
                }),
            )
            .await;
        }

        cancellation.await.unwrap();
    }

    #[tokio::test]
    async fn compatibility_notification_takes_precedence_over_terminal_error() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let server = server_with_client(GradleJsonRpcClient::new(client_read, client_write));
        let build = tokio::spawn(async move { server.get_build("/project", "sync-1").await });

        let mut reader = AsyncLspReader::new(server_read);
        let raw = reader.read_message().await.unwrap().unwrap();
        let request = parse_lsp_content(&raw).unwrap();
        let stream_id = request["params"]["streamId"].as_u64().unwrap();
        let compatibility = GetBuildReply {
            kind: Some(Kind::CompatibilityCheckError(
                "Gradle and Java are incompatible".to_string(),
            )),
        };
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "method": "gradle/getBuild/reply",
                "params": {
                    "streamId": stream_id,
                    "payload": BASE64.encode(compatibility.encode_to_vec())
                }
            }),
        )
        .await;
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": { "code": -32603, "message": "Internal error" }
            }),
        )
        .await;

        match build.await.unwrap() {
            BuildOutcome::Error { error, .. } => {
                assert_eq!(error, "Gradle and Java are incompatible");
            }
            _ => panic!("expected compatibility error"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn task_socket_accepts_reconnections_and_cleans_up() {
        let directory = socket_directory().unwrap();
        let path = directory.join("task.sock");
        let path_string = path.to_str().unwrap().to_string();
        let (_server, listener) =
            bind_task_pipe(&path_string, DistributionConfig::default()).unwrap();

        let first = tokio::net::UnixStream::connect(&path).await.unwrap();
        drop(first);
        let second = tokio::net::UnixStream::connect(&path).await.unwrap();
        drop(second);
        drop(listener);
        assert!(!path.exists());
        let _ = std::fs::remove_dir(directory);
    }
}
