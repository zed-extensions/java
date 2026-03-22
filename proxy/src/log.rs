use serde::Serialize;
use std::io::{self, Write};

use crate::lsp::encode_lsp;

/// LSP `MessageType` constants as defined in the specification.
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#messageType
#[allow(dead_code)]
mod message_type {
    pub const ERROR: u8 = 1;
    pub const WARNING: u8 = 2;
    pub const INFO: u8 = 3;
    pub const LOG: u8 = 4;
    pub const DEBUG: u8 = 5;
}

#[derive(Serialize)]
struct LogMessageNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: LogMessageParams<'a>,
}

#[derive(Serialize)]
struct LogMessageParams<'a> {
    r#type: u8,
    message: &'a str,
}

/// Sends a `window/logMessage` LSP notification to stdout so that Zed
/// displays the message in its Server Logs panel.
///
/// This locks stdout for the duration of the write to ensure the LSP
/// framing is not interleaved with other output.
fn send_log_message(level: u8, message: &str) {
    let notification = LogMessageNotification {
        jsonrpc: "2.0",
        method: "window/logMessage",
        params: LogMessageParams {
            r#type: level,
            message,
        },
    };

    let encoded = encode_lsp(&notification);

    let stdout = io::stdout();
    let mut w = stdout.lock();
    let _ = w.write_all(encoded.as_bytes());
    let _ = w.flush();
}

#[allow(dead_code)]
pub fn error(message: &str) {
    send_log_message(message_type::ERROR, message);
}

#[allow(dead_code)]
pub fn warn(message: &str) {
    send_log_message(message_type::WARNING, message);
}

#[allow(dead_code)]
pub fn info(message: &str) {
    send_log_message(message_type::INFO, message);
}

#[allow(dead_code)]
pub fn log(message: &str) {
    send_log_message(message_type::LOG, message);
}

#[allow(dead_code)]
pub fn debug(message: &str) {
    send_log_message(message_type::DEBUG, message);
}

/// Logs a message at `Error` level (MessageType = 1) to Zed's Server Logs
/// via a `window/logMessage` LSP notification.
///
/// Supports `format!`-style arguments:
/// ```ignore
/// lsp_error!("something failed: {}", err);
/// ```
#[macro_export]
macro_rules! lsp_error {
    ($($arg:tt)*) => {
        $crate::log::error(&format!($($arg)*))
    };
}

/// Logs a message at `Warning` level (MessageType = 2) to Zed's Server Logs
/// via a `window/logMessage` LSP notification.
///
/// Supports `format!`-style arguments:
/// ```ignore
/// lsp_warn!("unexpected value: {}", val);
/// ```
#[macro_export]
macro_rules! lsp_warn {
    ($($arg:tt)*) => {
        $crate::log::warn(&format!($($arg)*))
    };
}

/// Logs a message at `Info` level (MessageType = 3) to Zed's Server Logs
/// via a `window/logMessage` LSP notification.
///
/// Supports `format!`-style arguments:
/// ```ignore
/// lsp_info!("proxy started on port {}", port);
/// ```
#[macro_export]
macro_rules! lsp_info {
    ($($arg:tt)*) => {
        $crate::log::info(&format!($($arg)*))
    };
}

/// Logs a message at `Log` level (MessageType = 4) to Zed's Server Logs
/// via a `window/logMessage` LSP notification.
///
/// Supports `format!`-style arguments:
/// ```ignore
/// lsp_log!("forwarding request id={}", id);
/// ```
#[macro_export]
macro_rules! lsp_log {
    ($($arg:tt)*) => {
        $crate::log::log(&format!($($arg)*))
    };
}

/// Logs a message at `Debug` level (MessageType = 5) to Zed's Server Logs
/// via a `window/logMessage` LSP notification.
///
/// Supports `format!`-style arguments:
/// ```ignore
/// lsp_debug!("raw message bytes: {}", raw.len());
/// ```
#[macro_export]
macro_rules! lsp_debug {
    ($($arg:tt)*) => {
        $crate::log::debug(&format!($($arg)*))
    };
}
