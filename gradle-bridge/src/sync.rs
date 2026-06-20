//! Serializes Gradle build-model syncs: at most one runs at a time, and requests
//! arriving while a sync is in flight collapse into a single pending rerun. This
//! mirrors the VS Code extension, which serializes refreshes and coalesces bursts
//! (e.g. "save all") rather than launching a Gradle build per event.
//!
//! Unlike the previous fork-per-save helper, a sync here is a `GetBuild` RPC to
//! the already-warm `gradle-server`, so a coalesced burst resolves quickly.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use proxy_common::encode_lsp;

use crate::channel::{build_eval_diagnostics, EditorChannel, INJECTED_ID_PREFIX};
use crate::grpc::{model_to_commands, BuildOutcome, GradleServer};
use crate::transport::LsWriter;

/// Coordinates single-flight syncs against the shared [`GradleServer`].
#[derive(Clone)]
pub struct SyncScheduler {
    state: Arc<Mutex<SyncState>>,
    server: GradleServer,
    channel: Arc<EditorChannel>,
    ls_writer: LsWriter,
    project_dir: String,
}

#[derive(Default)]
struct SyncState {
    running: bool,
    /// A rerun was requested while a sync was in flight; run exactly once more.
    pending: bool,
    /// A monotonically increasing key so a superseding sync can cancel the
    /// in-flight build.
    seq: u64,
}

impl SyncScheduler {
    pub fn new(
        server: GradleServer,
        channel: Arc<EditorChannel>,
        ls_writer: LsWriter,
        project_dir: String,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(SyncState::default())),
            server,
            channel,
            ls_writer,
            project_dir,
        }
    }

    /// Request a sync. Starts a worker if none is running; otherwise marks a
    /// single rerun pending and cancels the in-flight build so the newest inputs
    /// win quickly.
    pub async fn request(&self) {
        let mut state = self.state.lock().await;
        if state.running {
            state.pending = true;
            let key = cancellation_key(state.seq);
            drop(state);
            // Cancel the in-flight build; the pending rerun picks up the change.
            self.server.cancel(&key).await;
            return;
        }
        state.running = true;
        drop(state);
        self.spawn_worker();
    }

    fn spawn_worker(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let seq = {
                    let mut s = this.state.lock().await;
                    s.pending = false;
                    s.seq += 1;
                    s.seq
                };

                this.run_once(seq).await;

                let mut s = this.state.lock().await;
                if !s.pending {
                    s.running = false;
                    break;
                }
            }
        });
    }

    /// Perform one `GetBuild` + forward cycle.
    async fn run_once(&self, seq: u64) {
        let key = cancellation_key(seq);
        match self.server.get_build(&self.project_dir, &key).await {
            BuildOutcome::Model(root) => {
                // Successful evaluation: clear any prior build-eval diagnostics.
                self.channel
                    .set_sync_diagnostics(std::collections::HashMap::new())
                    .await;

                let commands = model_to_commands(&root);
                for (idx, (command, arguments)) in commands.into_iter().enumerate() {
                    let msg = json!({
                        "jsonrpc": "2.0",
                        "id": format!("{INJECTED_ID_PREFIX}{seq}-{idx}"),
                        "method": "workspace/executeCommand",
                        "params": { "command": command, "arguments": arguments }
                    });
                    self.ls_writer.send(encode_lsp(&msg).into_bytes()).await;
                }
            }
            BuildOutcome::Error { error, causes } => {
                // Log the full detail (the top-level message plus the Gradle
                // stderr captured as causes), not just the generic outer message
                // — the causes are where the offending build file and line live.
                eprintln!("[gradle-bridge] build model sync failed: {error}");
                for cause in &causes {
                    eprintln!("[gradle-bridge]   {cause}");
                }
                let build_file = default_build_file(&self.project_dir);
                self.channel
                    .set_sync_diagnostics(build_eval_diagnostics(
                        &error,
                        &causes,
                        build_file.as_deref(),
                    ))
                    .await;
            }
        }
    }
}

fn cancellation_key(seq: u64) -> String {
    format!("gradle-bridge-sync-{seq}")
}

/// Best-effort default build file for attaching a diagnostic when the error
/// message carries no path: `build.gradle`, else `build.gradle.kts`.
fn default_build_file(project_dir: &str) -> Option<String> {
    let dir = std::path::Path::new(project_dir);
    for name in ["build.gradle", "build.gradle.kts"] {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return candidate.to_str().map(str::to_string);
        }
    }
    None
}
