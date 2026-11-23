use serde::Serialize;
use std::io::{self, Read, Write};

#[cfg(feature = "tokio")]
use tokio::io::{AsyncRead, AsyncReadExt};

pub const CONTENT_LENGTH: &str = "Content-Length";
pub const HEADER_SEP: &[u8] = b"\r\n\r\n";

pub struct LspReader<R> {
    reader: R,
}

impl<R: Read> LspReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn read_message(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut header_buf = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match self.reader.read(&mut byte) {
                Ok(0) => return Ok(None),
                Ok(_) => header_buf.push(byte[0]),
                Err(e) => return Err(e),
            }
            if header_buf.ends_with(HEADER_SEP) {
                break;
            }
        }

        let content_length = parse_content_length(&header_buf);
        let mut content = vec![0u8; content_length];
        self.reader.read_exact(&mut content)?;

        let mut message = header_buf;
        message.extend_from_slice(&content);
        Ok(Some(message))
    }
}

#[cfg(feature = "tokio")]
pub struct AsyncLspReader<R> {
    reader: R,
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead + Unpin> AsyncLspReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub async fn read_message(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut header_buf = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match self.reader.read(&mut byte).await {
                Ok(0) => return Ok(None),
                Ok(_) => header_buf.push(byte[0]),
                Err(e) => return Err(e),
            }
            if header_buf.ends_with(HEADER_SEP) {
                break;
            }
        }

        let content_length = parse_content_length(&header_buf);
        let mut content = vec![0u8; content_length];
        self.reader.read_exact(&mut content).await?;

        let mut message = header_buf;
        message.extend_from_slice(&content);
        Ok(Some(message))
    }
}

/// Parse the `Content-Length` value from a complete LSP header block (the bytes
/// up to and including the `\r\n\r\n` separator). Returns `0` when absent.
///
/// Shared by the synchronous [`LspReader`] and the bridge's async reader, which
/// differ only in how they read bytes off the wire, not in how they frame them.
pub fn parse_content_length(header: &[u8]) -> usize {
    String::from_utf8_lossy(header)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            if name.eq_ignore_ascii_case(CONTENT_LENGTH) {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

/// The JSON body of a raw LSP message — everything after the `\r\n\r\n` header
/// separator — or `None` if the framing is absent.
pub fn lsp_body(raw: &[u8]) -> Option<&[u8]> {
    let sep_pos = raw.windows(4).position(|w| w == HEADER_SEP)?;
    Some(&raw[sep_pos + 4..])
}

/// Whether `needle` occurs anywhere in `haystack`.
pub fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

pub fn parse_lsp_content(raw: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(lsp_body(raw)?).ok()
}

/// Cheap check for the presence of an `"id"` key in the JSON body of a raw LSP
/// message. Used to skip full JSON parsing for notifications, which carry no
/// `id` and therefore cannot be responses or completion results.
pub fn raw_has_id(raw: &[u8]) -> bool {
    lsp_body(raw).is_some_and(|body| contains_subslice(body, b"\"id\":"))
}

pub fn encode_lsp(value: &impl Serialize) -> String {
    let json = serde_json::to_string(value).unwrap();
    format!("{CONTENT_LENGTH}: {}\r\n\r\n{json}", json.len())
}

/// Write raw LSP bytes to a writer, flushing afterward.
pub fn write_raw(w: &mut impl Write, raw: &[u8]) {
    let _ = w.write_all(raw);
    let _ = w.flush();
}

/// Encode a value as an LSP message and write it to stdout.
pub fn write_to_stdout(value: &impl Serialize) {
    let out = encode_lsp(value);
    let mut w = io::stdout().lock();
    let _ = w.write_all(out.as_bytes());
    let _ = w.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn reads_a_framed_message() {
        let body = r#"{"jsonrpc":"2.0","id":1}"#;
        let mut reader = LspReader::new(std::io::Cursor::new(frame(body)));
        let msg = reader.read_message().unwrap().expect("a message");
        assert_eq!(lsp_body(&msg), Some(body.as_bytes()));
        assert!(reader.read_message().unwrap().is_none()); // EOF
    }

    #[test]
    fn parses_content_length_case_insensitively() {
        assert_eq!(parse_content_length(b"Content-Length: 42\r\n\r\n"), 42);
        assert_eq!(parse_content_length(b"content-length: 7\r\n\r\n"), 7);
        // Missing / malformed header -> 0.
        assert_eq!(parse_content_length(b"X-Other: 1\r\n\r\n"), 0);
    }

    #[test]
    fn lsp_body_requires_framing() {
        assert_eq!(lsp_body(b"no separator here"), None);
        assert_eq!(lsp_body(b"H: 1\r\n\r\nbody"), Some(&b"body"[..]));
    }

    #[test]
    fn contains_subslice_basics() {
        assert!(contains_subslice(b"hello world", b"o w"));
        assert!(!contains_subslice(b"abc", b"abcd")); // needle longer than haystack
        assert!(!contains_subslice(b"abc", b"")); // empty needle
    }

    #[test]
    fn raw_has_id_detects_id_field() {
        assert!(raw_has_id(&frame(r#"{"id":1}"#)));
        assert!(!raw_has_id(&frame(r#"{"method":"x"}"#)));
    }
}
