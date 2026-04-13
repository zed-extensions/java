use serde::Serialize;
use std::io::{self, Read, Write};

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

        let header_str = String::from_utf8_lossy(&header_buf);
        let content_length = header_str
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(": ")?;
                if name.eq_ignore_ascii_case(CONTENT_LENGTH) {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let mut content = vec![0u8; content_length];
        self.reader.read_exact(&mut content)?;

        let mut message = header_buf;
        message.extend_from_slice(&content);
        Ok(Some(message))
    }
}

pub fn parse_lsp_content(raw: &[u8]) -> Option<serde_json::Value> {
    let sep_pos = raw.windows(4).position(|w| w == HEADER_SEP)?;
    serde_json::from_slice(&raw[sep_pos + 4..]).ok()
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
