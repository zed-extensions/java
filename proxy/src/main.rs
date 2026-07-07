mod completions;
mod decompile;
mod http;
mod log;
mod lsp;
mod platform;

use completions::{is_completion_response, process_completions, sanitize_resolved_completion};
use decompile::{rewrite_jdt_in_strings, rewrite_jdt_locations};
use http::handle_http;
use lsp::{parse_lsp_content, raw_has_id, write_raw, write_to_stdout, LspReader};
use platform::spawn_parent_monitor;
use serde_json::{json, Value};
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
enum RewriteKind {
    Definition,
    Documentation,
}

#[derive(Clone)]
enum FallbackResult {
    Null,
    EmptyArray,
    CompletionItem(Value),
}

#[derive(Clone)]
struct TrackedRequest {
    method: String,
    rewrite: Option<RewriteKind>,
    fallback: FallbackResult,
}

impl TrackedRequest {
    fn new(method: &str, rewrite: Option<RewriteKind>, fallback: FallbackResult) -> Self {
        Self {
            method: method.to_string(),
            rewrite,
            fallback,
        }
    }

    fn fallback_result(&self) -> Value {
        match &self.fallback {
            FallbackResult::Null => Value::Null,
            FallbackResult::EmptyArray => json!([]),
            FallbackResult::CompletionItem(item) => item.clone(),
        }
    }
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

    // Track requests whose responses need rewriting or a soft fallback when
    // JDTLS returns a JSON-RPC internal error.
    let tracked_ids: Arc<Mutex<HashMap<Value, TrackedRequest>>> =
        Arc::new(Mutex::new(HashMap::new()));

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
                            if let Some((id, request)) = tracked_request_for(&msg) {
                                tracked_in.lock().unwrap().insert(id, request);
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

                    // Rewrite jdt:// URIs, or turn known JDTLS internal errors
                    // into harmless fallback results so one bad request doesn't
                    // break Java editing until the language server is restarted.
                    if let Some(id) = msg.get("id").cloned() {
                        if let Some(request) = tracked_out.lock().unwrap().remove(&id) {
                            if is_jdtls_internal_error(&msg) {
                                lsp_warn!(
                                    "JDTLS internal error for {}; returning fallback result",
                                    request.method
                                );
                                write_to_stdout(&fallback_response(id, &request));
                                continue;
                            }

                            let Some(rewrite) = request.rewrite else {
                                write_raw(&mut io::stdout().lock(), &raw);
                                continue;
                            };
                            let sanitize_signature_help =
                                request.method == "textDocument/signatureHelp";

                            // Spawns a thread so this loop stays unblocked and can route
                            // the java/classFileContents response back via `pending`.
                            let writer = Arc::clone(&decompile_writer);
                            let pending = Arc::clone(&decompile_pending);
                            let pid = decompile_proxy_id.clone();
                            let counter = Arc::clone(&decompile_counter);
                            thread::spawn(move || {
                                let mut next_id = move || {
                                    let seq = counter.fetch_add(1, Ordering::Relaxed);
                                    Value::String(format!("{pid}-decompile-{seq}"))
                                };
                                match rewrite {
                                    RewriteKind::Definition => {
                                        rewrite_jdt_locations(
                                            &mut msg,
                                            &writer,
                                            &pending,
                                            &mut next_id,
                                        );
                                    }
                                    RewriteKind::Documentation => {
                                        rewrite_jdt_in_strings(
                                            &mut msg,
                                            &writer,
                                            &pending,
                                            &mut next_id,
                                        );
                                        sanitize_resolved_completion(&mut msg);
                                    }
                                }
                                if sanitize_signature_help {
                                    sanitize_signature_help_response(&mut msg);
                                }
                                write_to_stdout(&msg);
                            });
                            continue;
                        }
                    }

                    // Process completion responses (sort + sanitize) in a single pass
                    if is_completion_response(&msg) {
                        process_completions(&mut msg);
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

fn tracked_request_for(msg: &Value) -> Option<(Value, TrackedRequest)> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone();
    let request = match method {
        "textDocument/definition"
        | "textDocument/typeDefinition"
        | "textDocument/implementation" => {
            TrackedRequest::new(method, Some(RewriteKind::Definition), FallbackResult::Null)
        }
        "textDocument/hover" | "textDocument/signatureHelp" => TrackedRequest::new(
            method,
            Some(RewriteKind::Documentation),
            FallbackResult::Null,
        ),
        "completionItem/resolve" => TrackedRequest::new(
            method,
            Some(RewriteKind::Documentation),
            FallbackResult::CompletionItem(msg.get("params").cloned().unwrap_or(Value::Null)),
        ),
        "textDocument/codeAction" | "textDocument/codeLens" | "textDocument/documentHighlight" => {
            TrackedRequest::new(method, None, FallbackResult::EmptyArray)
        }
        _ => return None,
    };

    Some((id, request))
}

fn is_jdtls_internal_error(msg: &Value) -> bool {
    let Some(error) = msg.get("error") else {
        return false;
    };

    let has_internal_error_code = error
        .get("code")
        .and_then(|code| code.as_i64())
        .is_some_and(|code| code == -32603);
    let has_internal_error_message = error
        .get("message")
        .and_then(|message| message.as_str())
        .is_some_and(|message| message.to_ascii_lowercase().contains("internal error"));

    has_internal_error_code || has_internal_error_message
}

fn fallback_response(id: Value, request: &TrackedRequest) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": request.fallback_result(),
    })
}

fn sanitize_signature_help_response(msg: &mut Value) {
    let Some(result) = msg
        .get_mut("result")
        .and_then(|result| result.as_object_mut())
    else {
        return;
    };

    let has_negative_active_parameter = result
        .get("activeParameter")
        .and_then(|active_parameter| active_parameter.as_i64())
        .is_some_and(|active_parameter| active_parameter < 0);

    if has_negative_active_parameter {
        result.remove("activeParameter");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jdtls_internal_error_by_code() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32603,
                "message": "Request failed"
            }
        });

        assert!(is_jdtls_internal_error(&msg));
    }

    #[test]
    fn detects_jdtls_internal_error_by_message() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": 0,
                "message": "Internal error."
            }
        });

        assert!(is_jdtls_internal_error(&msg));
    }

    #[test]
    fn ignores_non_internal_errors() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32800,
                "message": "Request cancelled"
            }
        });

        assert!(!is_jdtls_internal_error(&msg));
    }

    #[test]
    fn builds_empty_array_fallback_response() {
        let request =
            TrackedRequest::new("textDocument/codeAction", None, FallbackResult::EmptyArray);

        assert_eq!(
            fallback_response(json!(7), &request),
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "result": []
            })
        );
    }

    #[test]
    fn completion_resolve_fallback_returns_original_item() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "completionItem/resolve",
            "params": {
                "label": "String",
                "kind": 7
            }
        });

        let (id, tracked) = tracked_request_for(&request).unwrap();

        assert_eq!(
            fallback_response(id, &tracked),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "label": "String",
                    "kind": 7
                }
            })
        );
    }

    #[test]
    fn removes_negative_signature_help_active_parameter() {
        let mut msg = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "signatures": [],
                "activeSignature": 0,
                "activeParameter": -1
            }
        });

        sanitize_signature_help_response(&mut msg);

        assert!(msg["result"].get("activeParameter").is_none());
    }

    #[test]
    fn preserves_valid_signature_help_active_parameter() {
        let mut msg = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "signatures": [],
                "activeSignature": 0,
                "activeParameter": 1
            }
        });

        sanitize_signature_help_response(&mut msg);

        assert_eq!(msg["result"]["activeParameter"], json!(1));
    }
}
