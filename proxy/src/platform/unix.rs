use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

pub fn spawn_parent_monitor(alive: Arc<AtomicBool>, child_pid: u32) {
    thread::spawn(move || {
        let ppid = unsafe { libc::getppid() };
        loop {
            thread::sleep(Duration::from_secs(5));
            if !alive.load(Ordering::Relaxed) {
                break;
            }
            if unsafe { libc::kill(ppid, 0) } != 0 {
                alive.store(false, Ordering::Relaxed);
                unsafe { libc::kill(child_pid as i32, libc::SIGTERM) };
                break;
            }
        }
    });
}
