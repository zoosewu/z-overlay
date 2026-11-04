//! Reads the keyboard for group hotkeys without hooking it, so the
//! application in front still receives every key.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW,
};
use z_overlay::groups::{VK_CONTROL, VK_MENU, VK_SHIFT, is_hotkey_key};
use z_overlay::settings::Hotkey;

const VK_ESCAPE: u16 = 0x1B;

pub fn is_down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(key as i32) as u16 & 0x8000 != 0 }
}

pub enum Recorded {
    Waiting,
    Canceled,
    Key(Hotkey),
}

/// Waits for every key to be released, then records the next hotkey pressed.
#[derive(Default)]
pub struct HotkeyRecorder {
    armed: bool,
}

impl HotkeyRecorder {
    pub fn poll(&mut self) -> Recorded {
        let pressed = (1..=0xFE).find(|&key| is_hotkey_key(key) && is_down(key));
        if !self.armed {
            self.armed = pressed.is_none();
            return Recorded::Waiting;
        }
        match pressed {
            None => Recorded::Waiting,
            Some(VK_ESCAPE) => Recorded::Canceled,
            Some(key) => Recorded::Key(Hotkey {
                ctrl: is_down(VK_CONTROL),
                alt: is_down(VK_MENU),
                shift: is_down(VK_SHIFT),
                key,
            }),
        }
    }
}

/// "Ctrl+Alt+F1", with the key named in the keyboard layout's language.
pub fn hotkey_label(hotkey: &Hotkey) -> String {
    let mut parts = Vec::new();
    if hotkey.ctrl {
        parts.push("Ctrl".to_string());
    }
    if hotkey.alt {
        parts.push("Alt".to_string());
    }
    if hotkey.shift {
        parts.push("Shift".to_string());
    }
    parts.push(key_name(hotkey.key));
    parts.join("+")
}

fn key_name(key: u16) -> String {
    match key {
        0x04 => return "Mouse 3".into(),
        0x05 => return "Mouse 4".into(),
        0x06 => return "Mouse 5".into(),
        _ => {}
    }
    let scan = unsafe { MapVirtualKeyW(key as u32, MAPVK_VK_TO_VSC_EX) };
    if scan != 0 {
        // Bits 16-23 hold the scan code; bit 24 marks the extended keys (E0 prefix).
        let extended = if scan & 0xFF00 != 0 { 1 << 24 } else { 0 };
        let lparam = ((scan & 0xFF) << 16 | extended) as i32;
        let mut buffer = [0u16; 64];
        let length = unsafe { GetKeyNameTextW(lparam, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length > 0 {
            return String::from_utf16_lossy(&buffer[..length as usize]);
        }
    }
    format!("0x{key:02X}")
}
