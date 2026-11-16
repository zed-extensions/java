use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use crate::pending::PendingResponses;
use proxy_common::encode_lsp;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const BUILD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct HttpBody {
    method: String,
    params: Value,
}

impl HttpBody {
    fn timeout(&self) -> Duration {
        if self.method == "workspace/executeCommand"
            && self.params.get("command").and_then(Value::as_str)
                == Some("vscode.java.buildWorkspace")
        {
            BUILD_TIMEOUT
        } else {
            DEFAULT_TIMEOUT
        }
    }
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
    pending: Arc<PendingResponses>,
    counter: Arc<AtomicU64>,
    owned_id_prefix: &str,
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
    let id = Value::String(format!("{owned_id_prefix}http-{seq}"));
    let rx = pending.register(id.clone());
    let timeout = req.timeout();

    let lsp_req = LspRequest {
        jsonrpc: "2.0",
        id: id.clone(),
        method: req.method,
        params: req.params,
    };
    let encoded = encode_lsp(&lsp_req);
    let request_sent = {
        let mut w = writer.lock().unwrap();
        w.write_all(encoded.as_bytes()).is_ok() && w.flush().is_ok()
    };

    let response = if request_sent {
        match rx.recv_timeout(timeout) {
            Ok(resp) => resp,
            Err(_) => {
                pending.remove(&id);
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
                        "message": format!(
                            "Request to language server timed out after {}ms.",
                            timeout.as_millis()
                        )
                    }
                })
            }
        }
    } else {
        pending.remove(&id);
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32603,
                "message": "Failed to write request to language server."
            }
        })
    };

    let resp_body = serde_json::to_vec(&response).unwrap();
    let http_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        resp_body.len()
    );
    let _ = stream.write_all(http_resp.as_bytes());
    let _ = stream.write_all(&resp_body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proxy_common::{parse_lsp_content, LspReader};
    use serde_json::json;
    use std::{
        net::{TcpListener, TcpStream},
        thread::{self, JoinHandle},
    };

    struct HttpExchange {
        client: TcpStream,
        lsp: LspReader<TcpStream>,
        pending: Arc<PendingResponses>,
        handler: JoinHandle<()>,
    }

    impl HttpExchange {
        fn start(method: &str, params: Value) -> Self {
            let (mut client, server) = connected_streams();
            let (writer, lsp) = connected_streams();
            let pending = Arc::new(PendingResponses::new());
            let handler_pending = pending.clone();
            let handler = thread::spawn(move || {
                handle_http(
                    server,
                    Arc::new(Mutex::new(writer)),
                    handler_pending,
                    Arc::new(AtomicU64::new(1)),
                    "test-",
                );
            });
            let body = json!({ "method": method, "params": params }).to_string();
            write!(
                client,
                "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();

            Self {
                client,
                lsp: LspReader::new(lsp),
                pending,
                handler,
            }
        }

        fn next_lsp_message(&mut self) -> Value {
            let message = self.lsp.read_message().unwrap().unwrap();
            parse_lsp_content(&message).unwrap()
        }

        fn finish(mut self) -> Value {
            let mut response = String::new();
            self.client.read_to_string(&mut response).unwrap();
            self.handler.join().unwrap();
            let (headers, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("HTTP/1.1 200 OK"));
            serde_json::from_str(body).unwrap()
        }
    }

    fn connected_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        for stream in [&client, &server] {
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
        }
        (client, server)
    }

    #[test]
    fn build_succeeding_after_the_default_timeout_returns_success() {
        let mut exchange = HttpExchange::start(
            "workspace/executeCommand",
            json!({
                "command": "vscode.java.buildWorkspace",
                "arguments": ["{\"mainClass\":\"example.Main\",\"isFullBuild\":false}"],
            }),
        );
        let request = exchange.next_lsp_message();

        thread::sleep(DEFAULT_TIMEOUT + Duration::from_secs(1));
        let response = json!({ "jsonrpc": "2.0", "id": request["id"], "result": 1 });
        assert!(exchange.pending.route(&response));

        assert_eq!(exchange.finish(), response);
    }

    #[test]
    fn ordinary_request_timeout_cancels_the_request_and_reports_its_duration() {
        let mut exchange = HttpExchange::start(
            "workspace/executeCommand",
            json!({ "command": "vscode.java.resolveMainMethod" }),
        );
        let request = exchange.next_lsp_message();

        let cancellation = exchange.next_lsp_message();
        let response = exchange.finish();

        assert_eq!(cancellation["method"], "$/cancelRequest");
        assert_eq!(cancellation["params"]["id"], request["id"]);
        assert_eq!(response["id"], request["id"]);
        assert_eq!(response["error"]["code"], -32803);
        assert_eq!(
            response["error"]["message"],
            "Request to language server timed out after 5000ms."
        );
    }

    #[test]
    fn build_command_in_another_method_keeps_the_default_timeout() {
        let request = HttpBody {
            method: "textDocument/hover".into(),
            params: json!({ "command": "vscode.java.buildWorkspace" }),
        };

        assert_eq!(request.timeout(), DEFAULT_TIMEOUT);
    }
}
