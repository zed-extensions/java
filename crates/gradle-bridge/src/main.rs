//! `gradle-lsp-bridge` — bridges Zed (LSP over stdio) to the Microsoft Gradle
//! Language Server (LSP over a Unix socket / Windows named pipe) and drives the
//! real shipped `gradle-server.jar` over JSON-RPC to feed the LS a plugin-aware
//! build model.
//!
//! Invocation (set up by the Zed Java extension):
//!
//! ```text
//! gradle-lsp-bridge <java> -cp <classpath>
//! ```
//!
//! The classpath already contains every jar the Gradle server needs, so the
//! bridge launches the task and language servers together in one JVM.

mod channel;
mod jsonrpc;
mod model;
mod proto;
mod server;
mod sync;
mod transport;

use std::process;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use channel::EditorChannel;
use server::{bind_task_pipe, DistributionConfig, GradleServer};
use sync::SyncScheduler;
use transport::{pump_editor_to_ls, pump_ls_to_editor, LsWriter};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Parsed launch arguments: the Java binary and Gradle extension classpath.
struct Args {
    java: String,
    classpath: String,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Expect: <java> -cp <classpath>
    let cp_idx = args.iter().position(|a| a == "-cp");
    let (Some(java), Some(cp_idx)) = (args.first().cloned(), cp_idx) else {
        eprintln!("Usage: gradle-lsp-bridge <java> -cp <classpath>");
        process::exit(1);
    };
    let Some(classpath) = args.get(cp_idx + 1).cloned() else {
        eprintln!("gradle-lsp-bridge: missing classpath after -cp");
        process::exit(1);
    };
    Args { java, classpath }
}

/// The project root the editor opened. The process working directory is
/// authoritative; `PWD` is only a fallback when it cannot be read.
fn project_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .or_else(|| std::env::var("PWD").ok())
}

fn main() {
    let args = parse_args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("gradle-lsp-bridge: failed to start tokio runtime: {e}");
            process::exit(1);
        });
    if let Err(error) = runtime.block_on(run(args)) {
        eprintln!("gradle-lsp-bridge: {error}");
        process::exit(1);
    }
}

/// Wire up the channel/writer/scheduler and run both pumps to completion.
async fn drive<R, W>(ls_read: R, ls_write: W, server: GradleServer)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let channel = Arc::new(EditorChannel::new());
    let ls_writer = LsWriter::new(ls_write);
    let dir = project_dir().unwrap_or_else(|| ".".to_string());
    let scheduler = SyncScheduler::new(server, Arc::clone(&channel), ls_writer.clone(), dir);

    // LS -> editor: frame, drop injected responses, merge diagnostics.
    let ls_to_editor = tokio::spawn(pump_ls_to_editor(ls_read, Arc::clone(&channel)));

    // editor -> LS: forward + drive the build-model sync.
    let editor = tokio::io::stdin();
    let editor_to_ls = tokio::spawn(pump_editor_to_ls(editor, ls_writer, scheduler));

    // Either side closing ends the bridge. Abort and await the other pump so no
    // detached task retains a pipe or stdout handle during shutdown.
    let mut ls_to_editor = ls_to_editor;
    let mut editor_to_ls = editor_to_ls;
    let ls_finished_first = tokio::select! {
        _ = &mut ls_to_editor => {
            editor_to_ls.abort();
            true
        },
        _ = &mut editor_to_ls => {
            ls_to_editor.abort();
            false
        },
    };
    if ls_finished_first {
        let _ = editor_to_ls.await;
    } else {
        let _ = ls_to_editor.await;
    }
}

#[cfg(unix)]
async fn run(args: Args) -> Result<(), String> {
    use tokio::net::UnixListener;

    let socket_dir = server::socket_directory()?;
    let _socket_cleanup = UnixSocketCleanup(socket_dir.clone());
    let language_path = socket_dir.join("ls.sock");
    let task_path = socket_dir.join("task.sock");
    let language_path = language_path
        .to_str()
        .ok_or_else(|| "Gradle language socket path is not UTF-8".to_string())?;
    let task_path = task_path
        .to_str()
        .ok_or_else(|| "Gradle task socket path is not UTF-8".to_string())?;

    let language_listener = UnixListener::bind(language_path)
        .map_err(|error| format!("Failed to bind Gradle language socket: {error}"))?;
    let (server, task_listener) = bind_task_pipe(task_path, DistributionConfig::from_env())?;
    let mut child = spawn_gradle_server(&args, task_listener.pipe_path(), language_path)?;

    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    if let Some(child_pid) = child.id() {
        proxy_common::spawn_parent_monitor(Arc::clone(&alive), child_pid);
    }

    let stream = tokio::select! {
        result = language_listener.accept() => {
            result
                .map(|(stream, _)| stream)
                .map_err(|error| format!("Failed to accept Gradle language connection: {error}"))?
        }
        status = child.wait() => {
            return Err(format!("Gradle server exited before connecting: {}", format_exit(status)));
        }
        _ = tokio::time::sleep(STARTUP_TIMEOUT) => {
            return Err("Timed out waiting for Gradle language server connection".to_string());
        }
    };
    let (ls_read, ls_write) = stream.into_split();

    drive(ls_read, ls_write, server).await;

    alive.store(false, std::sync::atomic::Ordering::Release);
    stop_child(&mut child).await;
    drop(task_listener);
    Ok(())
}

#[cfg(unix)]
struct UnixSocketCleanup(std::path::PathBuf);

#[cfg(unix)]
impl Drop for UnixSocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
async fn run(args: Args) -> Result<(), String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let language_pipe = server::windows_pipe_name("language");
    let task_pipe = server::windows_pipe_name("task");
    let language_listener = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&language_pipe)
        .map_err(|error| format!("Failed to create Gradle language pipe: {error}"))?;
    let (server, task_listener) = bind_task_pipe(&task_pipe, DistributionConfig::from_env())?;
    let mut child = spawn_gradle_server(&args, task_listener.pipe_path(), &language_pipe)?;

    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    if let Some(child_pid) = child.id() {
        proxy_common::spawn_parent_monitor(Arc::clone(&alive), child_pid);
    }

    tokio::select! {
        result = language_listener.connect() => {
            result.map_err(|error| format!("Failed to accept Gradle language connection: {error}"))?;
        }
        status = child.wait() => {
            return Err(format!("Gradle server exited before connecting: {}", format_exit(status)));
        }
        _ = tokio::time::sleep(STARTUP_TIMEOUT) => {
            return Err("Timed out waiting for Gradle language server connection".to_string());
        }
    }

    let (ls_read, ls_write) = transport::split_duplex(language_listener);

    drive(ls_read, ls_write, server).await;

    alive.store(false, std::sync::atomic::Ordering::Release);
    stop_child(&mut child).await;
    drop(task_listener);
    Ok(())
}

fn spawn_gradle_server(
    args: &Args,
    task_pipe: &str,
    language_pipe: &str,
) -> Result<tokio::process::Child, String> {
    let mut command = tokio::process::Command::new(&args.java);
    command
        .args([
            "-Dfile.encoding=UTF-8",
            "-cp",
            &args.classpath,
            "com.github.badsyntax.gradle.GradleServer",
            &format!("--pipe={task_pipe}"),
            &format!("--parentPid={}", process::id()),
            "--startBuildServer=false",
            &format!("--languageServerPipePath={language_pipe}"),
        ])
        .stdin(Stdio::null())
        // Bridge stdout is the editor protocol; child output must never inherit it.
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        if !java_home.is_empty() {
            command.env("JAVA_HOME", &java_home);
            command.env("VSCODE_JAVA_HOME", java_home);
        }
    }
    command
        .spawn()
        .map_err(|error| format!("Failed to spawn Gradle server: {error}"))
}

async fn stop_child(child: &mut tokio::process::Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

fn format_exit(status: std::io::Result<std::process::ExitStatus>) -> String {
    status.map_or_else(|error| error.to_string(), |status| status.to_string())
}
