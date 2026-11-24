use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    },
    System::Threading::{
        GetCurrentProcessId, OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    },
};

struct ScopedSnapshot(HANDLE);

impl Drop for ScopedSnapshot {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn parent_pid() -> u32 {
    unsafe {
        let pid = GetCurrentProcessId();
        let snap_handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

        if snap_handle == INVALID_HANDLE_VALUE {
            return 0;
        }

        let _snap = ScopedSnapshot(snap_handle);

        let mut entry: PROCESSENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snap_handle, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == pid {
                    return entry.th32ParentProcessID;
                }
                if Process32Next(snap_handle, &mut entry) == 0 {
                    break;
                }
            }
        }

        0
    }
}

pub fn spawn_parent_monitor(alive: Arc<AtomicBool>, child_pid: u32) {
    let ppid = parent_pid();

    thread::spawn(move || {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, ppid) };
        if handle.is_null() {
            return;
        }

        loop {
            thread::sleep(Duration::from_secs(5));
            if !alive.load(Ordering::Relaxed) {
                break;
            }
            if unsafe { WaitForSingleObject(handle, 0) } == 0 {
                alive.store(false, Ordering::Relaxed);
                let _ = Command::new("taskkill")
                    .args(["/pid", &child_pid.to_string(), "/T", "/F"])
                    .spawn();
                break;
            }
        }
        unsafe { CloseHandle(handle) };
    });
}
