use std::{
    fs::{self},
    path::Path,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use zed_extension_api::{
    self as zed,
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
pub struct LspClient {
    workspace: String,
}

impl LspClient {
    pub fn new(workspace: String) -> LspClient {
        LspClient { workspace }
    }

    pub fn resolve_class_path(&self, args: Vec<Option<String>>) -> zed::Result<Vec<Vec<String>>> {
        self.request::<Vec<Vec<String>>>(
            "workspace/executeCommand",
            json!({
                "command": "vscode.java.resolveClasspath",
                "arguments": args
            }),
        )
    }

    pub fn resolve_main_class(&self, args: Vec<String>) -> zed::Result<Vec<MainClassEntry>> {
        self.request::<Vec<MainClassEntry>>(
            "workspace/executeCommand",
            json!({
                "command": "vscode.java.resolveMainClass",
                "arguments": args
            }),
        )
    }

    pub fn request<T>(&self, method: &str, params: Value) -> Result<T, String>
    where
        T: DeserializeOwned,
    {
        // We cannot cache it because the user may restart the LSP
        let port = {
            let filename = string_to_hex(&self.workspace);

            let port_path = Path::new("proxy").join(filename);

            if !fs::metadata(&port_path).is_ok_and(|file| file.is_file()) {
                return Err("Failed to find lsp port file".to_owned());
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

        let data: LspResponse<T> = serde_json::from_slice(&res.body)
            .map_err(|e| format!("Failed to parse response from lsp proxy {e}"))?;

        match data {
            LspResponse::Sucess { result } => return Ok(result),
            LspResponse::Error { error } => {
                return Err(format!("{} {} {}", error.code, error.message, error.data));
            }
        }
    }
}
fn string_to_hex(s: &str) -> String {
    let mut hex_string = String::new();
    for byte in s.as_bytes() {
        hex_string.push_str(&format!("{:02x}", byte));
    }
    hex_string
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum LspResponse<T> {
    Sucess { result: T },
    Error { error: LspError },
}

#[derive(Serialize, Deserialize)]
pub struct LspError {
    code: i64,
    message: String,
    data: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainClassEntry {
    pub main_class: String,
    pub project_name: String,
    pub file_path: String,
}
