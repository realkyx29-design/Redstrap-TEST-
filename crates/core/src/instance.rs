//! Single-instance guards ("only one settings window at a time").
//!
//! Windows uses a named mutex; Unix uses an `flock`'d file in the temp
//! directory. The guard releases automatically when dropped.

use crate::error::{Error, Result};

/// RAII guard proving this process owns the named instance slot.
pub struct InstanceLock {
    _guard: imp::Guard,
}

impl InstanceLock {
    /// Try to acquire the slot `name`. Returns `Ok(None)` when another
    /// process already holds it.
    pub fn try_acquire(name: &str) -> Result<Option<Self>> {
        imp::try_acquire(&sanitize(name)).map(|g| g.map(|_guard| Self { _guard }))
    }
}

fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                Some(c)
            } else {
                None
            }
        })
        .collect();
    if out.is_empty() {
        out.push_str("app");
    }
    if out.len() > 64 {
        out.truncate(64);
    }
    out
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    use super::*;

    pub struct Guard {
        handle: HANDLE,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    // The handle is only ever closed on drop from the owning thread.
    unsafe impl Send for Guard {}

    pub fn try_acquire(name: &str) -> Result<Option<Guard>> {
        let wide = crate::util::to_wide(&format!("Local\\RedStrap-{name}"));
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        if handle == 0 {
            let code = unsafe { GetLastError() };
            return Err(Error::Win32(code));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        Ok(Some(Guard { handle }))
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    use super::*;

    pub struct Guard {
        _file: std::fs::File,
    }

    pub fn try_acquire(name: &str) -> Result<Option<Guard>> {
        let path = std::env::temp_dir().join(format!("{}.{}.lock", crate::consts::APP_ID, name));
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| Error::with_path(&path, e))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(Some(Guard { _file: file }))
        } else {
            let errno = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(-1);
            if errno == libc::EWOULDBLOCK {
                Ok(None)
            } else {
                Err(Error::Process(format!(
                    "could not lock {}: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                )))
            }
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::*;

    pub struct Guard;

    pub fn try_acquire(_name: &str) -> Result<Option<Guard>> {
        // No OS primitive available; allow the instance (documented).
        Ok(Some(Guard))
    }
}
