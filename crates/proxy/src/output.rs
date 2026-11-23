use proxy_common::encode_lsp;
use serde::Serialize;
use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct Output {
    inner: Arc<Inner>,
}

struct Inner {
    sender: Mutex<Option<mpsc::Sender<Message>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    done: Mutex<Option<mpsc::Receiver<()>>>,
    failed: Arc<AtomicBool>,
}

enum Message {
    Frame(Vec<u8>),
    Shutdown,
}

impl Output {
    pub fn start() -> Self {
        Self::start_with_writer(io::stdout())
    }

    fn start_with_writer(writer: impl Write + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (done_sender, done) = mpsc::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let writer_failed = Arc::clone(&failed);
        let worker = thread::spawn(move || {
            write_loop(writer, receiver, writer_failed);
            let _ = done_sender.send(());
        });
        Self {
            inner: Arc::new(Inner {
                sender: Mutex::new(Some(sender)),
                worker: Mutex::new(Some(worker)),
                done: Mutex::new(Some(done)),
                failed,
            }),
        }
    }

    pub fn send_raw(&self, raw: Vec<u8>) -> bool {
        let sent = self
            .inner
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|sender| sender.send(Message::Frame(raw)).is_ok());
        if !sent {
            self.inner.failed.store(true, Ordering::Relaxed);
        }
        sent
    }

    pub fn send_value(&self, value: &impl Serialize) -> bool {
        self.send_raw(encode_lsp(value).into_bytes())
    }

    pub fn failed(&self) -> bool {
        self.inner.failed.load(Ordering::Relaxed)
    }

    /// Stops accepting frames and gives stdout a bounded interval to drain.
    /// A blocked editor must not prevent the proxy process from terminating.
    pub fn shutdown(&self) {
        if let Some(sender) = self.inner.sender.lock().unwrap().take() {
            let _ = sender.send(Message::Shutdown);
        }
        let drained = self
            .inner
            .done
            .lock()
            .unwrap()
            .take()
            .is_none_or(|done| done.recv_timeout(SHUTDOWN_DRAIN_TIMEOUT).is_ok());
        if let Some(worker) = self.inner.worker.lock().unwrap().take() {
            if drained {
                let _ = worker.join();
            }
        }
    }
}

fn write_loop(mut writer: impl Write, receiver: mpsc::Receiver<Message>, failed: Arc<AtomicBool>) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Frame(frame) => {
                if writer.write_all(&frame).is_err() || writer.flush().is_err() {
                    failed.store(true, Ordering::Relaxed);
                    break;
                }
            }
            Message::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BlockingWriter {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = self.started.send(());
            let _ = self.release.recv();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serializes_complete_frames_in_submission_order() {
        let writer = SharedWriter::default();
        let bytes = Arc::clone(&writer.0);
        let output = Output::start_with_writer(writer);

        assert!(output.send_raw(b"first".to_vec()));
        assert!(output.send_raw(b"second".to_vec()));
        output.shutdown();

        assert_eq!(*bytes.lock().unwrap(), b"firstsecond");
        assert!(!output.send_raw(b"third".to_vec()));
    }

    #[test]
    fn reports_writer_failure() {
        let output = Output::start_with_writer(FailingWriter);
        assert!(output.send_raw(b"message".to_vec()));

        for _ in 0..100 {
            if output.failed() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(1));
        }

        assert!(output.failed());
        output.shutdown();
    }

    #[test]
    fn shutdown_does_not_wait_forever_for_blocked_stdout() {
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let output = Output::start_with_writer(BlockingWriter {
            started: started_sender,
            release: release_receiver,
        });
        assert!(output.send_raw(b"blocked".to_vec()));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let started = std::time::Instant::now();
        output.shutdown();

        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = release_sender.send(());
    }
}
