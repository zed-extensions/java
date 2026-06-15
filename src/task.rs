use serde_json::Value;
use zed_extension_api::{self as zed, LanguageServerId, Worktree};

use crate::download;

const TASK_HELPER_BINARY: &str = "java-task-helper";

pub fn task_helper_binary_path(
    cached: &mut Option<String>,
    configuration: &Option<Value>,
    language_server_id: &LanguageServerId,
    worktree: &Worktree,
) -> zed::Result<String> {
    download::download_binary(
        cached,
        configuration,
        language_server_id,
        worktree,
        TASK_HELPER_BINARY,
    )
}
