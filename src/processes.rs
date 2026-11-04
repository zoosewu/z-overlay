use std::ffi::OsString;
use std::mem::zeroed;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use z_overlay::Result;

#[derive(Clone)]
pub struct ProcessChoice {
    title: String,
    name: String,
    path: Option<PathBuf>,
}

impl ProcessChoice {
    pub fn identity(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.name.clone())
    }

    pub fn label(&self) -> String {
        format!("{} — {}", self.title, self.identity())
    }
}

pub struct ProcessIdentity {
    pub name: String,
    pub path: Option<PathBuf>,
}

pub unsafe fn enumerate_processes() -> Result<Vec<ProcessChoice>> {
    let mut processes = Vec::<ProcessChoice>::new();
    if EnumWindows(Some(enum_window), &mut processes as *mut _ as isize) == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    processes.sort_by_key(|choice| choice.identity().to_lowercase());
    processes.dedup_by(|a, b| a.identity().eq_ignore_ascii_case(&b.identity()));
    processes.sort_by_key(|choice| choice.label().to_lowercase());
    Ok(processes)
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || pid == GetCurrentProcessId() {
        return 1;
    }
    let Some(identity) = process_identity(pid) else {
        return 1;
    };
    let title = match window_text(hwnd) {
        title if !title.is_empty() => title,
        _ => identity.name.clone(),
    };
    let processes = &mut *(lparam as *mut Vec<ProcessChoice>);
    processes.push(ProcessChoice {
        title,
        name: identity.name,
        path: identity.path,
    });
    1
}

pub unsafe fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let path = process_path(pid);
    let name = path
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| process_name(pid))?;
    Some(ProcessIdentity { name, path })
}

unsafe fn process_path(pid: u32) -> Option<PathBuf> {
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if process.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
    CloseHandle(process);
    (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..length as usize])))
}

unsafe fn process_name(pid: u32) -> Option<String> {
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: PROCESSENTRY32W = zeroed();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    if Process32FirstW(snapshot, &mut entry) != 0 {
        loop {
            if entry.th32ProcessID == pid {
                found = Some(wide_array_to_string(&entry.szExeFile));
                break;
            }
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
    }
    CloseHandle(snapshot);
    found
}

unsafe fn window_text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd);
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

fn wide_array_to_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}
