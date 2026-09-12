//! Minimal Windows registry access (HKCU only).
//!
//! Red Strap never touches HKLM: install records, URL protocols, and
//! per-application graphics settings all live under HKEY_CURRENT_USER, so
//! no elevation is ever required. On other platforms every function reports
//! [`Error::Unsupported`] so call sites stay clean.

use crate::consts::APP_NAME;
use crate::error::{Error, Result};

/// `Software\Microsoft\Windows\CurrentVersion\Uninstall\RedStrap`
pub fn uninstall_key() -> String {
    format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{APP_NAME}"
    )
}

/// `Software\RedStrap`
pub fn app_key() -> String {
    format!("Software\\{APP_NAME}")
}

/// URL protocol key, e.g. `Software\Classes\roblox-player`.
pub fn protocol_key(protocol: &str) -> String {
    format!("Software\\Classes\\{protocol}")
}

/// Write a REG_SZ value. `value_name = None` targets the `(Default)` value.
pub fn set_sz(subkey: &str, value_name: Option<&str>, data: &str) -> Result<()> {
    imp::set_sz(subkey, value_name, data)
}

/// Write a REG_DWORD value. `value_name = None` targets `(Default)`.
pub fn set_dword(subkey: &str, value_name: Option<&str>, data: u32) -> Result<()> {
    imp::set_dword(subkey, value_name, data)
}

/// Read a REG_SZ value. Returns `Ok(None)` when the key or value is absent.
pub fn get_sz(subkey: &str, value_name: Option<&str>) -> Result<Option<String>> {
    imp::get_sz(subkey, value_name)
}

/// Delete a whole key tree. Absent keys are not an error.
pub fn delete_tree(subkey: &str) -> Result<()> {
    imp::delete_tree(subkey)
}

/// Delete a single value. Absent values are not an error.
pub fn delete_value(subkey: &str, value_name: Option<&str>) -> Result<()> {
    imp::delete_value(subkey, value_name)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE,
        REG_CREATED_NEW_KEY, REG_DWORD, REG_OPENED_EXISTING_KEY, REG_OPTION_NON_VOLATILE,
        REG_SZ,
    };

    use super::*;

    // Win32 "file not found" — used to recognise absent keys/values.
    const ERROR_FILE_NOT_FOUND: u32 = 2;

    fn wide_or_default(name: Option<&str>) -> Vec<u16> {
        match name {
            Some(n) => crate::util::to_wide(n),
            None => vec![0],
        }
    }

    fn create_key(subkey: &str) -> std::result::Result<HKEY, Error> {
        let wide = crate::util::to_wide(subkey);
        let mut handle: HKEY = 0;
        let mut disposition = REG_CREATED_NEW_KEY;
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut handle,
                &mut disposition,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(Error::Registry(format!(
                "could not create '{subkey}' (error {status})"
            )));
        }
        let _ = disposition;
        let _ = REG_OPENED_EXISTING_KEY;
        Ok(handle)
    }

    fn open_key(subkey: &str) -> std::result::Result<Option<HKEY>, Error> {
        let wide = crate::util::to_wide(subkey);
        let mut handle: HKEY = 0;
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide.as_ptr(),
                0,
                KEY_READ,
                &mut handle,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(Error::Registry(format!(
                "could not open '{subkey}' (error {status})"
            )));
        }
        Ok(Some(handle))
    }

    fn close(handle: HKEY) {
        unsafe {
            RegCloseKey(handle);
        }
    }

    pub fn set_sz(subkey: &str, value_name: Option<&str>, data: &str) -> Result<()> {
        let handle = create_key(subkey)?;
        let name = wide_or_default(value_name);
        let bytes = crate::util::to_wide(data);
        let byte_len = (bytes.len() * 2) as u32;
        let status = unsafe {
            RegSetValueExW(
                handle,
                name.as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr() as *const u8,
                byte_len,
            )
        };
        close(handle);
        if status != ERROR_SUCCESS {
            return Err(Error::Registry(format!(
                "could not write '{subkey}' (error {status})"
            )));
        }
        Ok(())
    }

    pub fn set_dword(subkey: &str, value_name: Option<&str>, data: u32) -> Result<()> {
        let handle = create_key(subkey)?;
        let name = wide_or_default(value_name);
        let bytes = data.to_le_bytes();
        let status = unsafe {
            RegSetValueExW(
                handle,
                name.as_ptr(),
                0,
                REG_DWORD,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        };
        close(handle);
        if status != ERROR_SUCCESS {
            return Err(Error::Registry(format!(
                "could not write '{subkey}' (error {status})"
            )));
        }
        Ok(())
    }

    pub fn get_sz(subkey: &str, value_name: Option<&str>) -> Result<Option<String>> {
        let handle = match open_key(subkey)? {
            Some(h) => h,
            None => return Ok(None),
        };
        let name = wide_or_default(value_name);

        // Query the required size first.
        let mut value_type: u32 = 0;
        let mut byte_len: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                handle,
                name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                std::ptr::null_mut(),
                &mut byte_len,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            close(handle);
            return Ok(None);
        }
        if status != ERROR_SUCCESS || value_type != REG_SZ {
            close(handle);
            if status != ERROR_SUCCESS {
                return Err(Error::Registry(format!(
                    "could not read '{subkey}' (error {status})"
                )));
            }
            return Ok(None);
        }

        let mut buffer = vec![0u16; (byte_len as usize / 2).max(1)];
        let mut byte_len = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                handle,
                name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                buffer.as_mut_ptr() as *mut u8,
                &mut byte_len,
            )
        };
        close(handle);
        if status != ERROR_SUCCESS {
            return Err(Error::Registry(format!(
                "could not read '{subkey}' (error {status})"
            )));
        }
        let text = unsafe {
            crate::util::wide_to_string_lossy(buffer.as_ptr(), buffer.len())
        };
        Ok(Some(text))
    }

    pub fn delete_tree(subkey: &str) -> Result<()> {
        let wide = crate::util::to_wide(subkey);
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(Error::Registry(format!(
                "could not delete '{subkey}' (error {status})"
            )))
        }
    }

    pub fn delete_value(subkey: &str, value_name: Option<&str>) -> Result<()> {
        let handle = match open_key(subkey)? {
            Some(h) => h,
            None => return Ok(()),
        };
        // RegDeleteValueW needs write access; reopen via create (idempotent).
        close(handle);
        let handle = create_key(subkey)?;
        let name = wide_or_default(value_name);
        let status = unsafe { RegDeleteValueW(handle, name.as_ptr()) };
        close(handle);
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(Error::Registry(format!(
                "could not delete value in '{subkey}' (error {status})"
            )))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn set_sz(_subkey: &str, _value: Option<&str>, _data: &str) -> Result<()> {
        Err(Error::Unsupported(String::from(
            "the Windows registry is unavailable on this platform",
        )))
    }

    pub fn set_dword(_subkey: &str, _value: Option<&str>, _data: u32) -> Result<()> {
        Err(Error::Unsupported(String::from(
            "the Windows registry is unavailable on this platform",
        )))
    }

    pub fn get_sz(_subkey: &str, _value: Option<&str>) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn delete_tree(_subkey: &str) -> Result<()> {
        Ok(())
    }

    pub fn delete_value(_subkey: &str, _value: Option<&str>) -> Result<()> {
        Ok(())
    }
}
