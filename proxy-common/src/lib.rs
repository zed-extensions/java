//! Primitives shared by the Zed Java extension's native binaries
//! (`java-lsp-proxy` and `gradle-lsp-bridge`):
//!
//! - [`lsp`]: LSP message framing — a streaming reader, content parsing, and
//!   encoding helpers.
//! - [`platform`]: a parent-process monitor that terminates the spawned child
//!   when the editor that launched us goes away.
//! - [`uri`]: filesystem-path-to-`file://`-URI conversion.

pub mod lsp;
pub mod platform;
pub mod uri;

#[cfg(feature = "tokio")]
pub use lsp::AsyncLspReader;
pub use lsp::{
    contains_subslice, encode_lsp, lsp_body, parse_content_length, parse_lsp_content, raw_has_id,
    write_raw, write_to_stdout, LspReader, CONTENT_LENGTH, HEADER_SEP,
};
pub use platform::spawn_parent_monitor;
pub use uri::path_to_file_uri;
