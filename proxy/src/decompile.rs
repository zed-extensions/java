use serde_json::{json, Value};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    env, fs,
    hash::{Hash, Hasher},
    io::Write,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
};

use crate::{lsp::encode_lsp, lsp_error, lsp_info, lsp_warn};

const DECOMPILED_DIR: &str = "jdtls-decompiled";

fn cache_dir() -> PathBuf {
    env::temp_dir().join(DECOMPILED_DIR)
}

fn cache_path(uri: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    uri.hash(&mut hasher);
    let hex = format!("{:016x}", hasher.finish());
    let name = uri
        .rsplit('/')
        .find_map(|seg| seg.strip_suffix(".java?").or(seg.strip_suffix(".class?")))
        .unwrap_or("Decompiled");
    cache_dir().join(format!("{name}-{hex}.java"))
}

/// Send `java/classFileContents` to JDTLS and wait for the response.
fn fetch_class_contents(
    uri: &str,
    writer: &Arc<Mutex<impl Write>>,
    pending: &Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>>,
    request_id: Value,
) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(request_id.clone(), tx);

    let req = encode_lsp(&json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "java/classFileContents",
        "params": { "uri": uri }
    }));
    {
        let mut w = writer.lock().unwrap();
        let _ = w.write_all(req.as_bytes());
        let _ = w.flush();
    }

    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(resp) => {
            let content = resp.get("result")?.as_str()?;
            Some(content.to_string())
        }
        Err(_) => {
            lsp_warn!("[decompile] Timed out waiting for java/classFileContents response");
            None
        }
    }
}

fn resolve_jdt_uri(
    uri: &str,
    writer: &Arc<Mutex<impl Write>>,
    pending: &Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>>,
    request_id: Value,
) -> Option<String> {
    let path = cache_path(uri);
    if path.exists() {
        lsp_info!("[decompile] Cache hit: {}", path.display());
        return Some(format!("file://{}", path.display()));
    }

    lsp_info!("[decompile] Cache miss, fetching: {uri}");
    let content = fetch_class_contents(uri, writer, pending, request_id)?;
    let _ = fs::create_dir_all(cache_dir());
    match fs::write(&path, &content) {
        Ok(_) => lsp_info!("[decompile] Wrote {} bytes to {}", content.len(), path.display()),
        Err(e) => {
            lsp_error!("[decompile] Failed to write {}: {e}", path.display());
            return None;
        }
    }
    Some(format!("file://{}", path.display()))
}

/// Rewrite any `jdt://` URIs in a definition/typeDefinition/implementation response.
/// Returns `true` if any URI was rewritten.
pub fn rewrite_jdt_locations(
    msg: &mut Value,
    writer: &Arc<Mutex<impl Write>>,
    pending: &Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>>,
    next_id: &mut impl FnMut() -> Value,
) -> bool {
    let results = match msg.get_mut("result") {
        Some(Value::Array(arr)) => arr.iter_mut().collect::<Vec<_>>(),
        Some(obj @ Value::Object(_)) => vec![obj],
        _ => return false,
    };

    let mut rewritten = false;
    for loc in results {
        for key in &["uri", "targetUri"] {
            if let Some(Value::String(uri)) = loc.get(key) {
                if uri.starts_with("jdt://") {
                    let jdt_uri = uri.clone();
                    if let Some(file_uri) = resolve_jdt_uri(&jdt_uri, writer, pending, next_id()) {
                        loc[*key] = Value::String(file_uri);
                        rewritten = true;
                    }
                }
            }
        }
    }
    rewritten
}
