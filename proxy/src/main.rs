mod log;
mod platform;

use platform::spawn_parent_monitor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{self, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

const CONTENT_LENGTH: &str = "Content-Length";
const HEADER_SEP: &[u8] = b"\r\n\r\n";
const TIMEOUT: Duration = Duration::from_secs(5);

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
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

    // --- Thread 1: Zed stdin -> JDTLS stdin (passthrough) ---
    let stdin_writer = Arc::clone(&child_stdin);
    let alive_stdin = Arc::clone(&alive);
    thread::spawn(move || {
        let stdin = io::stdin().lock();
        let mut reader = LspReader::new(stdin);
        while alive_stdin.load(Ordering::Relaxed) {
            match reader.read_message() {
                Ok(Some(raw)) => {
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

    // --- Thread 2: JDTLS stdout -> modify completions -> Zed stdout / resolve pending ---
    let pending_out = Arc::clone(&pending);
    let alive_out = Arc::clone(&alive);
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

// --- LSP message reader ---

struct LspReader<R> {
    reader: R,
}

impl<R: Read> LspReader<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }

    fn read_message(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut header_buf = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match self.reader.read(&mut byte) {
                Ok(0) => return Ok(None),
                Ok(_) => header_buf.push(byte[0]),
                Err(e) => return Err(e),
            }
            if header_buf.ends_with(HEADER_SEP) {
                break;
            }
        }

        let header_str = String::from_utf8_lossy(&header_buf);
        let content_length = header_str
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(": ")?;
                if name.eq_ignore_ascii_case(CONTENT_LENGTH) {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let mut content = vec![0u8; content_length];
        self.reader.read_exact(&mut content)?;

        let mut message = header_buf;
        message.extend_from_slice(&content);
        Ok(Some(message))
    }
}

// --- LSP message encoding/parsing ---

fn parse_lsp_content(raw: &[u8]) -> Option<Value> {
    let sep_pos = raw.windows(4).position(|w| w == HEADER_SEP)?;
    serde_json::from_slice(&raw[sep_pos + 4..]).ok()
}

fn encode_lsp(value: &impl Serialize) -> String {
    let json = serde_json::to_string(value).unwrap();
    format!("{CONTENT_LENGTH}: {}\r\n\r\n{json}", json.len())
}

// --- Completion sorting ---

fn should_sort_completions(msg: &Value) -> bool {
    msg.get("result").is_some_and(|result| {
        result.get("items").is_some_and(|v| v.is_array()) || result.is_array()
    })
}

fn sort_completions_by_param_count(msg: &mut Value) {
    let items = if let Some(result) = msg.get_mut("result") {
        if result.is_array() {
            result.as_array_mut()
        } else {
            result.get_mut("items").and_then(|v| v.as_array_mut())
        }
    } else {
        None
    };

    if let Some(items) = items {
        for item in items.iter_mut() {
            let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
            if kind == 2 || kind == 3 {
                let detail = item
                    .pointer("/labelDetails/detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let count = count_params(detail);
                let existing = item.get("sortText").and_then(|v| v.as_str()).unwrap_or("");
                item["sortText"] = Value::String(format!("{count:02}{existing}"));
            }
        }
    }
}

fn count_params(detail: &str) -> usize {
    if detail.is_empty() || detail == "()" {
        return 0;
    }
    let inner = detail
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(detail)
        .trim();
    if inner.is_empty() {
        return 0;
    }
    let mut count = 1usize;
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

// --- HTTP handler ---

#[derive(Deserialize)]
struct HttpBody {
    method: String,
    params: Value,
}

#[derive(Serialize)]
struct LspRequest {
    jsonrpc: &'static str,
    id: Value,
    method: String,
    params: Value,
}

fn handle_http(
    mut stream: std::net::TcpStream,
    writer: Arc<Mutex<impl Write>>,
    pending: Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>>,
    counter: Arc<AtomicU64>,
    proxy_id: &str,
) {
    let mut reader = BufReader::new(&stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    if !request_line.starts_with("POST") {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return;
    }

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(": ") {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }

    let req: HttpBody = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            return;
        }
    };

    let seq = counter.fetch_add(1, Ordering::Relaxed);
    let id = Value::String(format!("{proxy_id}-{seq}"));

    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(id.clone(), tx);

    let lsp_req = LspRequest {
        jsonrpc: "2.0",
        id: id.clone(),
        method: req.method,
        params: req.params,
    };
    let encoded = encode_lsp(&lsp_req);
    {
        let mut w = writer.lock().unwrap();
        let _ = w.write_all(encoded.as_bytes());
        let _ = w.flush();
    }

    let response = match rx.recv_timeout(TIMEOUT) {
        Ok(resp) => resp,
        Err(_) => {
            pending.lock().unwrap().remove(&id);
            let cancel = encode_lsp(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "$/cancelRequest",
                "params": { "id": id }
            }));
            let mut w = writer.lock().unwrap();
            let _ = w.write_all(cancel.as_bytes());
            let _ = w.flush();

            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32803,
                    "message": "Request to language server timed out after 5000ms."
                }
            })
        }
    };

    let resp_body = serde_json::to_vec(&response).unwrap();
    let http_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        resp_body.len()
    );
    let _ = stream.write_all(http_resp.as_bytes());
    let _ = stream.write_all(&resp_body);
}

// --- Utilities ---

fn hex_encode(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
