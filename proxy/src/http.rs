use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use crate::lsp::encode_lsp;

pub const TIMEOUT: Duration = Duration::from_secs(5);

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

pub fn handle_http(
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
