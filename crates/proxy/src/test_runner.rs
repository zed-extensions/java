use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const RESULT_GRACE: Duration = Duration::from_millis(500);

struct EditorOutput {
    stream: TcpStream,
    next_seq: u64,
    reverse_requests: HashMap<u64, u64>,
}

impl EditorOutput {
    fn send(&mut self, mut message: serde_json::Value) -> io::Result<()> {
        let sequence = self.next_seq;
        self.next_seq += 1;
        if message["type"] == "request" {
            if let Some(original) = message["seq"].as_u64() {
                self.reverse_requests.insert(sequence, original);
            }
        }
        message["seq"] = sequence.into();
        self.stream
            .write_all(proxy_common::encode_lsp(&message).as_bytes())
    }
}

fn read_dap(
    reader: &mut proxy_common::LspReader<&TcpStream>,
) -> io::Result<Option<serde_json::Value>> {
    reader
        .read_message()?
        .map(|raw| {
            proxy_common::parse_lsp_content(&raw)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid DAP message"))
        })
        .transpose()
}

pub fn start(adapter_port: u16) -> io::Result<(u16, u16)> {
    let dap = bind_listener()?;
    let results = bind_listener()?;
    let ports = (dap.local_addr()?.port(), results.local_addr()?.port());
    thread::Builder::new()
        .name("java-test-session".into())
        .spawn(move || {
            if let Err(error) = serve(dap, results, adapter_port, CONNECT_TIMEOUT) {
                crate::lsp_warn!("Java test session ended: {error}");
            }
        })?;
    Ok(ports)
}

fn bind_listener() -> io::Result<TcpListener> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn accept(listener: TcpListener, keep_waiting: impl Fn() -> bool) -> io::Result<Option<TcpStream>> {
    while keep_waiting() {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn serve(
    dap: TcpListener,
    results: TcpListener,
    adapter_port: u16,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let Some(editor) = accept(dap, || Instant::now() < deadline)? else {
        return Ok(());
    };
    let adapter = TcpStream::connect_timeout(&([127, 0, 0, 1], adapter_port).into(), timeout)?;
    let finished = AtomicBool::new(false);
    let results_done = AtomicBool::new(false);
    let output = Arc::new(Mutex::new(EditorOutput {
        stream: editor.try_clone()?,
        next_seq: 1,
        reverse_requests: HashMap::new(),
    }));
    thread::scope(|scope| {
        scope.spawn(|| {
            if let Err(error) = drain_results(results, &finished, &output) {
                crate::lsp_warn!("Java test result connection ended: {error}");
            }
            results_done.store(true, Ordering::Release);
        });
        let result = relay(&editor, &adapter, &output, &results_done);
        finished.store(true, Ordering::Relaxed);
        result
    })
}

fn relay(
    editor: &TcpStream,
    adapter: &TcpStream,
    output: &Mutex<EditorOutput>,
    results_done: &AtomicBool,
) -> io::Result<()> {
    editor.set_nodelay(true)?;
    adapter.set_nodelay(true)?;
    thread::scope(|scope| {
        let request = scope.spawn(|| {
            let result = forward_requests(editor, adapter, output);
            let _ = adapter.shutdown(Shutdown::Both);
            result
        });
        let response = forward_responses(adapter, output, results_done);
        let _ = editor.shutdown(Shutdown::Both);
        let _ = adapter.shutdown(Shutdown::Both);
        let request = request.join().unwrap();
        response.and(request)
    })
}

fn forward_requests(
    editor: &TcpStream,
    mut adapter: &TcpStream,
    output: &Mutex<EditorOutput>,
) -> io::Result<()> {
    let mut reader = proxy_common::LspReader::new(editor);
    while let Some(mut message) = read_dap(&mut reader)? {
        if message["type"] == "response" {
            if let Some(seq) = message["request_seq"].as_u64() {
                if let Some(original) = output.lock().unwrap().reverse_requests.remove(&seq) {
                    message["request_seq"] = original.into();
                }
            }
        }
        adapter.write_all(proxy_common::encode_lsp(&message).as_bytes())?;
    }
    Ok(())
}

fn forward_responses(
    adapter: &TcpStream,
    output: &Mutex<EditorOutput>,
    results_done: &AtomicBool,
) -> io::Result<()> {
    let mut reader = proxy_common::LspReader::new(adapter);
    while let Some(message) = read_dap(&mut reader)? {
        if message["type"] == "event" && message["event"] == "terminated" {
            let deadline = Instant::now() + RESULT_GRACE;
            while !results_done.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(POLL_INTERVAL);
            }
        }
        output.lock().unwrap().send(message)?;
    }
    Ok(())
}

fn drain_results(
    listener: TcpListener,
    finished: &AtomicBool,
    output: &Mutex<EditorOutput>,
) -> io::Result<()> {
    let active = || !finished.load(Ordering::Relaxed);
    let Some(mut stream) = accept(listener, active)? else {
        return Ok(());
    };
    stream.set_read_timeout(Some(POLL_INTERVAL))?;
    let mut buffer = [0; 4096];
    let mut decoder = ResultDecoder::default();
    while active() {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                for text in decoder.push(&buffer[..count]) {
                    output.lock().unwrap().send(serde_json::json!({
                        "type": "event", "event": "output",
                        "body": { "category": "stderr", "output": text }
                    }))?;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[derive(Default)]
struct ResultDecoder {
    pending: Vec<u8>,
    recording: bool,
}

impl ResultDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        loop {
            if self.pending.starts_with(b"@@<TestRunner-") {
                let marker = b"-TestRunner>";
                let Some(end) = self
                    .pending
                    .windows(marker.len())
                    .position(|window| window == marker)
                else {
                    break;
                };
                let frame: Vec<_> = self.pending.drain(..end + marker.len()).collect();
                let json = &frame[b"@@<TestRunner-".len()..end];
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) {
                    if matches!(value["name"].as_str(), Some("testFailed" | "error")) {
                        let attributes = &value["attributes"];
                        let name = attributes["name"]
                            .as_str()
                            .or_else(|| attributes["message"].as_str())
                            .unwrap_or("TestNG");
                        let trace = attributes["trace"].as_str().unwrap_or("");
                        output.push(format!("Test failed: {name}\n{trace}\n"));
                    }
                }
            } else if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(['\r', '\n']);
                if let Some(failure) = line
                    .strip_prefix("%FAILED")
                    .or_else(|| line.strip_prefix("%ERROR"))
                {
                    output.push(format!("Test failed: {}\n", failure.trim_start()));
                } else if line.starts_with("%TRACES")
                    || line.starts_with("%EXPECTS")
                    || line.starts_with("%ACTUALS")
                {
                    self.recording = true;
                    if line.starts_with("%EXPECTS") {
                        output.push("Expected:\n".into());
                    }
                    if line.starts_with("%ACTUALS") {
                        output.push("Actual:\n".into());
                    }
                } else if line.starts_with("%TRACEE")
                    || line.starts_with("%EXPECTE")
                    || line.starts_with("%ACTUALE")
                {
                    self.recording = false;
                } else if self.recording {
                    output.push(format!("{line}\n"));
                }
            } else {
                break;
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn editor_disconnect_releases_results_with_or_without_a_connected_runner() {
        for connect_runner in [false, true] {
            let adapter = TcpListener::bind("127.0.0.1:0").unwrap();
            let adapter_port = adapter.local_addr().unwrap().port();
            let dap = bind_listener().unwrap();
            let results = bind_listener().unwrap();
            let address = results.local_addr().unwrap();
            let mut editor = TcpStream::connect(dap.local_addr().unwrap()).unwrap();
            let runner = connect_runner.then(|| TcpStream::connect(address).unwrap());
            let handler = thread::spawn(move || serve(dap, results, adapter_port, CONNECT_TIMEOUT));
            let (mut adapter, _) = adapter.accept().unwrap();
            editor
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            adapter
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let request = serde_json::json!({"seq": 1, "type": "request", "command": "initialize"});
            editor
                .write_all(proxy_common::encode_lsp(&request).as_bytes())
                .unwrap();
            let received = read_dap(&mut proxy_common::LspReader::new(&adapter))
                .unwrap()
                .unwrap();
            assert_eq!(received, request);
            let response = serde_json::json!({"seq": 2, "type": "response", "request_seq": 1, "command": "initialize", "success": true});
            adapter
                .write_all(proxy_common::encode_lsp(&response).as_bytes())
                .unwrap();
            let received = read_dap(&mut proxy_common::LspReader::new(&editor))
                .unwrap()
                .unwrap();
            assert_eq!(received["request_seq"], 1);
            drop(editor);
            handler.join().unwrap().unwrap();
            assert!(TcpStream::connect(address).is_err());
            if let Some(mut runner) = runner {
                runner
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut message = [0];
                match runner.read(&mut message) {
                    Ok(0) => {}
                    Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                    result => panic!("Expected the result connection to close, got {result:?}"),
                }
            }
        }
    }

    #[test]
    fn runner_protocol_handles_split_utf8_and_testng_frames() {
        let mut decoder = ResultDecoder::default();
        let input = "%FAILED 1,test\n%TRACES \n断言失败\n%TRACEE \n";
        let output: String = input
            .as_bytes()
            .iter()
            .flat_map(|byte| decoder.push(&[*byte]))
            .collect();
        assert!(output.contains("断言失败"));
        let input = r#"@@<TestRunner-{"name":"testFailed","attributes":{"name":"example.Test#fails","trace":"failure\nstack"}}-TestRunner>"#;
        let output: String = input
            .as_bytes()
            .iter()
            .flat_map(|byte| decoder.push(&[*byte]))
            .collect();
        assert!(output.contains("example.Test#fails"));
        assert!(output.contains("failure\nstack"));
    }

    #[test]
    fn failures_are_sent_to_editor_before_terminated() {
        let adapter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dap = bind_listener().unwrap();
        let results = bind_listener().unwrap();
        let mut editor = TcpStream::connect(dap.local_addr().unwrap()).unwrap();
        editor
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut runner = TcpStream::connect(results.local_addr().unwrap()).unwrap();
        let adapter_port = adapter_listener.local_addr().unwrap().port();
        let handler = thread::spawn(move || serve(dap, results, adapter_port, CONNECT_TIMEOUT));
        let (mut adapter, _) = adapter_listener.accept().unwrap();
        runner
            .write_all("%FAILED 1,example\n%TRACES \nexpected 2 but was 1\n%TRACEE \n".as_bytes())
            .unwrap();
        drop(runner);
        adapter
            .write_all(
                proxy_common::encode_lsp(&serde_json::json!({
                    "seq": 1, "type": "event", "event": "terminated"
                }))
                .as_bytes(),
            )
            .unwrap();
        let mut reader = proxy_common::LspReader::new(&editor);
        let mut text = String::new();
        loop {
            let message = read_dap(&mut reader).unwrap().unwrap();
            if message["event"] == "terminated" {
                break;
            }
            if let Some(output) = message["body"]["output"].as_str() {
                text.push_str(output);
            }
        }
        assert!(text.contains("expected 2 but was 1"));
        editor
            .write_all(
                proxy_common::encode_lsp(&serde_json::json!({
                    "seq": 1, "type": "request", "command": "disconnect"
                }))
                .as_bytes(),
            )
            .unwrap();
        drop(editor);
        drop(adapter);
        handler.join().unwrap().unwrap();
    }

    #[test]
    fn reverse_request_responses_keep_the_adapters_original_sequence() {
        let adapter_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dap = bind_listener().unwrap();
        let results = bind_listener().unwrap();
        let mut editor = TcpStream::connect(dap.local_addr().unwrap()).unwrap();
        let adapter_port = adapter_listener.local_addr().unwrap().port();
        let handler = thread::spawn(move || serve(dap, results, adapter_port, CONNECT_TIMEOUT));
        let (mut adapter, _) = adapter_listener.accept().unwrap();
        editor
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        adapter
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        adapter
            .write_all(
                proxy_common::encode_lsp(&serde_json::json!({
                    "seq": 73, "type": "request", "command": "runInTerminal", "arguments": {}
                }))
                .as_bytes(),
            )
            .unwrap();
        let request = read_dap(&mut proxy_common::LspReader::new(&editor))
            .unwrap()
            .unwrap();
        let proxy_seq = request["seq"].as_u64().unwrap();
        assert_ne!(proxy_seq, 73);
        editor
            .write_all(
                proxy_common::encode_lsp(&serde_json::json!({
                    "seq": 1, "type": "response", "request_seq": proxy_seq,
                    "command": "runInTerminal", "success": true
                }))
                .as_bytes(),
            )
            .unwrap();
        let response = read_dap(&mut proxy_common::LspReader::new(&adapter))
            .unwrap()
            .unwrap();
        assert_eq!(response["request_seq"], 73);
        drop(editor);
        drop(adapter);
        handler.join().unwrap().unwrap();
    }

    #[test]
    fn both_ports_close_when_editor_never_connects() {
        let dap = bind_listener().unwrap();
        let results = bind_listener().unwrap();
        let dap_address = dap.local_addr().unwrap();
        let result_address = results.local_addr().unwrap();
        serve(dap, results, 1, POLL_INTERVAL).unwrap();
        assert!(TcpStream::connect(dap_address).is_err());
        assert!(TcpStream::connect(result_address).is_err());
    }
}
