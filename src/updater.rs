//! Checks GitHub for a newer release in the background and, when asked,
//! downloads it, swaps it in and restarts. Problems never interrupt the user;
//! they only show where the update is offered.

use crate::{MainWindow, UpdateState};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::time::Duration;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use z_overlay::Result;
use z_overlay::update::{
    Release, Version, latest_release_url, newer, old_executable, parse_release, replace_executable,
    verify_checksum,
};

pub const UPDATED_ARG: &str = "--updated";
/// The first check waits for the app to settle; later ones repeat this often.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;

thread_local! {
    /// The release on offer; only touched on the event loop's thread.
    static OFFER: RefCell<Option<Release>> = const { RefCell::new(None) };
}

pub fn start(ui: &MainWindow) {
    ui.set_version(Version::current().to_string().into());
    let weak = ui.as_weak();
    ui.on_update_clicked(move || {
        if let (Some(ui), Some(release)) =
            (weak.upgrade(), OFFER.with(|offer| offer.borrow().clone()))
        {
            install(&ui, release);
        }
    });

    let weak = ui.as_weak();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK_DELAY);
        loop {
            // A failed check is simply tried again at the next interval.
            if let Ok(Some(release)) = check() {
                let weak = weak.clone();
                let sent = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        offer_release(&ui, release);
                    }
                });
                if sent.is_err() {
                    return;
                }
            }
            std::thread::sleep(CHECK_INTERVAL);
        }
    });
}

fn offer_release(ui: &MainWindow, release: Release) {
    // Leave a download in progress alone.
    if ui.get_update_state() == UpdateState::Downloading {
        return;
    }
    ui.set_update_version(release.version.to_string().into());
    ui.set_update_state(UpdateState::Available);
    OFFER.with(|offer| *offer.borrow_mut() = Some(release));
}

fn agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(timeout))
        .user_agent(format!("z-overlay/{}", Version::current()))
        .build()
        .new_agent()
}

/// The newest release, if it is newer than this build.
fn check() -> Result<Option<Release>> {
    let mut response = match agent(Duration::from_secs(30))
        .get(latest_release_url())
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        // No release has been published yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let json = response.body_mut().read_to_string()?;
    Ok(newer(parse_release(&json)?, Version::current()))
}

fn install(ui: &MainWindow, release: Release) {
    ui.set_update_state(UpdateState::Downloading);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = download_and_replace(&release);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            match result.and_then(|()| restart()) {
                Ok(()) => {
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    ui.set_update_error(error.to_string().into());
                    ui.set_update_state(UpdateState::Failed);
                }
            }
        });
    });
}

fn download_and_replace(release: &Release) -> Result<()> {
    let agent = agent(Duration::from_secs(300));
    let data = agent
        .get(&release.executable_url)
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()?;
    let checksum = agent
        .get(&release.checksum_url)
        .call()?
        .body_mut()
        .read_to_string()?;
    verify_checksum(&data, &checksum)?;
    replace_executable(&std::env::current_exe()?, &data)
}

/// Starts the new executable, which waits for this one to exit.
fn restart() -> Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg(UPDATED_ARG)
        .spawn()?;
    Ok(())
}

/// Removes the executable an update left behind, once its process has exited.
pub fn clean_up_after_update() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let old = old_executable(&executable);
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..40 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}
