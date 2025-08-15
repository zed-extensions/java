use std::{
    fs::{self},
    path::Path,
};

use zed_extension_api::{
    Worktree,
    http_client::{HttpMethod, HttpRequest, fetch},
    serde_json::{self, Map, Value},
};

/**
 * `proxy.mjs` starts an HTTP server and writes its port to
 * `${workdir}/proxy/${hex(project_root)}`.
 *
 * This allows us to send LSP requests directly from the Java extension.
 * It’s  a temporary workaround until `zed_extension_api`
 * provides the ability to send LSP requests directly.
*/
pub struct LspClient {}

impl LspClient {
    pub fn request(worktree: &Worktree, method: &str, params: Value) -> Result<Value, String> {
        // We cannot cache it because the user may restart the LSP
        let port = {
            let filename = string_to_hex(worktree.root_path().as_str());

            let port_path = Path::new("proxy").join(filename);

            if !fs::metadata(&port_path).is_ok_and(|file| file.is_file()) {
                return Err("Lsp proxy is not running".to_owned());
            }

            fs::read_to_string(port_path)
                .map_err(|e| format!("Failed to read a lsp proxy port from file {e}"))?
                .parse::<u16>()
                .map_err(|e| format!("Failed to read a lsp proxy port, file corrupted {e}"))?
        };

        let mut body = Map::new();
        body.insert("method".to_owned(), Value::String(method.to_owned()));
        body.insert("params".to_owned(), params);

        let res = fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Post)
                .url(format!("http://localhost:{port}"))
                .body(Value::Object(body).to_string())
                .build()?,
        )
        .map_err(|e| format!("Failed to send request to lsp proxy {e}"))?;

        Ok(serde_json::from_slice(&res.body)
            .map_err(|e| format!("Failed to parse response from lsp proxy {e}"))?)
    }
}
fn string_to_hex(s: &str) -> String {
    let mut hex_string = String::new();
    for byte in s.as_bytes() {
        hex_string.push_str(&format!("{:02x}", byte));
    }
    hex_string
}
