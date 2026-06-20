//! LSP message helpers shared across the bridge: build-file/save detectors,
//! build-evaluation diagnostics parsing, and the [`EditorChannel`] that owns the
//! single byte stream to the editor and merges diagnostics from two sources.
//!
//! Most of this is byte-level and synchronous (ported verbatim from the original
//! single-binary proxy); only [`EditorChannel`] is async because it writes to
//! the editor over tokio's stdout.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use tokio::io::{AsyncWriteExt, Stdout};
use tokio::sync::Mutex;

use proxy_common::{contains_subslice, encode_lsp, lsp_body, parse_lsp_content, path_to_file_uri};

/// Prefix for the JSON-RPC `id` of requests the bridge injects into the language
/// server (the build-model sync commands). Responses carry the same id, letting
/// us recognize and drop them so they are never forwarded to the editor, which
/// never issued them.
pub const INJECTED_ID_PREFIX: &str = "gradle-sync-";

/// Quick check for the `"initialized"` method in the LSP message body.
pub fn is_initialized_notification(raw: &[u8]) -> bool {
    let Some(body) = lsp_body(raw) else {
        return false;
    };
    body.windows(13).any(|w| w == b"\"initialized\"")
}

/// Detect a `textDocument/didSave` notification for a Gradle build file
/// (`*.gradle` / `*.gradle.kts`). Saving a build file can change the resolved
/// build model, so we re-run the sync to refresh the language server's
/// plugins/closures/classpaths.
pub fn is_gradle_build_file_save(raw: &[u8]) -> bool {
    let Some(body) = lsp_body(raw) else {
        return false;
    };
    // `.gradle` also matches `.gradle.kts`.
    contains_subslice(body, b"\"textDocument/didSave\"") && contains_subslice(body, b".gradle")
}

/// Whether a raw LSP message is a response to one of the bridge's injected
/// requests, identified by a string `id` beginning with [`INJECTED_ID_PREFIX`].
/// Such responses must not reach the editor, which never sent the corresponding
/// request.
///
/// A cheap byte pre-check (does the body even mention the prefix?) gates a
/// proper JSON parse of the `id` field, so the common case — the vast majority
/// of messages, which don't contain the prefix at all — stays allocation-free,
/// while matches are confirmed without depending on the server's exact
/// whitespace around `"id":` (compact today, but not guaranteed).
pub fn is_injected_response(raw: &[u8]) -> bool {
    let Some(body) = lsp_body(raw) else {
        return false;
    };
    if !contains_subslice(body, INJECTED_ID_PREFIX.as_bytes()) {
        return false;
    }
    parse_lsp_content(raw)
        .and_then(|msg| msg.get("id")?.as_str().map(str::to_string))
        .is_some_and(|id| id.starts_with(INJECTED_ID_PREFIX))
}

/// If `raw` is a `textDocument/publishDiagnostics` notification, return its
/// `(uri, diagnostics)`. Returns `None` for any other message.
pub fn parse_publish_diagnostics(raw: &[u8]) -> Option<(String, Vec<Value>)> {
    let msg = parse_lsp_content(raw)?;
    if msg.get("method")?.as_str()? != "textDocument/publishDiagnostics" {
        return None;
    }
    let params = msg.get("params")?;
    let uri = params.get("uri")?.as_str()?.to_string();
    let diagnostics = params.get("diagnostics")?.as_array()?.clone();
    Some((uri, diagnostics))
}

/// Build the `uri -> [diagnostic]` map for a build-evaluation failure.
///
/// `error` is the top-level message (typically the gRPC `Status` message or a
/// `compatibility_check_error`); `causes` are appended line by line. The target
/// file and line/column are parsed from the Gradle message when present
/// (`build file '…': N:` and `@ line N, column C`), otherwise the diagnostic is
/// attached at the top of `build_file` when provided.
pub fn build_eval_diagnostics(
    error: &str,
    causes: &[String],
    build_file: Option<&str>,
) -> HashMap<String, Vec<Value>> {
    let mut message = error.to_string();
    for cause in causes {
        message.push('\n');
        message.push_str(cause);
    }

    let parsed_path = parse_build_file_path(&message);
    let path = parsed_path.as_deref().or(build_file);
    let Some(path) = path else {
        return HashMap::new();
    };

    let (line, character) = parse_line_column(&message).unwrap_or((0, 0));
    let diagnostic = serde_json::json!({
        "range": {
            "start": { "line": line, "character": character },
            "end": { "line": line, "character": character.saturating_add(1) }
        },
        "severity": 1,
        "source": "Gradle",
        "message": message
    });

    let uri = path_to_file_uri(Path::new(path));
    let mut map = HashMap::new();
    map.insert(uri, vec![diagnostic]);
    map
}

/// Parse the build-file path from a Gradle error message. Gradle prints either
/// `build file '/abs/path/build.gradle'` or `Build file '/abs/path/build.gradle'`.
pub fn parse_build_file_path(message: &str) -> Option<String> {
    for marker in ["build file '", "Build file '"] {
        if let Some(start) = message.find(marker) {
            let rest = &message[start + marker.len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Parse a zero-based `(line, column)` from a Gradle error message. Gradle
/// reports 1-based positions as `@ line N, column C` or `line: N`; we convert to
/// the 0-based positions LSP expects. Returns `None` if no line is found.
pub fn parse_line_column(message: &str) -> Option<(u64, u64)> {
    if let Some(idx) = message.find("@ line ") {
        let rest = &message[idx + "@ line ".len()..];
        let line = take_u64(rest)?;
        let column = rest
            .find("column ")
            .and_then(|c| take_u64(&rest[c + "column ".len()..]))
            .unwrap_or(1);
        return Some((line.saturating_sub(1), column.saturating_sub(1)));
    }
    if let Some(idx) = message.find("line: ") {
        let line = take_u64(&message[idx + "line: ".len()..])?;
        return Some((line.saturating_sub(1), 0));
    }
    None
}

/// Read the leading run of ASCII digits as a `u64`.
fn take_u64(s: &str) -> Option<u64> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Owns the single byte stream to the editor (the bridge's stdout) and the
/// diagnostics merge state.
///
/// `textDocument/publishDiagnostics` *replaces* a URI's diagnostics for the
/// publishing server, and to the editor the bridge is one server. So both the
/// language server (Groovy syntax errors) and the bridge's own build-model sync
/// (Gradle evaluation errors) must publish through here; the channel keeps each
/// source's diagnostics per URI and always emits their union, so neither erases
/// the other.
pub struct EditorChannel {
    inner: Mutex<EditorChannelInner>,
}

struct EditorChannelInner {
    stdout: Stdout,
    /// Diagnostics last published by the language server, per URI.
    server: HashMap<String, Vec<Value>>,
    /// Diagnostics derived from the build-model sync, per URI.
    sync: HashMap<String, Vec<Value>>,
}

impl EditorChannel {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(EditorChannelInner {
                stdout: tokio::io::stdout(),
                server: HashMap::new(),
                sync: HashMap::new(),
            }),
        }
    }

    /// Forward a raw LSP message to the editor verbatim. Returns false if the
    /// write failed (editor side closed).
    pub async fn forward_raw(&self, raw: &[u8]) -> bool {
        let mut inner = self.inner.lock().await;
        inner.stdout.write_all(raw).await.is_ok() && inner.stdout.flush().await.is_ok()
    }

    /// Record the language server's diagnostics for `uri` and re-emit the merged
    /// set. Called instead of forwarding the server's raw publishDiagnostics.
    pub async fn set_server_diagnostics(&self, uri: String, diagnostics: Vec<Value>) {
        let mut inner = self.inner.lock().await;
        inner.server.insert(uri.clone(), diagnostics);
        inner.publish_merged(&uri).await;
    }

    /// Replace all build-model-sync diagnostics with `next` (URI -> diagnostics).
    /// Any URI that previously had sync diagnostics but is absent from `next` is
    /// cleared. Re-emits the merged set for every affected URI.
    pub async fn set_sync_diagnostics(&self, next: HashMap<String, Vec<Value>>) {
        let mut inner = self.inner.lock().await;
        let mut affected: Vec<String> = next.keys().cloned().collect();
        for uri in inner.sync.keys() {
            if !next.contains_key(uri) {
                affected.push(uri.clone());
            }
        }
        inner.sync = next;
        for uri in affected {
            inner.publish_merged(&uri).await;
        }
    }
}

impl EditorChannelInner {
    /// Emit `textDocument/publishDiagnostics` for `uri` carrying the union of the
    /// server's and the sync's diagnostics.
    async fn publish_merged(&mut self, uri: &str) {
        let mut merged: Vec<Value> = Vec::new();
        if let Some(d) = self.server.get(uri) {
            merged.extend(d.iter().cloned());
        }
        if let Some(d) = self.sync.get(uri) {
            merged.extend(d.iter().cloned());
        }
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": merged }
        });
        let encoded = encode_lsp(&msg);
        let _ = self.stdout.write_all(encoded.as_bytes()).await;
        let _ = self.stdout.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a JSON body in LSP framing the way the language server transmits it.
    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn detects_injected_response_with_string_id() {
        let raw = frame(r#"{"jsonrpc":"2.0","id":"gradle-sync-0","result":null}"#);
        assert!(is_injected_response(&raw));

        // Tolerant of whitespace around the colon (compact today, but the parse
        // path must not depend on it).
        let spaced = frame(r#"{"jsonrpc": "2.0", "id": "gradle-sync-7", "result": null}"#);
        assert!(is_injected_response(&spaced));
    }

    #[test]
    fn does_not_drop_genuine_editor_responses() {
        let raw = frame(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert!(!is_injected_response(&raw));

        let raw = frame(r#"{"jsonrpc":"2.0","id":"client-42","result":"gradle-sync-x"}"#);
        assert!(!is_injected_response(&raw));
    }

    #[test]
    fn detects_build_file_saves() {
        let save = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":"file:///p/build.gradle"}}}"#,
        );
        assert!(is_gradle_build_file_save(&save));

        let kts = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":"file:///p/build.gradle.kts"}}}"#,
        );
        assert!(is_gradle_build_file_save(&kts));
    }

    #[test]
    fn ignores_non_gradle_and_non_save() {
        let java_save = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":"file:///p/Main.java"}}}"#,
        );
        assert!(!is_gradle_build_file_save(&java_save));

        let open = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///p/build.gradle"}}}"#,
        );
        assert!(!is_gradle_build_file_save(&open));
    }

    #[test]
    fn wrapper_properties_save_is_not_a_build_file_save() {
        // `gradle-wrapper.properties` is a `.properties` file, not a build
        // script, so it must not trigger a build-model re-sync.
        let save = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":"file:///p/gradle/wrapper/gradle-wrapper.properties"}}}"#,
        );
        assert!(!is_gradle_build_file_save(&save));
    }

    #[test]
    fn detects_initialized_notification() {
        let init = frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);
        assert!(is_initialized_notification(&init));
    }

    #[test]
    fn parses_publish_diagnostics() {
        let raw = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///p/build.gradle","diagnostics":[{"message":"x"}]}}"#,
        );
        let (uri, diags) = parse_publish_diagnostics(&raw).expect("should parse");
        assert_eq!(uri, "file:///p/build.gradle");
        assert_eq!(diags.len(), 1);

        let other = frame(r#"{"jsonrpc":"2.0","method":"window/logMessage","params":{}}"#);
        assert!(parse_publish_diagnostics(&other).is_none());
    }

    #[test]
    fn parses_build_file_path_and_line_column() {
        let msg = "Could not compile build file '/Users/me/proj/build.gradle'.\nstartup failed:\nbuild file '/Users/me/proj/build.gradle': 9: Unexpected input: '{' @ line 9, column 6.";
        assert_eq!(
            parse_build_file_path(msg).as_deref(),
            Some("/Users/me/proj/build.gradle")
        );
        assert_eq!(parse_line_column(msg), Some((8, 5)));
    }

    #[test]
    fn parses_line_only_form() {
        let msg = "build file '/p/build.gradle' line: 12";
        assert_eq!(parse_line_column(msg), Some((11, 0)));
    }

    #[test]
    fn parses_kotlin_dsl_build_failure() {
        // The shape gradle-server emits for a Kotlin-DSL script error, captured
        // from its stderr: capital "Build file '…'" with a `line: N` marker.
        let msg = "FAILURE: Build failed with an exception.\n* Where:\nBuild file '/Users/me/proj/build.gradle.kts' line: 4\n* What went wrong:\nScript compilation error:\n  Line 4:     adewdw\n              ^ Unresolved reference 'adewdw'.";
        assert_eq!(
            parse_build_file_path(msg).as_deref(),
            Some("/Users/me/proj/build.gradle.kts")
        );
        // 1-based "line: 4" -> 0-based line 4 -> 3.
        assert_eq!(parse_line_column(msg), Some((3, 0)));
    }

    #[test]
    fn no_location_for_methodless_errors() {
        let msg =
            "Could not find method implementatoin() for arguments [com.google.gwt:gwt:2.10.0]";
        assert_eq!(parse_build_file_path(msg), None);
        assert_eq!(parse_line_column(msg), None);
    }

    #[test]
    fn build_eval_diagnostics_uses_parsed_location() {
        let causes = vec![
            "startup failed:\nbuild file '/p/build.gradle': 9: Unexpected input: '{' @ line 9, column 6.".to_string(),
        ];
        let map = build_eval_diagnostics(
            "Could not compile build file '/p/build.gradle'.",
            &causes,
            Some("/p/build.gradle"),
        );
        let diags = map
            .get("file:///p/build.gradle")
            .expect("diag for build file");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0]["range"]["start"]["line"], 8);
        assert_eq!(diags[0]["severity"], 1);
        assert_eq!(diags[0]["source"], "Gradle");
    }

    #[test]
    fn build_eval_diagnostics_falls_back_to_build_file_top() {
        let causes = vec![
            "Could not find method implementatoin() for arguments [com.google.gwt:gwt:2.10.0]"
                .to_string(),
        ];
        let map = build_eval_diagnostics(
            "A problem occurred evaluating root project 'proj'.",
            &causes,
            Some("/p/build.gradle"),
        );
        let diags = map
            .get("file:///p/build.gradle")
            .expect("diag for build file");
        assert_eq!(diags[0]["range"]["start"]["line"], 0);
        assert!(diags[0]["message"]
            .as_str()
            .unwrap()
            .contains("implementatoin()"));
    }
}
