use crate::lsp::{show_message_to_user, write_to_stdout};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub type FormatterResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Trait representing a Java source code formatter.
/// Formatter implementations handle their own configuration, activation state,
/// and process invocation details.
pub trait Formatter: Send + Sync {
    /// Returns the unique user-facing name of the formatter.
    fn name(&self) -> &'static str;

    /// Updates the formatter's settings based on incoming JSON configuration.
    fn update_config(&mut self, settings: &Value);

    /// Returns whether this formatter is currently enabled.
    fn is_enabled(&self) -> bool;

    /// Formats the provided original text and returns the formatted result.
    fn format(&self, original_text: &str) -> FormatterResult<String>;
}

pub struct GoogleJavaFormatter {
    enabled: bool,
    style: String,
    path: Option<String>,
    java_executable: Option<String>,
    workdir: String,
}

impl GoogleJavaFormatter {
    pub fn new(workdir: &str) -> Self {
        let enabled = std::env::var("GOOGLE_JAVA_FORMAT_ENABLED")
            .map(|v| v == "true")
            .unwrap_or(false);
        let style =
            std::env::var("GOOGLE_JAVA_FORMAT_STYLE").unwrap_or_else(|_| "GOOGLE".to_string());
        let path = std::env::var("GOOGLE_JAVA_FORMAT_BIN").ok();
        let java_executable = std::env::var("JAVA_EXECUTABLE").ok();

        Self {
            enabled,
            style,
            path,
            java_executable,
            workdir: workdir.to_string(),
        }
    }
}

impl Formatter for GoogleJavaFormatter {
    fn name(&self) -> &'static str {
        "google-java-format"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn update_config(&mut self, settings: &Value) {
        if let Some(gjf) = find_google_java_format_config(settings) {
            self.enabled = gjf
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(style) = gjf.get("style").and_then(|v| v.as_str()) {
                self.style = style.to_string();
            }
            if let Some(path) = gjf.get("path").and_then(|v| v.as_str()) {
                self.path = Some(path.to_string());
            }
        } else if is_settings_payload(settings) {
            self.enabled = false;
        }
    }

    fn format(&self, original_text: &str) -> FormatterResult<String> {
        let path = self
            .path
            .clone()
            .or_else(|| find_latest_local_google_java_format(&self.workdir))
            .ok_or_else(|| {
                let msg = "Google Java Format is enabled but the binary was not found. Please restart Zed or reload the workspace to download it.";
                show_message_to_user(msg);
                Box::<dyn std::error::Error + Send + Sync>::from(msg)
            })?;

        let mut cmd = if path.ends_with(".jar") {
            let java_exe = self.java_executable.as_deref().unwrap_or("java");
            let mut c = Command::new(java_exe);
            c.arg("-jar").arg(path);
            c
        } else {
            Command::new(path)
        };
        if self.style == "AOSP" {
            cmd.arg("--aosp");
        }
        cmd.arg("-");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                let msg = format!("Failed to execute google-java-format: {err}. Please check your Java installation or settings.");
                show_message_to_user(&msg);
                err
            })?;
        {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "Failed to open stdin for formatter",
                )
            })?;
            stdin.write_all(original_text.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "Formatter exited with error: {}",
                stderr
            )));
        }

        let formatted = String::from_utf8(output.stdout)?;

        Ok(formatted)
    }
}
fn find_google_java_format_config(value: &Value) -> Option<&Value> {
    if let Some(obj) = value.as_object() {
        if let Some(gjf) = obj.get("google_java_format") {
            return Some(gjf);
        }
        for val in obj.values() {
            if let Some(found) = find_google_java_format_config(val) {
                return Some(found);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for val in arr {
            if let Some(found) = find_google_java_format_config(val) {
                return Some(found);
            }
        }
    }
    None
}

fn find_latest_local_google_java_format(workdir: &str) -> Option<String> {
    let install_dir = std::path::PathBuf::from(workdir).join("google-java-format");
    if !install_dir.exists() {
        return None;
    }
    let mut entries = std::fs::read_dir(&install_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .collect::<Vec<_>>();

    entries.sort();
    entries
        .into_iter()
        .next_back()
        .map(|p| p.to_string_lossy().to_string())
}

fn find_latest_local_palantir_java_format(workdir: &str) -> Option<String> {
    let install_dir = std::path::PathBuf::from(workdir).join("palantir-java-format");
    if !install_dir.exists() {
        return None;
    }
    let mut entries = std::fs::read_dir(&install_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .collect::<Vec<_>>();

    entries.sort();
    entries
        .into_iter()
        .next_back()
        .map(|p| p.to_string_lossy().to_string())
}

pub struct PalantirJavaFormatter {
    enabled: bool,
    path: Option<String>,
    java_executable: Option<String>,
    workdir: String,
}

impl PalantirJavaFormatter {
    pub fn new(workdir: &str) -> Self {
        let enabled = std::env::var("PALANTIR_JAVA_FORMAT_ENABLED")
            .map(|v| v == "true")
            .unwrap_or(false);
        let path = std::env::var("PALANTIR_JAVA_FORMAT_BIN").ok();
        let java_executable = std::env::var("JAVA_EXECUTABLE").ok();

        Self {
            enabled,
            path,
            java_executable,
            workdir: workdir.to_string(),
        }
    }
}

impl Formatter for PalantirJavaFormatter {
    fn name(&self) -> &'static str {
        "palantir-java-format"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn update_config(&mut self, settings: &Value) {
        if let Some(pjf) = find_palantir_java_format_config(settings) {
            self.enabled = pjf
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(path) = pjf.get("path").and_then(|v| v.as_str()) {
                self.path = Some(path.to_string());
            }
        } else if is_settings_payload(settings) {
            self.enabled = false;
        }
    }

    fn format(&self, original_text: &str) -> FormatterResult<String> {
        let path = self
            .path
            .clone()
            .or_else(|| find_latest_local_palantir_java_format(&self.workdir))
            .ok_or_else(|| {
                let msg = "Palantir Java Format is enabled but the binary was not found. Please restart Zed or reload the workspace to download it.";
                show_message_to_user(msg);
                Box::<dyn std::error::Error + Send + Sync>::from(msg)
            })?;

        let mut cmd = if path.ends_with(".jar") {
            let java_exe = self.java_executable.as_deref().unwrap_or("java");
            let mut c = Command::new(java_exe);
            c.arg("-jar").arg(path);
            c
        } else {
            Command::new(path)
        };
        cmd.arg("--palantir").arg("-");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                let msg = format!(
                    "Failed to execute palantir-java-format: {err}. Please check your settings."
                );
                show_message_to_user(&msg);
                err
            })?;
        {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "Failed to open stdin for formatter",
                )
            })?;
            stdin.write_all(original_text.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "Formatter exited with error: {}",
                stderr
            )));
        }

        let formatted = String::from_utf8(output.stdout)?;

        Ok(formatted)
    }
}

fn is_settings_payload(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("settings")
                || map.contains_key("google_java_format")
                || map.contains_key("palantir_java_format")
                || map.contains_key("java")
        }
        _ => false,
    }
}

fn find_palantir_java_format_config(value: &Value) -> Option<&Value> {
    if let Some(obj) = value.as_object() {
        if let Some(pjf) = obj.get("palantir_java_format") {
            return Some(pjf);
        }
        for val in obj.values() {
            if let Some(found) = find_palantir_java_format_config(val) {
                return Some(found);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for val in arr {
            if let Some(found) = find_palantir_java_format_config(val) {
                return Some(found);
            }
        }
    }
    None
}

pub struct FormatterState {
    pub formatters: Mutex<Vec<Box<dyn Formatter>>>,
    pub document_cache: Mutex<HashMap<String, String>>,
}

impl FormatterState {
    pub fn new(workdir: &str) -> Self {
        Self {
            formatters: Mutex::new(vec![
                Box::new(GoogleJavaFormatter::new(workdir)),
                Box::new(PalantirJavaFormatter::new(workdir)),
            ]),
            document_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn handle_did_open(&self, msg: &Value) {
        if let Some(params) = msg.get("params") {
            if let Some(text_document) = params.get("textDocument") {
                if let Some(uri) = text_document.get("uri").and_then(|v| v.as_str()) {
                    if let Some(text) = text_document.get("text").and_then(|v| v.as_str()) {
                        self.document_cache
                            .lock()
                            .unwrap()
                            .insert(uri.to_string(), text.to_string());
                    }
                }
            }
        }
    }

    pub fn handle_did_change(&self, msg: &Value) {
        if let Some(params) = msg.get("params") {
            if let Some(text_document) = params.get("textDocument") {
                if let Some(uri) = text_document.get("uri").and_then(|v| v.as_str()) {
                    if let Some(content_changes) =
                        params.get("contentChanges").and_then(|v| v.as_array())
                    {
                        if let Some(first_change) = content_changes.first() {
                            if let Some(text) = first_change.get("text").and_then(|v| v.as_str()) {
                                self.document_cache
                                    .lock()
                                    .unwrap()
                                    .insert(uri.to_string(), text.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn handle_did_close(&self, msg: &Value) {
        if let Some(params) = msg.get("params") {
            if let Some(text_document) = params.get("textDocument") {
                if let Some(uri) = text_document.get("uri").and_then(|v| v.as_str()) {
                    self.document_cache.lock().unwrap().remove(uri);
                }
            }
        }
    }

    pub fn update_config(&self, settings: &Value) {
        let mut formatters = self.formatters.lock().unwrap();
        for formatter in formatters.iter_mut() {
            formatter.update_config(settings);
        }
    }

    pub fn handle_formatting_request(&self, msg: &Value) -> bool {
        let formatters = self.formatters.lock().unwrap();
        let active_formatter = formatters.iter().find(|f| f.is_enabled());

        let Some(formatter) = active_formatter else {
            return false;
        };

        crate::lsp_info!("Formatting document using {}", formatter.name());

        let Some(id) = msg.get("id") else {
            return false;
        };
        let Some(params) = msg.get("params") else {
            return false;
        };
        let Some(text_document) = params.get("textDocument") else {
            return false;
        };
        let Some(uri) = text_document.get("uri").and_then(|v| v.as_str()) else {
            return false;
        };

        let original_text = self.document_cache.lock().unwrap().get(uri).cloned();

        let original_text = match original_text {
            Some(text) => text,
            None => {
                crate::lsp_error!("Formatting failed: Document not found in cache: {}", uri);
                return false;
            }
        };

        match formatter.format(&original_text) {
            Ok(formatted_text) => {
                let (end_line, end_char) = get_full_range(&original_text);

                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end": { "line": end_line, "character": end_char }
                            },
                            "newText": formatted_text
                        }
                    ]
                });
                write_to_stdout(&response);
            }
            Err(err) => {
                crate::lsp_error!("{} failed: {}", formatter.name(), err);
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32603,
                        "message": format!("{} failed: {}", formatter.name(), err)
                    }
                });
                write_to_stdout(&response);
            }
        }
        true
    }
}

pub fn intercept_capabilities(msg: &mut Value) {
    if let Some(result) = msg.get_mut("result") {
        if let Some(capabilities) = result.get_mut("capabilities") {
            if let Some(obj) = capabilities.as_object_mut() {
                obj.insert("textDocumentSync".to_string(), serde_json::json!(1));
            }
        }
    }
}

fn get_full_range(text: &str) -> (u32, u32) {
    let mut line_count = 0;
    let mut last_line_len = 0;
    for line in text.split('\n') {
        line_count += 1;
        last_line_len = line.chars().count();
    }
    let end_line = if line_count > 0 { line_count - 1 } else { 0 };
    (end_line as u32, last_line_len as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_java_format_dynamic_reconfiguration() {
        let mut formatter = GoogleJavaFormatter::new(".");
        assert!(!formatter.is_enabled());

        let settings = serde_json::json!({
            "google_java_format": {
                "enabled": true,
                "style": "AOSP",
                "path": "/some/path"
            }
        });

        formatter.update_config(&settings);
        assert!(formatter.is_enabled());
        assert_eq!(formatter.style, "AOSP");
        assert_eq!(formatter.path.unwrap(), "/some/path");
    }

    #[test]
    fn test_document_cache_lifecycle() {
        let state = FormatterState::new(".");
        let uri = "file:///home/test/Main.java";

        let open_msg = serde_json::json!({
            "params": {
                "textDocument": {
                    "uri": uri,
                    "text": "public class Main {}"
                }
            }
        });
        state.handle_did_open(&open_msg);
        assert_eq!(
            state.document_cache.lock().unwrap().get(uri).unwrap(),
            "public class Main {}"
        );

        let change_msg = serde_json::json!({
            "params": {
                "textDocument": {
                    "uri": uri
                },
                "contentChanges": [
                    {
                        "text": "public class Main {\n  // Changed\n}"
                    }
                ]
            }
        });
        state.handle_did_change(&change_msg);
        assert_eq!(
            state.document_cache.lock().unwrap().get(uri).unwrap(),
            "public class Main {\n  // Changed\n}"
        );

        let close_msg = serde_json::json!({
            "params": {
                "textDocument": {
                    "uri": uri
                }
            }
        });
        state.handle_did_close(&close_msg);
        assert!(state.document_cache.lock().unwrap().get(uri).is_none());
    }

    #[test]
    fn test_intercept_capabilities() {
        let mut capabilities = serde_json::json!({
            "result": {
                "capabilities": {
                    "textDocumentSync": 2
                }
            }
        });
        intercept_capabilities(&mut capabilities);
        assert_eq!(
            capabilities["result"]["capabilities"]["textDocumentSync"],
            1
        );
    }

    #[test]
    fn test_formatting_request_when_disabled() {
        let state = FormatterState::new(".");
        let request = serde_json::json!({
            "id": 1,
            "params": {
                "textDocument": {
                    "uri": "file:///home/test/Main.java"
                }
            }
        });
        // Formatter is disabled by default, so it should return false
        assert!(!state.handle_formatting_request(&request));
    }

    #[test]
    fn test_get_full_range() {
        assert_eq!(get_full_range(""), (0, 0));
        assert_eq!(get_full_range("hello"), (0, 5));
        assert_eq!(get_full_range("hello\nworld"), (1, 5));
        assert_eq!(get_full_range("hello\nworld\n"), (2, 0));
    }

    #[test]
    fn test_formatter_state_update_config() {
        let state = FormatterState::new(".");
        let settings = serde_json::json!({
            "google_java_format": {
                "enabled": true,
                "style": "AOSP"
            }
        });
        state.update_config(&settings);

        let formatters = state.formatters.lock().unwrap();
        assert!(formatters[0].is_enabled());
    }

    #[test]
    fn test_palantir_dynamic_reconfiguration() {
        let mut formatter = PalantirJavaFormatter::new(".");
        assert!(!formatter.is_enabled());

        let settings = serde_json::json!({
            "palantir_java_format": {
                "enabled": true,
                "path": "/some/palantir/path"
            }
        });

        formatter.update_config(&settings);
        assert!(formatter.is_enabled());
        assert_eq!(formatter.path.unwrap(), "/some/palantir/path");
    }

    #[test]
    fn test_find_config_nested() {
        let settings_arr = serde_json::json!([
            {
                "other_key": 1
            },
            {
                "google_java_format": {
                    "enabled": true
                }
            }
        ]);
        assert!(find_google_java_format_config(&settings_arr).is_some());

        let settings_nested = serde_json::json!({
            "level1": {
                "level2": {
                    "google_java_format": {
                        "style": "GOOGLE"
                    }
                }
            }
        });
        assert_eq!(
            find_google_java_format_config(&settings_nested).unwrap()["style"],
            "GOOGLE"
        );
    }

    #[test]
    fn test_disable_formatters() {
        let state = FormatterState::new(".");
        let settings_enabled = serde_json::json!({
            "google_java_format": {
                "enabled": true
            }
        });
        state.update_config(&settings_enabled);
        {
            let formatters = state.formatters.lock().unwrap();
            assert!(formatters[0].is_enabled());
            assert!(!formatters[1].is_enabled());
        }
        let settings_commented = serde_json::json!({
            "settings": {
                "java": {}
            }
        });
        state.update_config(&settings_commented);
        {
            let formatters = state.formatters.lock().unwrap();
            assert!(!formatters[0].is_enabled());
            assert!(!formatters[1].is_enabled());
        }
    }
}
