use serde_json::{json, Value};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    env, fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

use crate::{lsp::encode_lsp, lsp_error, lsp_warn};

const DECOMPILED_DIR: &str = "jdtls-decompiled";

/// Convert a `PathBuf` to a proper `file://` URI.
///
/// On Unix the path already starts with `/`, so `file://` + path gives us
/// the correct `file:///…` form with no extra work.
///
/// On Windows we must replace `\` with `/` and prepend `file:///` before the
/// drive letter so that we get `file:///C:/…` instead of `file://C:\…`.
#[cfg(unix)]
fn path_to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(windows)]
fn path_to_file_uri(path: &Path) -> String {
    let s = path.display().to_string().replace('\\', "/");
    format!("file:///{s}")
}

fn cache_dir() -> PathBuf {
    env::temp_dir().join(DECOMPILED_DIR)
}

fn cache_path(uri: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    uri.hash(&mut hasher);
    let hex = format!("{:016x}", hasher.finish());

    // jdt://contents/java.base/java.util/ArrayList.java?=.../%3Cjava.util%28ArrayList.class
    // The class name is between the last %28 (URL-encoded '(') and .class at the end
    let name = uri
        .rsplit_once("%28")
        .and_then(|(_, rest)| rest.strip_suffix(".class"))
        .or_else(|| {
            uri.split('?')
                .next()
                .and_then(|path| path.rsplit('/').next())
                .and_then(|seg| seg.strip_suffix(".java").or(seg.strip_suffix(".class")))
        })
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
            lsp_warn!("[decompile] Timed out fetching class contents for {uri}");
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
        return Some(path_to_file_uri(&path));
    }

    let content = fetch_class_contents(uri, writer, pending, request_id)?;
    let _ = fs::create_dir_all(cache_dir());
    match fs::write(&path, &content) {
        Ok(_) => Some(path_to_file_uri(&path)),
        Err(e) => {
            lsp_error!("[decompile] Failed to write {}: {e}", path.display());
            None
        }
    }
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

/// A jdt:// URI in embedded markdown/text terminates at whitespace or any of these
/// delimiters commonly used in markdown links and JSON strings. The URI itself only
/// contains URL-encoded forms of these characters, so scanning until we hit one of
/// them is safe.
fn jdt_uri_end(s: &str) -> usize {
    s.find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '"' | '>' | '`' | '\''))
        .unwrap_or(s.len())
}

/// Extract all unique `jdt://` URIs appearing inside any string in `value`.
fn collect_jdt_uris(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            let mut rest = s.as_str();
            while let Some(pos) = rest.find("jdt://") {
                let tail = &rest[pos..];
                let end = jdt_uri_end(tail);
                let uri = tail[..end].to_string();
                if !out.contains(&uri) {
                    out.push(uri);
                }
                rest = &tail[end..];
            }
        }
        Value::Array(arr) => arr.iter().for_each(|v| collect_jdt_uris(v, out)),
        Value::Object(obj) => obj.values().for_each(|v| collect_jdt_uris(v, out)),
        _ => {}
    }
}

/// Replace all occurrences of any key in `map` with its value, inside every string
/// contained in `value` (recursively).
fn replace_in_strings(value: &mut Value, map: &HashMap<String, String>) {
    match value {
        Value::String(s) => {
            for (from, to) in map {
                if s.contains(from.as_str()) {
                    *s = s.replace(from.as_str(), to);
                }
            }
        }
        Value::Array(arr) => arr.iter_mut().for_each(|v| replace_in_strings(v, map)),
        Value::Object(obj) => obj.values_mut().for_each(|v| replace_in_strings(v, map)),
        _ => {}
    }
}

/// Scan a documentation response (hover, signatureHelp, completionItem/resolve, …)
/// for embedded `jdt://` URIs, resolve each one to a `file://` URI backed by a temp
/// file, and replace the URIs in-place in every string of `msg.result`.
pub fn rewrite_jdt_in_strings(
    msg: &mut Value,
    writer: &Arc<Mutex<impl Write>>,
    pending: &Arc<Mutex<HashMap<Value, mpsc::Sender<Value>>>>,
    next_id: &mut impl FnMut() -> Value,
) {
    let Some(result) = msg.get_mut("result") else {
        return;
    };

    let mut uris = Vec::new();
    collect_jdt_uris(result, &mut uris);
    if uris.is_empty() {
        return;
    }

    let mut map = HashMap::new();
    for uri in uris {
        if let Some(file_uri) = resolve_jdt_uri(&uri, writer, pending, next_id()) {
            map.insert(uri, file_uri);
        }
    }
    if !map.is_empty() {
        replace_in_strings(result, &map);
    }
}
