//! Cross-platform process helpers: discovery, termination, priority.
//!
//! Windows uses ToolHelp snapshots + `OpenProcess`; Unix scans `/proc` and
//! uses `kill(2)` / `setpriority(2)`. Everything degrades to descriptive
//! errors (never panics) when permissions are insufficient.

use crate::error::{Error, Result};
use crate::settings::ProcessPriority;

// ---------------------------------------------------------------------------
// Public API (shared)
// ---------------------------------------------------------------------------

/// PIDs of processes whose executable name contains `needle`
/// (case-insensitive substring match).
pub fn find_pids_by_name(needle: &str) -> Vec<u32> {
    let needle = needle.to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    imp::find_pids_by_name(&needle)
}

/// True when at least one process matches `needle`.
pub fn is_running_by_name(needle: &str) -> bool {
    !find_pids_by_name(needle).is_empty()
}

/// True when `pid` currently exists.
pub fn process_exists(pid: u32) -> bool {
    imp::process_exists(pid)
}

/// Terminate `pid`. `force` selects SIGKILL/`TerminateProcess`; otherwise a
/// graceful SIGTERM is attempted first on Unix.
pub fn kill_pid(pid: u32, force: bool) -> Result<()> {
    if pid == 0 {
        return Err(Error::Process(String::from("refusing to kill PID 0")));
    }
    imp::kill_pid(pid, force)
}

/// Apply an OS scheduling priority to a running process.
pub fn set_priority(pid: u32, priority: ProcessPriority) -> Result<()> {
    if pid == 0 {
        return Err(Error::Process(String::from("invalid PID 0")));
    }
    imp::set_priority(pid, priority)
}

/// Kill every process matching `needle`, returning the kill count.
/// Processes that vanish mid-scan are skipped, not errors.
pub fn kill_all_by_name(needle: &str) -> usize {
    let mut killed = 0;
    for pid in find_pids_by_name(needle) {
        if pid == std::process::id() {
            continue;
        }
        if kill_pid(pid, true).is_ok() {
            killed += 1;
        }
    }
    killed
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, BOOL, HANDLE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, SetPriorityClass, TerminateProcess, ABOVE_NORMAL_PRIORITY_CLASS,
        BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS,
        NORMAL_PRIORITY_CLASS, PROCESS_SET_INFORMATION, PROCESS_TERMINATE,
        REALTIME_PRIORITY_CLASS,
    };

    use super::*;

    // Eternal Win32 values, defined locally to avoid extra imports.
    const TRUE: BOOL = 1;
    #[allow(dead_code)]
    const FALSE: BOOL = 0;
    const INVALID_HANDLE_VALUE: HANDLE = -1;

    fn last_win32_error(what: &str) -> Error {
        let code = unsafe { GetLastError() };
        Error::Process(format!("{what} (Win32 error {code})"))
    }

    pub fn find_pids_by_name(needle: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return out;
        }

        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if unsafe { Process32FirstW(snapshot, &mut entry) } != TRUE {
            unsafe {
                CloseHandle(snapshot);
            }
            return out;
        }

        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if name.to_ascii_lowercase().contains(needle) {
                out.push(entry.th32ProcessID);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } != TRUE {
                break;
            }
        }

        unsafe {
            CloseHandle(snapshot);
        }
        out
    }

    pub fn process_exists(pid: u32) -> bool {
        // A snapshot scan needs no special access rights, unlike OpenProcess
        // with query rights on protected processes.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        if unsafe { Process32FirstW(snapshot, &mut entry) } == TRUE {
            loop {
                if entry.th32ProcessID == pid {
                    found = true;
                    break;
                }
                if unsafe { Process32NextW(snapshot, &mut entry) } != TRUE {
                    break;
                }
            }
        }
        unsafe {
            CloseHandle(snapshot);
        }
        found
    }

    fn open(pid: u32, access: u32) -> std::result::Result<HANDLE, Error> {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle == 0 {
            return Err(last_win32_error(&format!("could not open PID {pid}")));
        }
        Ok(handle)
    }

    pub fn kill_pid(pid: u32, _force: bool) -> Result<()> {
        let handle = open(pid, PROCESS_TERMINATE)?;
        let ok = unsafe { TerminateProcess(handle, 1) };
        unsafe {
            CloseHandle(handle);
        }
        if ok == TRUE {
            Ok(())
        } else {
            Err(last_win32_error(&format!("could not terminate PID {pid}")))
        }
    }

    pub fn set_priority(pid: u32, priority: ProcessPriority) -> Result<()> {
        let class = match priority {
            ProcessPriority::Low => IDLE_PRIORITY_CLASS,
            ProcessPriority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            ProcessPriority::Normal => NORMAL_PRIORITY_CLASS,
            ProcessPriority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            ProcessPriority::High => HIGH_PRIORITY_CLASS,
            ProcessPriority::RealTime => REALTIME_PRIORITY_CLASS,
        };
        let handle = open(pid, PROCESS_SET_INFORMATION)?;
        let ok = unsafe { SetPriorityClass(handle, class) };
        unsafe {
            CloseHandle(handle);
        }
        if ok == TRUE {
            Ok(())
        } else {
            Err(last_win32_error(&format!(
                "could not set priority of PID {pid} (try running as administrator)"
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Unix implementation
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::path::Path;

    use super::*;

    pub fn find_pids_by_name(needle: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir("/proc") {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let pid: u32 = match name.to_str().and_then(|s| s.parse().ok()) {
                Some(pid) => pid,
                None => continue,
            };
            let base = Path::new("/proc").join(name);
            // `comm` holds the bare executable name; `cmdline` covers
            // interpreters and renamed processes.
            let comm = std::fs::read_to_string(base.join("comm"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let cmdline = std::fs::read(base.join("cmdline"))
                .map(|b| String::from_utf8_lossy(&b).to_ascii_lowercase())
                .unwrap_or_default();
            if comm.contains(needle) || cmdline.contains(needle) {
                out.push(pid);
            }
        }
        out
    }

    pub fn process_exists(pid: u32) -> bool {
        // Signal 0 performs no action; success (or EPERM) means "alive".
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    pub fn kill_pid(pid: u32, force: bool) -> Result<()> {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if result == 0 {
            Ok(())
        } else {
            Err(Error::Process(format!(
                "could not signal PID {pid}: {}",
                std::io::Error::last_os_error()
            )))
        }
    }

    pub fn set_priority(pid: u32, priority: ProcessPriority) -> Result<()> {
        // nice values: negative needs privileges, positive is always fine.
        let nice = match priority {
            ProcessPriority::Low => 10,
            ProcessPriority::BelowNormal => 5,
            ProcessPriority::Normal => 0,
            ProcessPriority::AboveNormal => -5,
            ProcessPriority::High => -10,
            ProcessPriority::RealTime => -19,
        };
        let result =
            unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) };
        if result == 0 {
            Ok(())
        } else {
            Err(Error::Process(format!(
                "could not renice PID {pid}: {}",
                std::io::Error::last_os_error()
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Fallback for exotic platforms
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::*;

    pub fn find_pids_by_name(_needle: &str) -> Vec<u32> {
        Vec::new()
    }

    pub fn process_exists(_pid: u32) -> bool {
        false
    }

    pub fn kill_pid(_pid: u32, _force: bool) -> Result<()> {
        Err(Error::Unsupported(String::from(
            "process control is unavailable on this platform",
        )))
    }

    pub fn set_priority(_pid: u32, _priority: ProcessPriority) -> Result<()> {
        Err(Error::Unsupported(String::from(
            "process priority is unavailable on this platform",
        )))
    }
}
