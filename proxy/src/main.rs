mod completions;
mod decompile;
mod http;
mod log;
mod lsp;
mod platform;

use completions::{should_sort_completions, sort_completions_by_param_count};
use decompile::{rewrite_jdt_in_strings, rewrite_jdt_locations};
use http::handle_http;
use lsp::{parse_lsp_content, raw_has_id, write_raw, write_to_stdout, LspReader};
use platform::spawn_parent_monitor;
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{self, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};

#[derive(Clone, Copy)]
enum TrackedKind {
    Definition,
    Doc,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Usage: java-lsp-proxy <workdir> <bin> [args...]");
        lsp_error!("Usage: java-lsp-proxy <workdir> <bin> [args...]");
        process::exit(1);
    }

    let workdir = &args[0];
    let bin = &args[1];
    let child_args = &args[2..];

    lsp_info!("java-lsp-proxy starting: bin={bin}, workdir={workdir}");

    let proxy_id = hex_encode(
        env::current_dir()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches('/'),
    );

    // Spawn JDTLS (use shell on Windows for .bat files)
    let mut cmd = Command::new(bin);
    cmd.args(child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    #[cfg(windows)]
    if bin.ends_with(".bat") || bin.ends_with(".cmd") {
        cmd = Command::new("cmd");
        cmd.arg("/C")
            .arg(bin)
            .args(child_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
    }

    let mut child = cmd.spawn().unwrap_or_else(|e| {
        eprintln!("Failed to spawn {bin}: {e}");
        lsp_error!("Failed to spawn {bin}: {e}");
        process::exit(1);
    });

    lsp_info!("JDTLS process spawned (pid={})", child.id());

    let child_stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let child_stdout = child.stdout.take().unwrap();
    let alive = Arc::new(AtomicBool::new(true));

    let pending: Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let port_file = Path::new(workdir).join("proxy").join(&proxy_id);
    fs::create_dir_all(port_file.parent().unwrap()).unwrap();
    fs::write(&port_file, port.to_string()).unwrap();

    lsp_info!("HTTP server listening on 127.0.0.1:{port}");

    let id_counter = Arc::new(AtomicU64::new(1));

    // Track definition/typeDefinition/implementation and documentation request IDs
    // so their responses can be intercepted and rewritten.
    let tracked_ids: Arc<Mutex<HashMap<Value, TrackedKind>>> = Arc::new(Mutex::new(HashMap::new()));

    // --- Thread 1: Zed stdin -> JDTLS stdin (track definition requests) ---
    let stdin_writer = Arc::clone(&child_stdin);
    let alive_stdin = Arc::clone(&alive);
    let tracked_in = Arc::clone(&tracked_ids);
    thread::spawn(move || {
        let stdin = io::stdin().lock();
        let mut reader = LspReader::new(BufReader::new(stdin));
        while alive_stdin.load(Ordering::Relaxed) {
            match reader.read_message() {
                Ok(Some(raw)) => {
                    // Only requests (not notifications) carry an `id`; skip the
                    // JSON parse entirely for high-volume notifications like
                    // textDocument/didChange.
                    if raw_has_id(&raw) {
                        if let Some(msg) = parse_lsp_content(&raw) {
                            if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                                let kind = match method {
                                    "textDocument/definition"
                                    | "textDocument/typeDefinition"
                                    | "textDocument/implementation" => {
                                        Some(TrackedKind::Definition)
                                    }
                                    "textDocument/hover"
                                    | "textDocument/signatureHelp"
                                    | "completionItem/resolve" => Some(TrackedKind::Doc),
                                    _ => None,
                                };
                                if let Some(kind) = kind {
                                    if let Some(id) = msg.get("id").cloned() {
                                        tracked_in.lock().unwrap().insert(id, kind);
                                    }
                                }
                            }
                        }
                    }
                    let mut w = stdin_writer.lock().unwrap();
                    if w.write_all(&raw).is_err() || w.flush().is_err() {
                        break;
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }
        alive_stdin.store(false, Ordering::Relaxed);
    });

    // --- Thread 2: JDTLS stdout -> rewrite jdt:// URIs, modify completions -> Zed stdout / resolve pending ---
    let pending_out = Arc::clone(&pending);
    let alive_out = Arc::clone(&alive);
    let tracked_out = Arc::clone(&tracked_ids);
    let decompile_writer = Arc::clone(&child_stdin);
    let decompile_pending = Arc::clone(&pending);
    let decompile_counter = Arc::clone(&id_counter);
    let decompile_proxy_id = proxy_id.clone();
    thread::spawn(move || {
        let mut reader = LspReader::new(BufReader::new(child_stdout));
        while alive_out.load(Ordering::Relaxed) {
            match reader.read_message() {
                Ok(Some(raw)) => {
                    // Fast path: notifications (no `id`) can't be responses we
                    // need to intercept. Forward the raw bytes without parsing.
                    if !raw_has_id(&raw) {
                        write_raw(&mut io::stdout().lock(), &raw);
                        continue;
                    }

                    let Some(mut msg) = parse_lsp_content(&raw) else {
                        write_raw(&mut io::stdout().lock(), &raw);
                        continue;
                    };

                    // Route responses to pending HTTP requests
                    if let Some(id) = msg.get("id") {
                        if let Some(tx) = pending_out.lock().unwrap().remove(id) {
                            let _ = tx.send(msg);
                            continue;
                        }
                    }

                    // Rewrite jdt:// URIs in definition or documentation responses.
                    // Spawns a thread so this loop stays unblocked and can route
                    // the java/classFileContents response back via `pending`.
                    if let Some(id) = msg.get("id").cloned() {
                        if let Some(kind) = tracked_out.lock().unwrap().remove(&id) {
                            let writer = Arc::clone(&decompile_writer);
                            let pending = Arc::clone(&decompile_pending);
                            let pid = decompile_proxy_id.clone();
                            let counter = Arc::clone(&decompile_counter);
                            thread::spawn(move || {
                                let mut next_id = move || {
                                    let seq = counter.fetch_add(1, Ordering::Relaxed);
                                    Value::String(format!("{pid}-decompile-{seq}"))
                                };
                                match kind {
                                    TrackedKind::Definition => {
                                        rewrite_jdt_locations(
                                            &mut msg,
                                            &writer,
                                            &pending,
                                            &mut next_id,
                                        );
                                    }
                                    TrackedKind::Doc => {
                                        rewrite_jdt_in_strings(
                                            &mut msg,
                                            &writer,
                                            &pending,
                                            &mut next_id,
                                        );
                                    }
                                }
                                write_to_stdout(&msg);
                            });
                            continue;
                        }
                    }

                    // Sort completion responses by param count
                    if should_sort_completions(&msg) {
                        sort_completions_by_param_count(&mut msg);
                        write_to_stdout(&msg);
                        continue;
                    }

                    // Passthrough
                    write_raw(&mut io::stdout().lock(), &raw);
                }
                Ok(None) | Err(_) => break,
            }
        }
        alive_out.store(false, Ordering::Relaxed);
    });

    // --- Thread 3: HTTP server for extension requests ---
    let http_writer = Arc::clone(&child_stdin);
    let http_pending = Arc::clone(&pending);
    let http_alive = Arc::clone(&alive);
    let http_id_counter = Arc::clone(&id_counter);
    let http_proxy_id = proxy_id.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            if !http_alive.load(Ordering::Relaxed) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let writer = Arc::clone(&http_writer);
            let pend = Arc::clone(&http_pending);
            let counter = Arc::clone(&http_id_counter);
            let pid = http_proxy_id.clone();

            thread::spawn(move || {
                handle_http(stream, writer, pend, counter, &pid);
            });
        }
    });

    // --- Thread 4: Parent process monitor ---
    spawn_parent_monitor(Arc::clone(&alive), child.id());

    // Wait for child to exit
    let status = child.wait();
    lsp_info!("JDTLS process exited: {status:?}");
    alive.store(false, Ordering::Relaxed);
    let _ = fs::remove_file(&port_file);
}

// --- Utilities ---

fn hex_encode(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
