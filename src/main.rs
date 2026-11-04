#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]

mod app;
mod editor;
mod host;
mod keys;
mod processes;
mod updater;

use std::ptr::null_mut;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, MB_ICONERROR, MB_OK, MessageBoxW,
};
use z_overlay::Result;
use z_overlay::system::{MINIMIZED_ARG, NamedEvent, SingleInstance, claim_single_instance, wide};

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
    let updated = std::env::args_os().any(|arg| arg == updater::UPDATED_ARG);
    let Some(_instance) = claim_instance(updated)? else {
        if !start_minimized {
            // Whoever launched this copy may bring windows forward; pass that on
            // to the running copy so its window comes to the front.
            unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            show_event.signal()?;
        }
        return Ok(());
    };
    if updated {
        updater::clean_up_after_update();
    }
    app::run(start_minimized, show_event)
}

/// After an update the previous version is still closing, so wait for it.
fn claim_instance(updated: bool) -> Result<Option<SingleInstance>> {
    let attempts = if updated { 100 } else { 1 };
    for attempt in 1..=attempts {
        if let Some(instance) = claim_single_instance(INSTANCE_MUTEX)? {
            return Ok(Some(instance));
        }
        if attempt < attempts {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    Ok(None)
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
