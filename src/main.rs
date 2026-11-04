#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]

mod app;
mod editor;
mod host;
mod keys;
mod processes;

use std::ptr::null_mut;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, MB_ICONERROR, MB_OK, MessageBoxW,
};
use z_overlay::Result;
use z_overlay::system::{MINIMIZED_ARG, NamedEvent, claim_single_instance, wide};

slint::include_modules!();

const INSTANCE_MUTEX: &str = r"Local\z-overlay.SingleInstance";
const SHOW_EVENT: &str = r"Local\z-overlay.Show";

fn main() {
    if let Err(error) = run() {
        show_error(&error.to_string());
    }
}

fn run() -> Result<()> {
    let start_minimized = std::env::args_os().any(|arg| arg == MINIMIZED_ARG);
    let show_event = NamedEvent::open_or_create(SHOW_EVENT)?;
    let Some(_instance) = claim_single_instance(INSTANCE_MUTEX)? else {
        if !start_minimized {
            // Whoever launched this copy may bring windows forward; pass that on
            // to the running copy so its window comes to the front.
            unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            show_event.signal()?;
        }
        return Ok(());
    };
    app::run(start_minimized, show_event)
}

fn show_error(text: &str) {
    let text = wide(text);
    let title = wide("z-overlay");
    unsafe {
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}
