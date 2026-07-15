//! Transport between the editor (stdin/stdout) and the Gradle Language Server
//! (a Unix domain socket on macOS/Linux, a named pipe on Windows — the LS does
//! not support stdio), plus the two async pumps that move LSP messages in each
//! direction.
//!
//! The pumps are generic over the LS-side read/write halves so the Unix-socket
//! and Windows-pipe paths share identical framing, sync-driving, injected-
//! response filtering, and diagnostics merging.

use std::sync::Arc;

use proxy_common::AsyncLspReader;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::channel::{
    is_gradle_build_file_save, is_initialized_notification, is_injected_response,
    parse_publish_diagnostics, EditorChannel,
};
use crate::sync::SyncScheduler;

/// A cloneable handle for writing framed LSP messages to the language server.
/// Wraps the LS-side writer behind a mutex so both the editor→LS pump and the
/// sync worker's injected commands can share it.
#[derive(Clone)]
pub struct LsWriter {
    inner: Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>,
}

impl LsWriter {
    pub fn new<W: AsyncWrite + Send + Unpin + 'static>(writer: W) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Box::new(writer))),
        }
    }

    /// Write raw bytes to the LS, flushing afterward. Errors are swallowed; a
    /// dead LS surfaces via the pumps' read loops ending.
    pub async fn send(&self, bytes: Vec<u8>) {
        let mut w = self.inner.lock().await;
        let _ = w.write_all(&bytes).await;
        let _ = w.flush().await;
    }
}

/// Pump messages from the language server to the editor via `channel`, parsing
/// LSP framing so that:
/// - responses to the bridge's own injected requests are dropped (the editor
///   never issued them);
/// - the server's `publishDiagnostics` are merged with the build-model sync's
///   diagnostics for the same URI rather than overwriting them.
///
/// All other messages are forwarded verbatim. Runs until the server closes the
/// connection.
pub async fn pump_ls_to_editor<R: AsyncRead + Unpin>(ls_reader: R, channel: Arc<EditorChannel>) {
    let mut reader = AsyncLspReader::new(ls_reader);
    while let Ok(Some(raw)) = reader.read_message().await {
        if is_injected_response(&raw) {
            continue;
        }
        if let Some((uri, diagnostics)) = parse_publish_diagnostics(&raw) {
            // The language server parses every file as Groovy, but Kotlin-DSL
            // build scripts (`*.gradle.kts`) are not Groovy — so its diagnostics
            // for those are spurious syntax errors. Drop them (publish an empty
            // set) while still letting the build-eval diagnostics from the
            // gradle-server sync show through the merge. Groovy `.gradle` files
            // keep their real syntax diagnostics.
            let diagnostics = if uri.ends_with(".gradle.kts") {
                Vec::new()
            } else {
                diagnostics
            };
            channel.set_server_diagnostics(uri, diagnostics).await;
            continue;
        }
        if !channel.forward_raw(&raw).await {
            break;
        }
    }
}

/// Pump messages from the editor's stdin to the language server, forwarding each
/// verbatim and driving the build-model sync: an initial sync once the server is
/// initialized, then a re-sync on every save of a Gradle build file or
/// `gradle-wrapper.properties`.
///
/// Runs until stdin closes.
pub async fn pump_editor_to_ls<R: AsyncRead + Unpin>(
    editor_reader: R,
    ls_writer: LsWriter,
    scheduler: SyncScheduler,
) {
    let mut reader = AsyncLspReader::new(editor_reader);
    let mut initialized_sent = false;

    while let Ok(Some(raw)) = reader.read_message().await {
        ls_writer.send(raw.clone()).await;

        // Initial sync once the server is ready, then re-sync on every save of a
        // Gradle build file — the build model (plugins, closures, classpaths)
        // can change and the language server otherwise keeps stale completions.
        // (We don't watch `gradle-wrapper.properties`: it belongs to the
        // Properties language, so the editor never routes its saves to us.)
        let should_sync = if !initialized_sent && is_initialized_notification(&raw) {
            initialized_sent = true;
            true
        } else {
            initialized_sent && is_gradle_build_file_save(&raw)
        };

        if should_sync {
            scheduler.request().await;
        }
    }
}

/// Split an owned duplex stream (used on Windows, where read and write share one
/// pipe handle) into the read and write halves the pumps expect.
#[cfg(windows)]
pub fn split_duplex<S>(stream: S) -> (tokio::io::ReadHalf<S>, tokio::io::WriteHalf<S>)
where
    S: AsyncRead + AsyncWrite,
{
    tokio::io::split(stream)
}
