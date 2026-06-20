//! `gradle-lsp-bridge` — bridges Zed (LSP over stdio) to the Microsoft Gradle
//! Language Server (LSP over a Unix socket / Windows named pipe) and drives the
//! real shipped `gradle-server.jar` over gRPC to feed the LS a plugin-aware
//! build model.
//!
//! Invocation (set up by the Zed Java extension):
//!
//! ```text
//! gradle-lsp-bridge <java> -cp <classpath> com.microsoft.gradle.GradleLanguageServer
//! ```
//!
//! The classpath already contains every jar the gradle-server needs
//! (`gradle-server.jar`, grpc-netty, netty, the Tooling API), so the bridge
//! launches the server from the same classpath — no extra jars shipped.

mod channel;
mod grpc;
mod proto;
mod sync;
mod transport;

use std::process;
use std::sync::Arc;

use channel::EditorChannel;
use grpc::{DistributionConfig, GradleServer};
use sync::SyncScheduler;
use transport::{pump_editor_to_ls, pump_ls_to_editor, LsWriter};

/// Parsed launch arguments: the java binary, the LS classpath, and the LS main
/// class. Mirrors the `<java> -cp <classpath> <mainclass>` shape.
struct Args {
    java: String,
    classpath: String,
    main_class: String,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Expect: <java> -cp <classpath> <mainclass>
    let cp_idx = args.iter().position(|a| a == "-cp");
    let (Some(java), Some(cp_idx)) = (args.first().cloned(), cp_idx) else {
        eprintln!(
            "Usage: gradle-lsp-bridge <java> -cp <classpath> com.microsoft.gradle.GradleLanguageServer"
        );
        process::exit(1);
    };
    let Some(classpath) = args.get(cp_idx + 1).cloned() else {
        eprintln!("gradle-lsp-bridge: missing classpath after -cp");
        process::exit(1);
    };
    let Some(main_class) = args.get(cp_idx + 2).cloned() else {
        eprintln!("gradle-lsp-bridge: missing language server main class");
        process::exit(1);
    };
    Args {
        java,
        classpath,
        main_class,
    }
}

/// The project root the editor opened. Zed launches the bridge with the project
/// root as the working directory (`PWD`), matching how the previous helper
/// resolved it.
fn project_dir() -> Option<String> {
    std::env::var("PWD").ok().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
    })
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
    runtime.block_on(run(args));
}

/// Construct the long-lived gradle-server manager from the launch args + env.
fn build_server(args: &Args) -> GradleServer {
    let java_home = std::env::var("JAVA_HOME").ok().filter(|s| !s.is_empty());
    GradleServer::new(
        args.java.clone(),
        args.classpath.clone(),
        java_home,
        DistributionConfig::from_env(),
    )
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

    // Either side closing ends the bridge.
    tokio::select! {
        _ = ls_to_editor => {}
        _ = editor_to_ls => {}
    }
}

#[cfg(unix)]
async fn run(args: Args) {
    use tokio::net::UnixListener;

    let socket_dir = std::env::temp_dir().join(format!("gradle-ls-{}", process::id()));
    if let Err(e) = tokio::fs::create_dir_all(&socket_dir).await {
        eprintln!("gradle-lsp-bridge: failed to create socket dir: {e}");
        process::exit(1);
    }
    let socket_path = socket_dir.join("ls.sock");

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gradle-lsp-bridge: failed to bind socket: {e}");
            process::exit(1);
        }
    };

    // Spawn the language server pointed at our socket.
    let mut ls_child = match std::process::Command::new(&args.java)
        .args([
            "-cp",
            &args.classpath,
            &args.main_class,
            &socket_path.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gradle-lsp-bridge: failed to spawn language server: {e}");
            process::exit(1);
        }
    };

    // Terminate the LS if the editor that launched us goes away.
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    proxy_common::spawn_parent_monitor(Arc::clone(&alive), ls_child.id());

    let stream = match listener.accept().await {
        Ok((stream, _)) => stream,
        Err(e) => {
            eprintln!("gradle-lsp-bridge: failed to accept LS connection: {e}");
            let _ = ls_child.kill();
            process::exit(1);
        }
    };
    let (ls_read, ls_write) = stream.into_split();

    let server = build_server(&args);
    drive(ls_read, ls_write, server.clone()).await;

    server.shutdown().await;
    let _ = ls_child.kill();
    let _ = tokio::fs::remove_file(&socket_path).await;
    let _ = tokio::fs::remove_dir(&socket_dir).await;
}

#[cfg(windows)]
async fn run(args: Args) {
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = format!("\\\\.\\pipe\\gradle-ls-{}", process::id());
    let server_pipe = match ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("gradle-lsp-bridge: failed to create named pipe: {e}");
            process::exit(1);
        }
    };

    // Spawn the language server pointed at our pipe.
    let mut ls_child = match std::process::Command::new(&args.java)
        .args(["-cp", &args.classpath, &args.main_class, &pipe_name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gradle-lsp-bridge: failed to spawn language server: {e}");
            process::exit(1);
        }
    };

    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    proxy_common::spawn_parent_monitor(Arc::clone(&alive), ls_child.id());

    if let Err(e) = server_pipe.connect().await {
        eprintln!("gradle-lsp-bridge: failed to accept pipe connection: {e}");
        let _ = ls_child.kill();
        process::exit(1);
    }

    let (ls_read, ls_write) = transport::split_duplex(server_pipe);

    let server = build_server(&args);
    drive(ls_read, ls_write, server.clone()).await;

    server.shutdown().await;
    let _ = ls_child.kill();
}
