use serde_json::Value;

/// Returns true if the message contains a completion response with items.
pub fn is_completion_response(msg: &Value) -> bool {
    msg.get("result").is_some_and(|result| {
        result.get("items").is_some_and(|v| v.is_array()) || result.is_array()
    })
}

/// Single-pass processing of completion items:
/// - Sorts methods/functions by parameter count (prepends count to sortText)
/// - Strips unsupported VS Code snippet variables ($TM_SELECTED_TEXT) from snippets
pub fn process_completions(msg: &mut Value) {
    let items = match msg.get_mut("result") {
        Some(result) if result.is_array() => result.as_array_mut(),
        Some(result) => result.get_mut("items").and_then(|v| v.as_array_mut()),
        None => None,
    };

    let Some(items) = items else { return };

    for item in items.iter_mut() {
        let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);

        match kind {
            // Method (2) or Function (3): prepend param count to sortText
            2 | 3 => {
                let detail = item
                    .pointer("/labelDetails/detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let count = count_params(detail);
                let existing = item.get("sortText").and_then(|v| v.as_str()).unwrap_or("");
                item["sortText"] = Value::String(format!("{count:02}{existing}"));
            }
            // Snippet (15): strip $TM_SELECTED_TEXT
            15 => {
                strip_tm_selected_text(item, "textEditText");
                strip_tm_selected_text(item, "insertText");
            }
            _ => {}
        }
    }
}

fn strip_tm_selected_text(item: &mut Value, key: &str) {
    if let Some(text) = item.get(key).and_then(|v| v.as_str()) {
        if text.contains("$TM_SELECTED_TEXT") {
            item[key] = Value::String(text.replace("$TM_SELECTED_TEXT", ""));
        }
    }
}

/// Sanitize a single resolved completion item (completionItem/resolve response).
pub fn sanitize_resolved_completion(msg: &mut Value) {
    let Some(result) = msg.get_mut("result") else {
        return;
    };
    strip_tm_selected_text(result, "textEditText");
    strip_tm_selected_text(result, "insertText");
    // Also check inside textEdit.newText
    if let Some(new_text) = result.pointer("/textEdit/newText").and_then(|v| v.as_str()) {
        if new_text.contains("$TM_SELECTED_TEXT") {
            result["textEdit"]["newText"] =
                Value::String(new_text.replace("$TM_SELECTED_TEXT", ""));
        }
    }
}

fn count_params(detail: &str) -> usize {
    let inner = match detail.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        Some(s) => s.trim(),
        None => return 0,
    };
    if inner.is_empty() {
        return 0;
    }
    let mut count = 1usize;
    let mut depth = 0i32;
    for ch in inner.bytes() {
        match ch {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}
