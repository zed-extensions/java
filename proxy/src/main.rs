mod completions;
mod decompile;
mod http;
mod log;
mod lsp;
mod platform;

use completions::{should_sort_completions, sort_completions_by_param_count};
use decompile::rewrite_jdt_locations;
use http::handle_http;
use lsp::{encode_lsp, parse_lsp_content, LspReader};
use platform::spawn_parent_monitor;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
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

    // Track definition/typeDefinition/implementation request IDs for jdt:// rewriting
    let definition_ids: Arc<Mutex<HashSet<Value>>> = Arc::new(Mutex::new(HashSet::new()));

    // --- Thread 1: Zed stdin -> JDTLS stdin (track definition requests) ---
    let stdin_writer = Arc::clone(&child_stdin);
    let alive_stdin = Arc::clone(&alive);
    let def_ids_in = Arc::clone(&definition_ids);
    thread::spawn(move || {
        let stdin = io::stdin().lock();
        let mut reader = LspReader::new(stdin);
        while alive_stdin.load(Ordering::Relaxed) {
            match reader.read_message() {
                Ok(Some(raw)) => {
                    if let Some(msg) = parse_lsp_content(&raw) {
                        if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                            if matches!(
                                method,
                                "textDocument/definition"
                                    | "textDocument/typeDefinition"
                                    | "textDocument/implementation"
                            ) {
                                if let Some(id) = msg.get("id").cloned() {
                                    lsp_info!("[decompile] Tracking {method} request id={id}");
                                    def_ids_in.lock().unwrap().insert(id);
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
    let def_ids_out = Arc::clone(&definition_ids);
    let decompile_writer = Arc::clone(&child_stdin);
    let decompile_pending = Arc::clone(&pending);
    let decompile_counter = Arc::clone(&id_counter);
    let decompile_proxy_id = proxy_id.clone();
    thread::spawn(move || {
        let mut reader = LspReader::new(BufReader::new(child_stdout));
        let stdout = io::stdout();
        while alive_out.load(Ordering::Relaxed) {
            match reader.read_message() {
                Ok(Some(raw)) => {
                    let Some(mut msg) = parse_lsp_content(&raw) else {
                        let mut w = stdout.lock();
                        let _ = w.write_all(&raw);
                        let _ = w.flush();
                        continue;
                    };

                    // Route responses to pending HTTP requests
                    if let Some(id) = msg.get("id") {
                        if let Some(tx) = pending_out.lock().unwrap().remove(id) {
                            let _ = tx.send(msg);
                            continue;
                        }
                    }

                    // Rewrite jdt:// URIs in definition responses
                    // Spawns a thread so this loop stays unblocked and can
                    // route the java/classFileContents response back via `pending`.
                    if let Some(id) = msg.get("id").cloned() {
                        if def_ids_out.lock().unwrap().remove(&id) {
                            lsp_info!("[decompile] Intercepted response id={id}, result: {}", 
                                serde_json::to_string(msg.get("result").unwrap_or(&Value::Null)).unwrap_or_default());
                            let writer = Arc::clone(&decompile_writer);
                            let pending = Arc::clone(&decompile_pending);
                            let pid = decompile_proxy_id.clone();
                            let counter = Arc::clone(&decompile_counter);
                            thread::spawn(move || {
                                let mut next_id = move || {
                                    let seq = counter.fetch_add(1, Ordering::Relaxed);
                                    Value::String(format!("{pid}-decompile-{seq}"))
                                };
                                rewrite_jdt_locations(
                                    &mut msg,
                                    &writer,
                                    &pending,
                                    &mut next_id,
                                );
                                let out = encode_lsp(&msg);
                                let mut w = io::stdout().lock();
                                let _ = w.write_all(out.as_bytes());
                                let _ = w.flush();
                            });
                            continue;
                        }
                    }

                    // Sort completion responses by param count
                    if should_sort_completions(&msg) {
                        sort_completions_by_param_count(&mut msg);
                        let out = encode_lsp(&msg);
                        let mut w = stdout.lock();
                        let _ = w.write_all(out.as_bytes());
                        let _ = w.flush();
                        continue;
                    }

                    // Passthrough
                    let mut w = stdout.lock();
                    let _ = w.write_all(&raw);
                    let _ = w.flush();
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
