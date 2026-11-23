//! Minimal JSON-RPC 2.0 client for the Gradle task server.
//!
//! vscode-gradle 3.18 keeps protobuf as the message schema, but carries encoded
//! protobuf bytes inside JSON-RPC envelopes framed with LSP `Content-Length`
//! headers. Only the methods used by this bridge are exposed here.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

use proxy_common::{encode_lsp, parse_lsp_content, AsyncLspReader};

const GET_BUILD_REPLY: &str = "gradle/getBuild/reply";

#[derive(Clone, Debug)]
pub struct RpcError {
    pub code: Option<i64>,
    pub message: String,
    pub data: Option<Value>,
    connection: bool,
}

impl RpcError {
    fn connection(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            data: None,
            connection: true,
        }
    }

    pub fn is_connection(&self) -> bool {
        self.connection
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RpcError {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GradleRequestParams {
    request: String,
    stream_id: Option<u64>,
}

#[derive(Deserialize)]
struct GradleResponse {
    reply: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GradleStreamPayload {
    stream_id: u64,
    payload: String,
}

type PendingSender = oneshot::Sender<Result<Value, RpcError>>;

struct ClientInner {
    writer: Mutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>,
    pending: Mutex<HashMap<u64, PendingSender>>,
    get_build_streams: Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>,
    next_request_id: AtomicU64,
    next_stream_id: AtomicU64,
    closed: AtomicBool,
    closed_tx: watch::Sender<bool>,
}

/// A cloneable client for one task-pipe connection.
#[derive(Clone)]
pub struct GradleJsonRpcClient {
    inner: Arc<ClientInner>,
}

impl GradleJsonRpcClient {
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (closed_tx, _) = watch::channel(false);
        let inner = Arc::new(ClientInner {
            writer: Mutex::new(Some(Box::new(writer))),
            pending: Mutex::new(HashMap::new()),
            get_build_streams: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            next_stream_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            closed_tx,
        });
        tokio::spawn(read_loop(reader, Arc::clone(&inner)));
        Self { inner }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    pub fn subscribe_closed(&self) -> watch::Receiver<bool> {
        self.inner.closed_tx.subscribe()
    }

    pub async fn close(&self, message: impl Into<String>) {
        close_connection(&self.inner, message).await;
    }

    /// Register a sink before sending `gradle/getBuild`, preventing an early
    /// server notification from racing ahead of stream registration.
    pub async fn register_get_build_stream(&self) -> (u64, mpsc::UnboundedReceiver<Vec<u8>>) {
        let stream_id = self.inner.next_stream_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner
            .get_build_streams
            .lock()
            .await
            .insert(stream_id, tx);
        (stream_id, rx)
    }

    pub async fn remove_get_build_stream(&self, stream_id: u64) {
        self.inner.get_build_streams.lock().await.remove(&stream_id);
    }

    /// Send a Gradle JSON-RPC request, encoding `request` as protobuf and
    /// decoding the optional protobuf reply bytes from the response envelope.
    pub async fn request<M: Message>(
        &self,
        method: &str,
        request: &M,
        stream_id: Option<u64>,
    ) -> Result<Option<Vec<u8>>, RpcError> {
        let id = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let params = GradleRequestParams {
            request: BASE64.encode(request.encode_to_vec()),
            stream_id,
        };
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let framed = encode_lsp(&message);
        let (tx, rx) = oneshot::channel();
        let mut closed = self.inner.closed_tx.subscribe();
        {
            let mut pending = self.inner.pending.lock().await;
            if self.is_closed() {
                return Err(RpcError::connection("Gradle JSON-RPC connection is closed"));
            }
            pending.insert(id, tx);
        }

        let write_result = tokio::select! {
            result = write_request(&self.inner, framed.as_bytes()) => result,
            () = wait_for_close(&self.inner, &mut closed) => {
                Err(RpcError::connection(
                    "Gradle JSON-RPC connection closed while writing request",
                ))
            }
        };
        if let Err(error) = write_result {
            let message = error.message.clone();
            close_connection(&self.inner, message.clone()).await;
            return Err(error);
        }

        let result = rx.await.map_err(|_| {
            RpcError::connection("Gradle JSON-RPC connection closed before request completed")
        })??;
        if result.is_null() {
            return Ok(None);
        }
        let response: GradleResponse = match serde_json::from_value(result) {
            Ok(response) => response,
            Err(error) => {
                let message = format!("Invalid Gradle JSON-RPC response: {error}");
                self.close(message.clone()).await;
                return Err(RpcError::connection(message));
            }
        };
        let Some(reply) = response.reply else {
            return Ok(None);
        };
        match BASE64.decode(reply) {
            Ok(reply) => Ok(Some(reply)),
            Err(error) => {
                let message = format!("Invalid base64 Gradle protobuf response: {error}");
                self.close(message.clone()).await;
                Err(RpcError::connection(message))
            }
        }
    }
}

async fn write_request(inner: &ClientInner, framed: &[u8]) -> Result<(), RpcError> {
    let mut writer = inner.writer.lock().await;
    if inner.closed.load(Ordering::Acquire) {
        return Err(RpcError::connection("Gradle JSON-RPC connection is closed"));
    }
    let writer = writer
        .as_mut()
        .ok_or_else(|| RpcError::connection("Gradle JSON-RPC writer is closed"))?;
    let result = match writer.write_all(framed).await {
        Ok(()) => writer.flush().await,
        Err(error) => Err(error),
    };
    result.map_err(|error| {
        RpcError::connection(format!("Failed to write Gradle JSON-RPC request: {error}"))
    })
}

async fn wait_for_close(inner: &ClientInner, closed: &mut watch::Receiver<bool>) {
    loop {
        if inner.closed.load(Ordering::Acquire) || *closed.borrow() {
            return;
        }
        if closed.changed().await.is_err() {
            return;
        }
    }
}

async fn read_loop<R>(reader: R, inner: Arc<ClientInner>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = AsyncLspReader::new(reader);
    let mut closed = inner.closed_tx.subscribe();
    loop {
        if inner.closed.load(Ordering::Acquire) {
            return;
        }
        let read_result = tokio::select! {
            result = reader.read_message() => result,
            changed = closed.changed() => {
                if changed.is_err() || *closed.borrow() {
                    return;
                }
                continue;
            }
        };
        let raw = match read_result {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                close_connection(&inner, "Gradle JSON-RPC connection closed").await;
                return;
            }
            Err(error) => {
                close_connection(
                    &inner,
                    format!("Failed to read Gradle JSON-RPC response: {error}"),
                )
                .await;
                return;
            }
        };
        let Some(message) = parse_lsp_content(&raw) else {
            close_connection(&inner, "Invalid JSON on Gradle JSON-RPC connection").await;
            return;
        };
        if message.get("method").and_then(Value::as_str) == Some(GET_BUILD_REPLY) {
            if let Err(error) = dispatch_get_build_reply(&inner, &message).await {
                close_connection(&inner, error).await;
                return;
            }
            continue;
        }

        if message.get("method").is_some() {
            continue;
        }
        let has_result = message.get("result").is_some();
        let has_error = message.get("error").is_some();
        if has_result == has_error {
            close_connection(
                &inner,
                "Invalid Gradle JSON-RPC response: expected exactly one of result or error",
            )
            .await;
            return;
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            close_connection(
                &inner,
                "Invalid Gradle JSON-RPC response: missing numeric id",
            )
            .await;
            return;
        };
        let sender = inner.pending.lock().await.remove(&id);
        let Some(sender) = sender else {
            close_connection(
                &inner,
                format!("Invalid Gradle JSON-RPC response: unknown id {id}"),
            )
            .await;
            return;
        };
        if let Some(error) = message.get("error") {
            let _ = sender.send(Err(parse_rpc_error(error)));
        } else {
            let _ = sender.send(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
        }
    }
}

async fn dispatch_get_build_reply(inner: &ClientInner, message: &Value) -> Result<(), String> {
    let params = message
        .get("params")
        .cloned()
        .ok_or_else(|| "Gradle getBuild reply is missing params".to_string())?;
    let payload = serde_json::from_value::<GradleStreamPayload>(params)
        .map_err(|error| format!("Invalid Gradle getBuild reply: {error}"))?;
    let bytes = BASE64
        .decode(payload.payload)
        .map_err(|error| format!("Invalid base64 Gradle getBuild reply: {error}"))?;
    if let Some(sender) = inner.get_build_streams.lock().await.get(&payload.stream_id) {
        let _ = sender.send(bytes);
    }
    Ok(())
}

fn parse_rpc_error(error: &Value) -> RpcError {
    RpcError {
        code: error.get("code").and_then(Value::as_i64),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Gradle JSON-RPC request failed")
            .to_string(),
        data: error.get("data").cloned(),
        connection: false,
    }
}

async fn close_connection(inner: &Arc<ClientInner>, message: impl Into<String>) {
    if inner.closed.swap(true, Ordering::AcqRel) {
        return;
    }
    let message = message.into();
    let _ = inner.closed_tx.send(true);
    let inner = Arc::clone(inner);
    let cleanup = tokio::spawn(async move {
        let pending = std::mem::take(&mut *inner.pending.lock().await);
        for (_, sender) in pending {
            let _ = sender.send(Err(RpcError::connection(message.clone())));
        }
        inner.get_build_streams.lock().await.clear();
        inner.writer.lock().await.take();
    });
    let _ = cleanup.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::gradle::{CancelBuildReply, CancelBuildRequest, GetBuildReply, Progress};
    use prost::Message;
    use tokio::io::{duplex, split, AsyncWriteExt};

    struct DropTrackingReader {
        dropped: Option<oneshot::Sender<()>>,
    }

    impl Drop for DropTrackingReader {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    impl AsyncRead for DropTrackingReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    struct DropTrackingWriter {
        started: Option<Arc<tokio::sync::Notify>>,
        dropped: Option<oneshot::Sender<()>>,
        block_writes: bool,
    }

    impl Drop for DropTrackingWriter {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    impl AsyncWrite for DropTrackingWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            if let Some(started) = &self.started {
                started.notify_one();
            }
            if self.block_writes {
                std::task::Poll::Pending
            } else {
                std::task::Poll::Ready(Ok(buf.len()))
            }
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    async fn send_json(writer: &mut (impl AsyncWrite + Unpin), value: Value) {
        writer
            .write_all(encode_lsp(&value).as_bytes())
            .await
            .unwrap();
        writer.flush().await.unwrap();
    }

    #[tokio::test]
    async fn sends_proto_request_and_decodes_reply() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);

        let server = tokio::spawn(async move {
            let mut reader = AsyncLspReader::new(server_read);
            let raw = reader.read_message().await.unwrap().unwrap();
            let request = parse_lsp_content(&raw).unwrap();
            assert_eq!(request["method"], "gradle/cancelBuild");
            let encoded = request["params"]["request"].as_str().unwrap();
            let decoded = BASE64.decode(encoded).unwrap();
            let request_proto = CancelBuildRequest::decode(decoded.as_slice()).unwrap();
            assert_eq!(request_proto.cancellation_key, "sync-1");

            let reply = CancelBuildReply {
                message: "cancelled".to_string(),
                build_running: true,
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
        });

        let bytes = client
            .request(
                "gradle/cancelBuild",
                &CancelBuildRequest {
                    cancellation_key: "sync-1".to_string(),
                },
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let reply = CancelBuildReply::decode(bytes.as_slice()).unwrap();
        assert!(reply.build_running);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn routes_stream_notification_before_terminal_response() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);
        let (stream_id, mut stream) = client.register_get_build_stream().await;

        let server = tokio::spawn(async move {
            let mut reader = AsyncLspReader::new(server_read);
            let raw = reader.read_message().await.unwrap().unwrap();
            let request = parse_lsp_content(&raw).unwrap();
            let progress = GetBuildReply {
                kind: Some(crate::proto::gradle::get_build_reply::Kind::Progress(
                    Progress {
                        message: "Configure project".to_string(),
                    },
                )),
            };
            send_json(
                &mut server_write,
                json!({
                    "jsonrpc": "2.0",
                    "method": GET_BUILD_REPLY,
                    "params": {
                        "streamId": stream_id,
                        "payload": BASE64.encode(progress.encode_to_vec())
                    }
                }),
            )
            .await;
            send_json(
                &mut server_write,
                json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": { "reply": Value::Null }
                }),
            )
            .await;
        });

        let _ = client
            .request(
                "gradle/getBuild",
                &crate::proto::gradle::GetBuildRequest::default(),
                Some(stream_id),
            )
            .await
            .unwrap();
        let bytes = stream.recv().await.unwrap();
        let reply = GetBuildReply::decode(bytes.as_slice()).unwrap();
        assert!(matches!(
            reply.kind,
            Some(crate::proto::gradle::get_build_reply::Kind::Progress(_))
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_pending_request_when_connection_closes() {
        let (client_side, server_side) = duplex(1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request("gradle/cancelBuild", &CancelBuildRequest::default(), None)
                    .await
            }
        });
        let mut reader = AsyncLspReader::new(server_read);
        reader.read_message().await.unwrap().unwrap();
        drop(reader);
        drop(server_write);

        let error = tokio::time::timeout(std::time::Duration::from_secs(1), request)
            .await
            .expect("request should fail promptly when the connection closes")
            .unwrap()
            .unwrap_err();
        assert!(error.message.contains("closed"));
        assert!(error.is_connection());
    }

    #[tokio::test]
    async fn client_close_sends_peer_eof() {
        let (client_side, server_side) = duplex(1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, _server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);
        let mut reader = AsyncLspReader::new(server_read);

        client.close("protocol error").await;

        let message =
            tokio::time::timeout(std::time::Duration::from_secs(1), reader.read_message())
                .await
                .expect("peer should observe EOF promptly")
                .unwrap();
        assert!(message.is_none());
    }

    #[tokio::test]
    async fn client_close_stops_reader_task() {
        let (reader_dropped_tx, reader_dropped_rx) = oneshot::channel();
        let client = GradleJsonRpcClient::new(
            DropTrackingReader {
                dropped: Some(reader_dropped_tx),
            },
            tokio::io::sink(),
        );

        client.close("protocol error").await;

        tokio::time::timeout(std::time::Duration::from_secs(1), reader_dropped_rx)
            .await
            .expect("reader task should stop promptly")
            .unwrap();
    }

    #[tokio::test]
    async fn client_close_drops_writer_with_noop_shutdown() {
        let (reader_dropped_tx, _reader_dropped_rx) = oneshot::channel();
        let (writer_dropped_tx, writer_dropped_rx) = oneshot::channel();
        let client = GradleJsonRpcClient::new(
            DropTrackingReader {
                dropped: Some(reader_dropped_tx),
            },
            DropTrackingWriter {
                started: None,
                dropped: Some(writer_dropped_tx),
                block_writes: false,
            },
        );

        client.close("protocol error").await;

        tokio::time::timeout(std::time::Duration::from_secs(1), writer_dropped_rx)
            .await
            .expect("writer should be dropped promptly")
            .unwrap();
    }

    #[tokio::test]
    async fn client_close_cancels_blocked_write() {
        let (reader_dropped_tx, _reader_dropped_rx) = oneshot::channel();
        let (writer_dropped_tx, writer_dropped_rx) = oneshot::channel();
        let write_started = Arc::new(tokio::sync::Notify::new());
        let client = GradleJsonRpcClient::new(
            DropTrackingReader {
                dropped: Some(reader_dropped_tx),
            },
            DropTrackingWriter {
                started: Some(Arc::clone(&write_started)),
                dropped: Some(writer_dropped_tx),
                block_writes: true,
            },
        );
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request("gradle/cancelBuild", &CancelBuildRequest::default(), None)
                    .await
            }
        });
        write_started.notified().await;

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.close("protocol error"),
        )
        .await
        .expect("close should not wait for a blocked write");

        let error = request.await.unwrap().unwrap_err();
        assert!(error.is_connection());
        writer_dropped_rx.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_response_closes_connection() {
        let (client_side, server_side) = duplex(1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request("gradle/cancelBuild", &CancelBuildRequest::default(), None)
                    .await
            }
        });
        let mut reader = AsyncLspReader::new(server_read);
        let raw = reader.read_message().await.unwrap().unwrap();
        let message = parse_lsp_content(&raw).unwrap();
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": "not a Gradle response"
            }),
        )
        .await;

        let error = request.await.unwrap().unwrap_err();
        assert!(error.message.contains("Invalid Gradle JSON-RPC response"));
        assert!(error.is_connection());
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn malformed_response_envelope_closes_connection() {
        let (client_side, server_side) = duplex(1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request("gradle/cancelBuild", &CancelBuildRequest::default(), None)
                    .await
            }
        });
        let mut reader = AsyncLspReader::new(server_read);
        reader.read_message().await.unwrap().unwrap();
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "result": Value::Null
            }),
        )
        .await;

        let error = request.await.unwrap().unwrap_err();
        assert!(error.message.contains("missing numeric id"));
        assert!(error.is_connection());
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn malformed_stream_payload_closes_connection() {
        let (client_side, server_side) = duplex(1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);
        let (stream_id, _stream) = client.register_get_build_stream().await;

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(
                        "gradle/getBuild",
                        &crate::proto::gradle::GetBuildRequest::default(),
                        Some(stream_id),
                    )
                    .await
            }
        });
        let mut reader = AsyncLspReader::new(server_read);
        reader.read_message().await.unwrap().unwrap();
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "method": GET_BUILD_REPLY,
                "params": {
                    "streamId": stream_id,
                    "payload": "not base64"
                }
            }),
        )
        .await;

        let error = request.await.unwrap().unwrap_err();
        assert!(error
            .message
            .contains("Invalid base64 Gradle getBuild reply"));
        assert!(error.is_connection());
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn cancel_request_completes_while_get_build_is_running() {
        let (client_side, server_side) = duplex(64 * 1024);
        let (client_read, client_write) = split(client_side);
        let (server_read, mut server_write) = split(server_side);
        let client = GradleJsonRpcClient::new(client_read, client_write);
        let (stream_id, mut stream) = client.register_get_build_stream().await;

        let get_build = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(
                        "gradle/getBuild",
                        &crate::proto::gradle::GetBuildRequest::default(),
                        Some(stream_id),
                    )
                    .await
            }
        });

        let mut reader = AsyncLspReader::new(server_read);
        let get_build_raw = reader.read_message().await.unwrap().unwrap();
        let get_build_request = parse_lsp_content(&get_build_raw).unwrap();

        let cancel = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(
                        "gradle/cancelBuild",
                        &CancelBuildRequest {
                            cancellation_key: "sync-1".to_string(),
                        },
                        None,
                    )
                    .await
            }
        });
        let cancel_raw = reader.read_message().await.unwrap().unwrap();
        let cancel_request = parse_lsp_content(&cancel_raw).unwrap();

        let cancel_reply = CancelBuildReply {
            message: "cancel requested".to_string(),
            build_running: true,
        };
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "id": cancel_request["id"],
                "result": { "reply": BASE64.encode(cancel_reply.encode_to_vec()) }
            }),
        )
        .await;

        let cancelled = GetBuildReply {
            kind: Some(crate::proto::gradle::get_build_reply::Kind::Cancelled(
                Default::default(),
            )),
        };
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "method": GET_BUILD_REPLY,
                "params": {
                    "streamId": stream_id,
                    "payload": BASE64.encode(cancelled.encode_to_vec())
                }
            }),
        )
        .await;
        send_json(
            &mut server_write,
            json!({
                "jsonrpc": "2.0",
                "id": get_build_request["id"],
                "result": { "reply": Value::Null }
            }),
        )
        .await;

        let cancel_bytes = cancel.await.unwrap().unwrap().unwrap();
        let decoded_cancel = CancelBuildReply::decode(cancel_bytes.as_slice()).unwrap();
        assert!(decoded_cancel.build_running);
        assert!(get_build.await.unwrap().unwrap().is_none());

        let payload = stream.recv().await.unwrap();
        let decoded = GetBuildReply::decode(payload.as_slice()).unwrap();
        assert!(matches!(
            decoded.kind,
            Some(crate::proto::gradle::get_build_reply::Kind::Cancelled(_))
        ));
    }
}
