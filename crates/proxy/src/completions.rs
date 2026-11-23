use serde_json::Value;

/// Single-pass processing of completion items:
/// - Sorts methods/functions by parameter count (prepends count to sortText)
/// - Strips unsupported VS Code snippet variables ($TM_SELECTED_TEXT) from snippets
pub fn process_completions(msg: &mut Value) {
    let default_insert_text_format = msg
        .pointer("/result/itemDefaults/insertTextFormat")
        .and_then(Value::as_u64);
    let items = match msg.get_mut("result") {
        Some(result) if result.is_array() => result.as_array_mut(),
        Some(result) => result.get_mut("items").and_then(Value::as_array_mut),
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
            _ => {}
        }

        let insert_text_format = item
            .get("insertTextFormat")
            .and_then(Value::as_u64)
            .or(default_insert_text_format);
        if kind == 15 || insert_text_format == Some(2) {
            sanitize_completion_item(item);
        }
    }
}

fn sanitize_completion_item(item: &mut Value) {
    strip_tm_selected_text(item, "/textEditText");
    strip_tm_selected_text(item, "/insertText");
    strip_tm_selected_text(item, "/textEdit/newText");
}

fn strip_tm_selected_text(item: &mut Value, pointer: &str) {
    if let Some(Value::String(text)) = item.pointer_mut(pointer) {
        if text.contains("$TM_SELECTED_TEXT") {
            *text = text.replace("$TM_SELECTED_TEXT", "");
        }
    }
}

/// Sanitize a single resolved completion item (completionItem/resolve response).
pub fn sanitize_resolved_completion(msg: &mut Value) {
    let Some(result) = msg.get_mut("result") else {
        return;
    };
    sanitize_completion_item(result);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn processes_array_completion_results() {
        let mut response = json!({
            "result": [{
                "kind": 2,
                "labelDetails": { "detail": "(String, List<String>)" },
                "sortText": "method"
            }]
        });

        process_completions(&mut response);

        assert_eq!(response["result"][0]["sortText"], json!("02method"));
    }

    #[test]
    fn applies_list_item_default_snippet_format() {
        let mut response = json!({
            "result": {
                "itemDefaults": { "insertTextFormat": 2 },
                "items": [{
                    "kind": 2,
                    "insertText": "$TM_SELECTED_TEXT.trim()"
                }]
            }
        });

        process_completions(&mut response);

        assert_eq!(
            response["result"]["items"][0]["insertText"],
            json!(".trim()")
        );
    }

    #[test]
    fn sanitizes_text_edit_text() {
        let mut response = json!({
            "result": [{
                "kind": 15,
                "textEditText": "$TM_SELECTED_TEXT.field"
            }]
        });

        process_completions(&mut response);

        assert_eq!(response["result"][0]["textEditText"], json!(".field"));
    }

    #[test]
    fn sanitizes_insert_replace_edit_text() {
        let mut response = json!({
            "result": [{
                "kind": 15,
                "textEdit": {
                    "newText": "$TM_SELECTED_TEXT.var",
                    "insert": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 0 }
                    },
                    "replace": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 3 }
                    }
                }
            }]
        });

        process_completions(&mut response);

        assert_eq!(response["result"][0]["textEdit"]["newText"], json!(".var"));
    }

    #[test]
    fn leaves_plain_text_completion_unchanged() {
        let mut response = json!({
            "result": [{
                "kind": 1,
                "insertTextFormat": 1,
                "insertText": "$TM_SELECTED_TEXT"
            }]
        });

        process_completions(&mut response);

        assert_eq!(
            response["result"][0]["insertText"],
            json!("$TM_SELECTED_TEXT")
        );
    }

    #[test]
    fn postfix_var_completion_keeps_jdtls_spacing() {
        let insertion = "var name = \"hello world\";";
        let mut response = json!({
            "result": [{
                "label": ".var",
                "kind": 15,
                "insertTextFormat": 2,
                "textEdit": {
                    "newText": insertion,
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 17 }
                    }
                }
            }]
        });

        process_completions(&mut response);

        assert_eq!(
            response["result"][0]["textEdit"]["newText"],
            json!(insertion)
        );
    }

    #[test]
    fn counts_nested_generic_parameters() {
        assert_eq!(count_params("(Map<String, List<Integer>>, int)"), 2);
        assert_eq!(count_params("()"), 0);
        assert_eq!(count_params("not-a-signature"), 0);
    }
}
