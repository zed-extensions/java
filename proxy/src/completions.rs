use serde_json::Value;

pub fn should_sort_completions(msg: &Value) -> bool {
    msg.get("result").is_some_and(|result| {
        result.get("items").is_some_and(|v| v.is_array()) || result.is_array()
    })
}

pub fn sort_completions_by_param_count(msg: &mut Value) {
    let items = if let Some(result) = msg.get_mut("result") {
        if result.is_array() {
            result.as_array_mut()
        } else {
            result.get_mut("items").and_then(|v| v.as_array_mut())
        }
    } else {
        None
    };

    if let Some(items) = items {
        for item in items.iter_mut() {
            let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
            if kind == 2 || kind == 3 {
                let detail = item
                    .pointer("/labelDetails/detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let count = count_params(detail);
                let existing = item.get("sortText").and_then(|v| v.as_str()).unwrap_or("");
                item["sortText"] = Value::String(format!("{count:02}{existing}"));
            }
        }
    }
}

fn count_params(detail: &str) -> usize {
    if detail.is_empty() || detail == "()" {
        return 0;
    }
    let inner = detail
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(detail)
        .trim();
    if inner.is_empty() {
        return 0;
    }
    let mut count = 1usize;
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}
