//! Thin wrappers over the Windows APIs the app needs outside the UI.

use crate::Result;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HANDLE,
    SetLastError, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
};

pub const MINIMIZED_ARG: &str = "--minimized";

/// The command Windows runs at sign-in.
pub fn startup_command(executable: &Path, minimized: bool) -> String {
    let command = format!("\"{}\"", executable.display());
    if minimized {
        format!("{command} {MINIMIZED_ARG}")
    } else {
        command
    }
}

pub fn set_user_registry_string(subkey: &str, name: &str, value: Option<&str>) -> Result<()> {
    let subkey = wide(subkey);
    let name = wide(name);
    let status = unsafe {
        match value {
            Some(value) => {
                let data = wide(value);
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    size_of_val(data.as_slice()) as u32,
                )
            }
            None => match RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr()) {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            },
        }
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status as i32).into())
    }
}

pub fn user_registry_dword(subkey: &str, name: &str) -> Option<u32> {
    let subkey = wide(subkey);
    let name = wide(name);
    let mut value = 0u32;
    let mut size = size_of_val(&value) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

pub fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

pub struct SingleInstance(HANDLE);

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn claim_single_instance(name: &str) -> Result<Option<SingleInstance>> {
    let name = wide(name);
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let handle = CreateMutexW(null(), 0, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            Ok(None)
        } else {
            Ok(Some(SingleInstance(handle)))
        }
    }
}

pub struct NamedEvent(HANDLE);

// Event handles can be used from any thread.
unsafe impl Send for NamedEvent {}

impl Drop for NamedEvent {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl NamedEvent {
    pub fn open_or_create(name: &str) -> Result<Self> {
        let name = wide(name);
        let handle = unsafe { CreateEventW(null(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(Self(handle))
        }
    }

    pub fn signal(&self) -> Result<()> {
        if unsafe { SetEvent(self.0) } == 0 {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(())
        }
    }

    pub fn wait(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, timeout_ms) == WAIT_OBJECT_0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegDeleteTreeW};

    struct DeleteTestKey(String);

    impl Drop for DeleteTestKey {
        fn drop(&mut self) {
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(&self.0).as_ptr()) };
        }
    }

    fn user_registry_string(subkey: &str, name: &str) -> Option<String> {
        let subkey = wide(subkey);
        let name = wide(name);
        let mut buffer = [0u16; 512];
        let mut size = size_of_val(&buffer) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS)
            .then(|| String::from_utf16(&buffer[..size as usize / 2 - 1]).unwrap())
    }

    #[test]
    fn startup_command_quotes_executable_and_adds_minimized_on_request() {
        let path = Path::new(r"C:\Program Files\z-overlay\z-overlay.exe");
        assert_eq!(
            startup_command(path, false),
            r#""C:\Program Files\z-overlay\z-overlay.exe""#
        );
        assert_eq!(
            startup_command(path, true),
            r#""C:\Program Files\z-overlay\z-overlay.exe" --minimized"#
        );
    }

    #[test]
    fn user_registry_string_is_set_and_deleted() {
        let key = format!(r"Software\z-overlay-test-{}", std::process::id());
        let _cleanup = DeleteTestKey(key.clone());
        set_user_registry_string(&key, "Value", Some("hello")).unwrap();
        assert_eq!(
            user_registry_string(&key, "Value").as_deref(),
            Some("hello")
        );
        set_user_registry_string(&key, "Value", None).unwrap();
        assert_eq!(user_registry_string(&key, "Value"), None);
        set_user_registry_string(&key, "Value", None).unwrap();
    }

    #[test]
    fn missing_registry_dword_reads_as_none() {
        assert_eq!(
            user_registry_dword(r"Software\z-overlay-missing", "Nope"),
            None
        );
    }

    #[test]
    fn named_event_wakes_another_handle_once() {
        let name = format!(r"Local\z-overlay-test-event-{}", std::process::id());
        let first = NamedEvent::open_or_create(&name).unwrap();
        let second = NamedEvent::open_or_create(&name).unwrap();
        assert!(!second.wait(0));
        first.signal().unwrap();
        assert!(second.wait(0));
        assert!(!second.wait(0));
    }

    #[test]
    fn named_instance_is_exclusive_until_released() {
        let name = format!(r"Local\z-overlay-test-instance-{}", std::process::id());
        let first = claim_single_instance(&name).unwrap();
        assert!(first.is_some());
        assert!(claim_single_instance(&name).unwrap().is_none());
        drop(first);
        assert!(claim_single_instance(&name).unwrap().is_some());
    }
}
