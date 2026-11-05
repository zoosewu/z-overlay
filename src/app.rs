use crate::editor::{self, EditorState};
use crate::host::{ForegroundTracker, Mode, OverlayHost, RenderCache, image_stamp};
use crate::keys::{self, HotkeyRecorder, Recorded, hotkey_label};
use crate::processes::{ProcessChoice, enumerate_processes};
use crate::updater;
use crate::{MainWindow, ManagedEntry, PromptKind, Tray};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, ModelRc, StandardListViewItem, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::null;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, SM_CXSMICON, SW_SHOWNORMAL, SetForegroundWindow,
};
use z_overlay::Result;
use z_overlay::groups::{GroupToggles, HotkeyPresses};
use z_overlay::render::Renderer;
use z_overlay::settings::{
    GROUP_NUMBERS, Language, ManagedApplication, SETTINGS_FILE_NAME, Settings, portable_dir,
};
use z_overlay::system::{
    NamedEvent, set_user_registry_string, startup_command, user_registry_dword, wide,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "z-overlay";
const TICK_INTERVAL: Duration = Duration::from_millis(16);
/// How often the image files in use are checked for changes.
const FILE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const TRAY_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const TRAY_THEME_INTERVAL: Duration = Duration::from_secs(3);
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

pub struct State {
    pub settings: Settings,
    pub base_dir: PathBuf,
    pub renderer: Renderer,
    pub cache: RenderCache,
    pub host: OverlayHost,
    tracker: ForegroundTracker,
    toggles: GroupToggles,
    presses: HotkeyPresses,
    active: Option<usize>,
    /// The group number whose hotkey is being recorded.
    recording: Option<(u8, HotkeyRecorder)>,
    /// The image files in use as last seen, and when they were last looked at.
    image_stamps: Vec<Option<(SystemTime, u64)>>,
    files_checked: Instant,
    running: Vec<ProcessChoice>,
    app_dialog_row: Option<usize>,
    pending_edit: Option<ManagedApplication>,
    pub pending_remove: Option<Removal>,
    pub editor: EditorState,
}

impl State {
    fn settings_path(&self) -> PathBuf {
        self.base_dir.join(SETTINGS_FILE_NAME)
    }
}

/// What a confirmed removal prompt removes.
pub enum Removal {
    App(usize),
    Overlay(usize),
    Group(usize),
}

pub type Shared = Rc<RefCell<State>>;

macro_rules! on {
    ($ui:expr, $state:expr, $register:ident, $handler:ident $(, $arg:ident)*) => {{
        let weak = $ui.as_weak();
        let state = Rc::clone($state);
        $ui.$register(move |$($arg),*| {
            if let Some(ui) = weak.upgrade() {
                $handler(&ui, &state $(, $arg)*);
            }
        });
    }};
}

pub fn run(start_minimized: bool, show_event: NamedEvent) -> Result<()> {
    let ui = MainWindow::new()?;
    let base_dir = portable_dir()?;
    let (settings, problem) = match Settings::load_from(&base_dir.join(SETTINGS_FILE_NAME)) {
        Ok(settings) => (settings, None),
        Err(error) => (Settings::default(), Some(error.to_string())),
    };
    // The folder may have moved since the startup entry was written.
    if settings.start_with_windows {
        let _ = configure_startup(true, settings.start_minimized);
    }

    let state = Rc::new(RefCell::new(State {
        settings,
        base_dir,
        renderer: Renderer::new(),
        cache: RenderCache::default(),
        host: OverlayHost::default(),
        tracker: ForegroundTracker::default(),
        toggles: GroupToggles::default(),
        presses: HotkeyPresses::default(),
        active: None,
        recording: None,
        image_stamps: Vec::new(),
        files_checked: Instant::now(),
        running: Vec::new(),
        app_dialog_row: None,
        pending_edit: None,
        pending_remove: None,
        editor: EditorState::default(),
    }));
    apply_language(state.borrow().settings.language);
    sync_settings(&ui, &state.borrow());
    if let Some(detail) = problem {
        show_prompt(&ui, PromptKind::LoadFailed, &detail);
    }
    connect(&ui, &state);
    editor::connect(&ui, &state);
    updater::start(&ui);

    let tick_timer = Timer::default();
    tick_timer.start(TimerMode::Repeated, TICK_INTERVAL, {
        let ui = ui.as_weak();
        let state = Rc::clone(&state);
        move || tick(&ui, &state)
    });
    let _tray = TrayKeeper::start(&ui);
    listen_for_show(&ui, show_event);

    if !start_minimized {
        ui.show()?;
    }
    slint::run_event_loop_until_quit()?;
    state.borrow_mut().host.reset();
    Ok(())
}

fn connect(ui: &MainWindow, state: &Shared) {
    on!(ui, state, on_add_clicked, add_clicked);
    on!(ui, state, on_edit_clicked, edit_clicked, row);
    on!(ui, state, on_overlays_clicked, overlays_clicked, row);
    on!(ui, state, on_remove_clicked, remove_clicked, row);
    on!(ui, state, on_running_selected, running_selected, index);
    on!(ui, state, on_refresh_running, refresh_running_clicked);
    on!(ui, state, on_app_dialog_saved, app_dialog_saved);
    on!(ui, state, on_prompt_accepted, prompt_accepted, kind);
    on!(ui, state, on_prompt_canceled, prompt_canceled, kind);
    on!(
        ui,
        state,
        on_start_with_windows_toggled,
        start_with_windows_toggled,
        enabled
    );
    on!(
        ui,
        state,
        on_start_minimized_toggled,
        start_minimized_toggled,
        enabled
    );
    on!(ui, state, on_language_selected, language_selected, index);
    on!(ui, state, on_open_settings_folder, open_settings_folder);
    on!(ui, state, on_record_number, record_number, number);
    on!(ui, state, on_clear_number, clear_number, number);
}

/// Runs every frame: follows the foreground window, reads group hotkeys and
/// places the overlays.
fn tick(ui: &slint::Weak<MainWindow>, state: &Shared) {
    let Some(ui) = ui.upgrade() else {
        return;
    };
    if ui.window().is_minimized() {
        let _ = ui.hide();
    }
    // Skip this tick if a callback is still using the state.
    let Ok(mut state) = state.try_borrow_mut() else {
        return;
    };
    let state = &mut *state;
    poll_recording(&ui, state);
    if state.files_checked.elapsed() >= FILE_CHECK_INTERVAL {
        state.files_checked = Instant::now();
        check_image_files(&ui, state);
    }
    // While overlays are edited on screen, the editor runs them.
    if editor::tick(&ui, state) {
        return;
    }

    let foreground = state.tracker.update(&state.settings.apps);
    let active = foreground.map(|foreground| foreground.app);
    if active != state.active {
        state.active = active;
        state.presses = HotkeyPresses::default();
        let name = active
            .and_then(|index| state.settings.apps.get(index))
            .map_or("", |app| app.display_name());
        ui.set_active_name(name.into());
    }
    match active.and_then(|index| state.settings.apps.get(index)) {
        Some(app) => {
            let hotkeys = state.presses.poll(&state.settings.hotkeys, keys::is_down);
            for number in hotkeys.pressed {
                state.toggles.press(app, number);
            }
            state.toggles.set_held(hotkeys.held);
        }
        None => state.toggles.set_held(Default::default()),
    }
    state.host.update(
        foreground,
        &state.settings,
        Mode::Normal(&state.toggles),
        &mut state.cache,
        &state.renderer,
        &state.base_dir,
    );
}

/// Redraws the overlays whose image file was saved again.
fn check_image_files(ui: &MainWindow, state: &mut State) {
    let stamps: Vec<_> = state
        .settings
        .apps
        .iter()
        .flat_map(|app| &app.overlays)
        .map(|overlay| image_stamp(&state.base_dir, overlay))
        .collect();
    if stamps != state.image_stamps {
        state.image_stamps = stamps;
        state.host.reset();
        editor::refresh_previews(ui, state);
    }
}

fn record_number(ui: &MainWindow, state: &Shared, number: i32) {
    let mut state = state.borrow_mut();
    let Ok(number) = u8::try_from(number) else {
        return;
    };
    // A second click stops recording.
    if state
        .recording
        .as_ref()
        .is_some_and(|(recording, _)| *recording == number)
    {
        state.recording = None;
        ui.set_recording_number(-1);
        return;
    }
    state.recording = Some((number, HotkeyRecorder::default()));
    ui.set_recording_number(i32::from(number));
}

fn clear_number(ui: &MainWindow, state: &Shared, number: i32) {
    let mut state = state.borrow_mut();
    state.recording = None;
    ui.set_recording_number(-1);
    set_number_hotkey(ui, &mut state, number, None);
}

fn set_number_hotkey(
    ui: &MainWindow,
    state: &mut State,
    number: i32,
    hotkey: Option<z_overlay::settings::Hotkey>,
) {
    let Some(slot) = usize::try_from(number - 1)
        .ok()
        .and_then(|index| state.settings.hotkeys.get_mut(index))
    else {
        return;
    };
    *slot = hotkey;
    save_settings(ui, state);
    sync_number_hotkeys(ui, state);
    editor::refresh_rows(ui, state);
}

/// Finishes recording once a hotkey is pressed.
fn poll_recording(ui: &MainWindow, state: &mut State) {
    let Some((number, recorder)) = &mut state.recording else {
        return;
    };
    let number = i32::from(*number);
    match recorder.poll() {
        Recorded::Waiting => {}
        Recorded::Canceled => {
            state.recording = None;
            ui.set_recording_number(-1);
        }
        Recorded::Key(hotkey) => {
            state.recording = None;
            ui.set_recording_number(-1);
            set_number_hotkey(ui, state, number, Some(hotkey));
        }
    }
}

pub fn sync_number_hotkeys(ui: &MainWindow, state: &State) {
    let labels: Vec<slint::SharedString> = (1..=GROUP_NUMBERS)
        .map(|number| {
            state
                .settings
                .hotkey(number)
                .as_ref()
                .map(hotkey_label)
                .unwrap_or_default()
                .into()
        })
        .collect();
    ui.set_number_hotkeys(ModelRc::new(VecModel::from(labels)));
}

fn add_clicked(ui: &MainWindow, state: &Shared) {
    open_app_dialog(ui, &mut state.borrow_mut(), None);
}

fn edit_clicked(ui: &MainWindow, state: &Shared, row: i32) {
    if let Ok(row) = usize::try_from(row) {
        open_app_dialog(ui, &mut state.borrow_mut(), Some(row));
    }
}

fn overlays_clicked(ui: &MainWindow, state: &Shared, row: i32) {
    if let Ok(row) = usize::try_from(row) {
        editor::open(ui, &mut state.borrow_mut(), row);
    }
}

fn open_app_dialog(ui: &MainWindow, state: &mut State, row: Option<usize>) {
    let app = match row {
        Some(row) => match state.settings.apps.get(row) {
            Some(app) => Some(app),
            None => return,
        },
        None => None,
    };
    ui.set_app_dialog_name(app.and_then(|app| app.name.as_deref()).unwrap_or("").into());
    ui.set_app_dialog_identity(app.map_or("", |app| app.identity.as_str()).into());
    ui.set_app_dialog_editing(row.is_some());
    state.app_dialog_row = row;
    refresh_running(ui, state);
    ui.set_app_dialog_open(true);
}

fn remove_clicked(ui: &MainWindow, state: &Shared, row: i32) {
    let mut state = state.borrow_mut();
    let Some((row, name)) = usize::try_from(row).ok().and_then(|row| {
        Some((
            row,
            state.settings.apps.get(row)?.display_name().to_string(),
        ))
    }) else {
        return;
    };
    state.pending_remove = Some(Removal::App(row));
    show_prompt(ui, PromptKind::ConfirmRemove, &name);
}

fn refresh_running_clicked(ui: &MainWindow, state: &Shared) {
    refresh_running(ui, &mut state.borrow_mut());
}

fn refresh_running(ui: &MainWindow, state: &mut State) {
    match unsafe { enumerate_processes() } {
        Ok(running) => {
            let items: Vec<StandardListViewItem> = running
                .iter()
                .map(|choice| StandardListViewItem::from(choice.label().as_str()))
                .collect();
            ui.set_running_apps(ModelRc::new(VecModel::from(items)));
            ui.set_running_current(-1);
            state.running = running;
        }
        Err(error) => show_prompt(ui, PromptKind::RefreshFailed, &error.to_string()),
    }
}

fn running_selected(ui: &MainWindow, state: &Shared, index: i32) {
    let Ok(state) = state.try_borrow() else {
        return;
    };
    if let Some(choice) = usize::try_from(index)
        .ok()
        .and_then(|index| state.running.get(index))
    {
        ui.set_app_dialog_identity(choice.identity().into());
    }
}

fn app_dialog_saved(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    let identity = ui
        .get_app_dialog_identity()
        .trim()
        .trim_matches('"')
        .to_string();
    if identity.is_empty() {
        show_prompt(ui, PromptKind::IdentityEmpty, "");
        return;
    }
    let editing = state.app_dialog_row;
    let duplicate = state
        .settings
        .apps
        .iter()
        .enumerate()
        .any(|(index, app)| Some(index) != editing && app.same_identity(&identity));
    if duplicate {
        show_prompt(ui, PromptKind::IdentityDuplicate, "");
        return;
    }

    let name = ui.get_app_dialog_name().trim().to_string();
    let name = (!name.is_empty()).then_some(name);
    match editing.and_then(|row| state.settings.apps.get(row)) {
        Some(existing) => {
            let app = ManagedApplication {
                identity,
                name,
                ..existing.clone()
            };
            show_prompt(ui, PromptKind::ConfirmEdit, app.display_name());
            state.pending_edit = Some(app);
        }
        None => {
            state
                .settings
                .apps
                .push(ManagedApplication::new(identity, name));
            ui.set_app_dialog_open(false);
            settings_changed(ui, &mut state);
            // A new application has nothing to show yet, so go straight to its overlays.
            let row = state.settings.apps.len() - 1;
            ui.set_managed_current(row as i32);
            editor::open(ui, &mut state, row);
        }
    }
}

fn prompt_accepted(ui: &MainWindow, state: &Shared, kind: PromptKind) {
    ui.set_prompt(PromptKind::Closed);
    let mut state = state.borrow_mut();
    let state = &mut *state;
    match kind {
        PromptKind::ConfirmEdit => {
            if let (Some(row), Some(app)) = (state.app_dialog_row, state.pending_edit.take())
                && let Some(slot) = state.settings.apps.get_mut(row)
            {
                *slot = app;
                ui.set_app_dialog_open(false);
                settings_changed(ui, state);
            }
        }
        PromptKind::ConfirmRemove => match state.pending_remove.take() {
            Some(Removal::App(row)) if row < state.settings.apps.len() => {
                state.settings.apps.remove(row);
                settings_changed(ui, state);
            }
            Some(Removal::Overlay(index)) => editor::remove_overlay(ui, state, index),
            Some(Removal::Group(index)) => editor::remove_group(ui, state, index),
            _ => {}
        },
        _ => {}
    }
}

fn prompt_canceled(ui: &MainWindow, state: &Shared, _kind: PromptKind) {
    ui.set_prompt(PromptKind::Closed);
    let mut state = state.borrow_mut();
    state.pending_edit = None;
    state.pending_remove = None;
}

fn start_with_windows_toggled(ui: &MainWindow, state: &Shared, enabled: bool) {
    let mut state = state.borrow_mut();
    if let Err(error) = configure_startup(enabled, state.settings.start_minimized) {
        ui.set_start_with_windows(state.settings.start_with_windows);
        show_prompt(ui, PromptKind::StartupFailed, &error.to_string());
        return;
    }
    state.settings.start_with_windows = enabled;
    save_settings(ui, &state);
}

fn start_minimized_toggled(ui: &MainWindow, state: &Shared, enabled: bool) {
    let mut state = state.borrow_mut();
    if state.settings.start_with_windows
        && let Err(error) = configure_startup(true, enabled)
    {
        ui.set_start_minimized(state.settings.start_minimized);
        show_prompt(ui, PromptKind::StartupFailed, &error.to_string());
        return;
    }
    state.settings.start_minimized = enabled;
    save_settings(ui, &state);
}

fn language_selected(ui: &MainWindow, state: &Shared, index: i32) {
    let language = language_from_index(index);
    apply_language(language);
    let mut state = state.borrow_mut();
    state.settings.language = language;
    save_settings(ui, &state);
}

fn apply_language(language: Language) {
    let translation = language.translation(sys_locale::get_locale().as_deref());
    let _ = slint::select_bundled_translation(translation);
}

fn open_settings_folder(ui: &MainWindow, state: &Shared) {
    let folder = wide(&state.borrow().base_dir.to_string_lossy());
    let verb = wide("open");
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            folder.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if result as usize <= 32 {
        show_prompt(
            ui,
            PromptKind::OpenFailed,
            &std::io::Error::last_os_error().to_string(),
        );
    }
}

/// Saves the settings and rebuilds everything that shows them.
pub fn settings_changed(ui: &MainWindow, state: &mut State) {
    save_settings(ui, state);
    sync_settings(ui, state);
    overlays_changed(state);
}

/// Rebuilds the overlay windows after an overlay or group changed.
pub fn overlays_changed(state: &mut State) {
    state.host.reset();
    state.tracker.forget();
}

pub fn save_settings(ui: &MainWindow, state: &State) {
    if let Err(error) = state.settings.save_to(&state.settings_path()) {
        show_prompt(ui, PromptKind::SaveFailed, &error.to_string());
    }
}

pub fn sync_settings(ui: &MainWindow, state: &State) {
    sync_apps(ui, state);
    ui.set_managed_current(-1);
    ui.set_start_with_windows(state.settings.start_with_windows);
    ui.set_start_minimized(state.settings.start_minimized);
    ui.set_language_index(language_index(state.settings.language));
    ui.set_settings_folder(state.base_dir.display().to_string().into());
    sync_number_hotkeys(ui, state);
}

/// Refreshes the application list, keeping the selection.
pub fn sync_apps(ui: &MainWindow, state: &State) {
    let entries: Vec<ManagedEntry> = state
        .settings
        .apps
        .iter()
        .map(|app| ManagedEntry {
            label: app.display_name().into(),
            overlays: app.overlays.len() as i32,
            groups: app.groups.len() as i32,
        })
        .collect();
    ui.set_managed_apps(ModelRc::new(VecModel::from(entries)));
}

pub fn show_prompt(ui: &MainWindow, kind: PromptKind, detail: &str) {
    ui.set_prompt_detail(detail.into());
    ui.set_prompt(kind);
}

fn show_window(ui: &MainWindow) {
    let _ = ui.show();
    ui.window().set_minimized(false);
    if let Ok(handle) = ui.window().window_handle().window_handle()
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        unsafe { SetForegroundWindow(handle.hwnd.get() as HWND) };
    }
}

pub fn file_dialog(ui: &MainWindow) -> rfd::FileDialog {
    rfd::FileDialog::new().set_parent(&ui.window().window_handle())
}

fn configure_startup(enabled: bool, minimized: bool) -> Result<()> {
    let command = if enabled {
        Some(startup_command(&std::env::current_exe()?, minimized))
    } else {
        None
    };
    set_user_registry_string(RUN_KEY, RUN_VALUE, command.as_deref())
}

fn language_index(language: Language) -> i32 {
    match language {
        Language::System => 0,
        Language::English => 1,
        Language::TraditionalChinese => 2,
    }
}

fn language_from_index(index: i32) -> Language {
    match index {
        1 => Language::English,
        2 => Language::TraditionalChinese,
        _ => Language::System,
    }
}

fn listen_for_show(ui: &MainWindow, show_event: NamedEvent) {
    let ui = ui.as_weak();
    std::thread::spawn(move || {
        while show_event.wait(INFINITE) {
            let ui = ui.clone();
            let shown = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    show_window(&ui);
                }
            });
            if shown.is_err() {
                break;
            }
        }
    });
}

// Slint gives up on a tray icon whose first registration fails, which happens when
// z-overlay starts before Explorer, so the tray is only created once the taskbar exists.
struct TrayKeeper {
    tray: RefCell<Option<Tray>>,
    retry: Timer,
    theme: Timer,
}

impl TrayKeeper {
    fn start(ui: &MainWindow) -> Rc<Self> {
        let keeper = Rc::new(Self {
            tray: RefCell::new(None),
            retry: Timer::default(),
            theme: Timer::default(),
        });
        let weak_keeper = Rc::downgrade(&keeper);
        let ui = ui.as_weak();
        keeper
            .retry
            .start(TimerMode::Repeated, TRAY_RETRY_INTERVAL, move || {
                let Some(keeper) = weak_keeper.upgrade() else {
                    return;
                };
                if taskbar_exists()
                    && let Ok(tray) = create_tray(&ui)
                {
                    *keeper.tray.borrow_mut() = Some(tray);
                    keeper.retry.stop();
                }
            });

        // The user can switch the taskbar between light and dark at any time.
        let weak_keeper = Rc::downgrade(&keeper);
        keeper
            .theme
            .start(TimerMode::Repeated, TRAY_THEME_INTERVAL, move || {
                let Some(keeper) = weak_keeper.upgrade() else {
                    return;
                };
                if let Some(tray) = &*keeper.tray.borrow() {
                    let light = light_taskbar();
                    if tray.get_light_taskbar() != light {
                        tray.set_light_taskbar(light);
                    }
                }
            });
        keeper
    }
}

fn create_tray(ui: &slint::Weak<MainWindow>) -> Result<Tray> {
    let tray = Tray::new()?;
    tray.set_light_taskbar(light_taskbar());
    tray.set_small_icon(unsafe { GetSystemMetrics(SM_CXSMICON) } <= 16);
    let ui = ui.clone();
    tray.on_show_window(move || {
        if let Some(ui) = ui.upgrade() {
            show_window(&ui);
        }
    });
    tray.on_exit(|| {
        let _ = slint::quit_event_loop();
    });
    Ok(tray)
}

fn light_taskbar() -> bool {
    user_registry_dword(PERSONALIZE_KEY, "SystemUsesLightTheme").unwrap_or(0) != 0
}

fn taskbar_exists() -> bool {
    let class = wide("Shell_TrayWnd");
    unsafe { !FindWindowW(class.as_ptr(), null()).is_null() }
}
