use std::{
    io::{self, Read},
    net::{Shutdown, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

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
    thread::scope(|scope| {
        scope.spawn(|| {
            if let Err(error) = drain_results(results, &finished) {
                crate::lsp_warn!("Java test result connection ended: {error}");
            }
        });
        let result = relay(&editor, &adapter);
        finished.store(true, Ordering::Relaxed);
        result
    })
}

fn relay(editor: &TcpStream, adapter: &TcpStream) -> io::Result<()> {
    editor.set_nodelay(true)?;
    adapter.set_nodelay(true)?;
    thread::scope(|scope| {
        let request = scope.spawn(|| forward(editor, adapter));
        let response = forward(adapter, editor);
        let request = request.join().unwrap();
        response.and(request)
    })
}

fn forward(mut source: &TcpStream, mut destination: &TcpStream) -> io::Result<()> {
    let result = io::copy(&mut source, &mut destination).map(|_| ());
    let _ = source.shutdown(Shutdown::Both);
    let _ = destination.shutdown(Shutdown::Both);
    result
}

fn drain_results(listener: TcpListener, finished: &AtomicBool) -> io::Result<()> {
    let active = || !finished.load(Ordering::Relaxed);
    let Some(mut stream) = accept(listener, active)? else {
        return Ok(());
    };
    stream.set_read_timeout(Some(POLL_INTERVAL))?;
    let mut buffer = [0; 4096];
    while active() {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
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
            let mut message = [0];
            editor.write_all(b"J").unwrap();
            adapter.read_exact(&mut message).unwrap();
            assert_eq!(&message, b"J");
            adapter.write_all(b"D").unwrap();
            editor.read_exact(&mut message).unwrap();
            assert_eq!(&message, b"D");
            drop(editor);
            handler.join().unwrap().unwrap();
            assert!(TcpStream::connect(address).is_err());
            if let Some(mut runner) = runner {
                runner
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                match runner.read(&mut message) {
                    Ok(0) => {}
                    Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                    result => panic!("Expected the result connection to close, got {result:?}"),
                }
            }
        }
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
