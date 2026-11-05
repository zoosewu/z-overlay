//! The overlay editor: one managed application's overlays and groups as a
//! hierarchy, a preview, editing on screen over the application, and a
//! details window opened by right-clicking an overlay.

use crate::app::{
    Removal, Shared, State, file_dialog, overlays_changed, save_settings, show_prompt, sync_apps,
};
use crate::host::{EditEvent, Mode, find_app_window, foreground_of};
use crate::keys::{self, hotkey_label};
use crate::{Editor, Inspector, InspectorWindow, MainWindow, OverlayRow, PromptKind, TreeRow};
use resvg::tiny_skia::Pixmap;
use slint::{
    ComponentHandle, Image, Model, ModelRc, PhysicalPosition, Rgba8Pixel, SharedPixelBuffer,
    SharedString, VecModel,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};
use z_overlay::hierarchy::{Row, drop_place, rows};
use z_overlay::placement::{Rect, offset_for, place};
use z_overlay::settings::{
    Align, Anchor, Color, Content, GROUP_NUMBERS, Group, GroupMode, ManagedApplication, Overlay,
    TextStyle, import_image,
};

const PREVIEW_SIZES: [[u32; 2]; 5] = [
    [1280, 720],
    [1920, 1080],
    [2560, 1440],
    [3440, 1440],
    [3840, 2160],
];
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "svg", "svgz"];
/// Size shown on the canvas for an overlay that could not be drawn.
const FAILED_SIZE: (i32, i32) = (48, 48);
const VK_ESCAPE: u16 = 0x1B;

#[derive(Default)]
pub struct EditorState {
    /// The managed application being edited.
    app: Option<usize>,
    /// The hierarchy as shown, to tell what a drop lands on.
    rows: Vec<Row>,
    on_screen: Option<OnScreen>,
    inspector: Option<InspectorWindow>,
    /// The overlay the details window shows.
    inspected: Option<usize>,
}

/// Editing on screen, over this window of the application.
struct OnScreen {
    window: HWND,
    banner: Rc<Pixmap>,
    escape_down: bool,
}

macro_rules! editor_on {
    ($ui:expr, $state:expr, $register:ident, $handler:ident $(, $arg:ident)*) => {{
        let weak = $ui.as_weak();
        let state = Rc::clone($state);
        $ui.global::<Editor>().$register(move |$($arg),*| {
            if let Some(ui) = weak.upgrade() {
                $handler(&ui, &state $(, $arg)*);
            }
        });
    }};
}

pub fn connect(ui: &MainWindow, state: &Shared) {
    editor_on!(ui, state, on_close, close);
    editor_on!(ui, state, on_preview_selected, preview_selected, index);
    editor_on!(ui, state, on_toggle_on_screen, toggle_on_screen);
    editor_on!(ui, state, on_select, select, index);
    editor_on!(ui, state, on_inspect, inspect_clicked, index);
    editor_on!(
        ui,
        state,
        on_enabled_toggled,
        enabled_toggled,
        index,
        enabled
    );
    editor_on!(ui, state, on_add_text, add_text, name);
    editor_on!(ui, state, on_add_image, add_image);
    editor_on!(ui, state, on_add_group, add_group, name);
    editor_on!(ui, state, on_duplicate, duplicate, index);
    editor_on!(ui, state, on_remove, remove, index);
    editor_on!(ui, state, on_dragged, dragged, index, x, y);
    editor_on!(ui, state, on_row_dropped, row_dropped, index, target);
    editor_on!(ui, state, on_group_enabled, group_enabled, index, enabled);
    editor_on!(ui, state, on_group_renamed, group_renamed, index, name);
    editor_on!(ui, state, on_group_number, group_number, index, number);
    editor_on!(ui, state, on_group_mode_toggled, group_mode_toggled, index);
    editor_on!(ui, state, on_remove_group, remove_group_clicked, index);
}

pub fn open(ui: &MainWindow, state: &mut State, app: usize) {
    if app >= state.settings.apps.len() {
        return;
    }
    close_inspector(state);
    state.editor.app = Some(app);
    state.editor.on_screen = None;
    let editor = ui.global::<Editor>();
    editor.set_selected(-1);
    editor.set_on_screen(false);
    sync_all(ui, state);
    ui.set_overlay_editor_open(true);
}

fn close(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    stop_on_screen(ui, &mut state);
    close_inspector(&mut state);
    state.editor.app = None;
    ui.set_overlay_editor_open(false);
    sync_apps(ui, &state);
}

/// The application being edited, if the editor is open.
fn app_index(state: &State) -> Option<usize> {
    state
        .editor
        .app
        .filter(|index| *index < state.settings.apps.len())
}

fn app(state: &State) -> Option<&ManagedApplication> {
    state.settings.apps.get(app_index(state)?)
}

fn app_mut(state: &mut State) -> Option<&mut ManagedApplication> {
    let index = app_index(state)?;
    state.settings.apps.get_mut(index)
}

fn preview_area(app: &ManagedApplication) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: app.preview_size[0] as i32,
        height: app.preview_size[1] as i32,
    }
}

fn sync_all(ui: &MainWindow, state: &mut State) {
    let Some(app) = app(state) else {
        return;
    };
    let editor = ui.global::<Editor>();
    editor.set_title(app.display_name().into());
    editor.set_preview_width(app.preview_size[0] as i32);
    editor.set_preview_height(app.preview_size[1] as i32);
    editor.set_preview_index(
        PREVIEW_SIZES
            .iter()
            .position(|size| *size == app.preview_size)
            .unwrap_or(1) as i32,
    );
    refresh_rows(ui, state);
    refresh_previews(ui, state);
}

/// Rebuilds the hierarchy.
pub fn refresh_rows(ui: &MainWindow, state: &mut State) {
    let Some(app) = app(state) else {
        return;
    };
    let rows = rows(app);
    let items: Vec<TreeRow> = rows.iter().map(|row| tree_row(state, app, *row)).collect();
    state.editor.rows = rows;
    ui.global::<Editor>()
        .set_rows(ModelRc::new(VecModel::from(items)));
}

fn tree_row(state: &State, app: &ManagedApplication, row: Row) -> TreeRow {
    match row {
        Row::NoGroup => TreeRow {
            kind: 0,
            ..TreeRow::default()
        },
        Row::Group(index) => {
            let group = &app.groups[index];
            TreeRow {
                kind: 1,
                index: index as i32,
                name: group.name.as_str().into(),
                enabled: group.enabled,
                number: i32::from(group.number),
                hotkey: state
                    .settings
                    .hotkey(group.number)
                    .as_ref()
                    .map(hotkey_label)
                    .unwrap_or_default()
                    .into(),
                hold: group.mode == GroupMode::Hold,
                ..TreeRow::default()
            }
        }
        Row::Overlay(index) => {
            let overlay = &app.overlays[index];
            TreeRow {
                kind: 2,
                index: index as i32,
                nested: overlay.group.is_some_and(|id| app.group(id).is_some()),
                name: overlay.name.as_str().into(),
                enabled: overlay.enabled,
                ..TreeRow::default()
            }
        }
    }
}

/// Updates one hierarchy row in place, so a field being typed in keeps its focus.
fn update_row(ui: &MainWindow, state: &State, row: Row) {
    let Some(app) = app(state) else {
        return;
    };
    let Some(position) = state.editor.rows.iter().position(|shown| *shown == row) else {
        return;
    };
    let rows = ui.global::<Editor>().get_rows();
    if position < rows.row_count() {
        rows.set_row_data(position, tree_row(state, app, row));
    }
}

/// Redraws the preview of every overlay, and the details window.
pub fn refresh_previews(ui: &MainWindow, state: &mut State) {
    let count = app(state).map_or(0, |app| app.overlays.len());
    let rows: Vec<OverlayRow> = (0..count)
        .filter_map(|index| preview_row(state, index))
        .collect();
    ui.global::<Editor>()
        .set_overlays(ModelRc::new(VecModel::from(rows)));
    refresh_inspector_feedback(state);
}

fn refresh_preview(ui: &MainWindow, state: &mut State, index: usize) {
    let overlays = ui.global::<Editor>().get_overlays();
    if index < overlays.row_count()
        && let Some(row) = preview_row(state, index)
    {
        overlays.set_row_data(index, row);
    }
}

fn preview_row(state: &mut State, index: usize) -> Option<OverlayRow> {
    let app_index = app_index(state)?;
    let app = &state.settings.apps[app_index];
    let overlay = app.overlays.get(index)?;
    let rendered = state.cache.get(&state.renderer, &state.base_dir, overlay);
    let (preview, size) = match &rendered {
        Ok(pixmap) => (
            to_image(pixmap),
            (pixmap.width() as i32, pixmap.height() as i32),
        ),
        Err(_) => (Image::default(), FAILED_SIZE),
    };
    let (x, y) = place(
        preview_area(app),
        overlay.align,
        overlay.pivot(),
        overlay.offset,
        size,
    );
    Some(OverlayRow {
        name: overlay.name.as_str().into(),
        enabled: overlay.enabled,
        preview,
        x: x as f32,
        y: y as f32,
        width: size.0 as f32,
        height: size.1 as f32,
        opacity: overlay.opacity,
        failed: rendered.is_err(),
    })
}

fn to_image(pixmap: &Pixmap) -> Image {
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        pixmap.data(),
        pixmap.width(),
        pixmap.height(),
    );
    Image::from_rgba8_premultiplied(buffer)
}

/// Saves after an overlay changed and redraws it everywhere.
fn overlay_changed(ui: &MainWindow, state: &mut State, index: usize) {
    save_settings(ui, state);
    overlays_changed(state);
    refresh_preview(ui, state, index);
    update_row(ui, state, Row::Overlay(index));
    if state.editor.inspected == Some(index) {
        refresh_inspector_feedback(state);
    }
}

/// Saves after overlays were added, removed or moved, and rebuilds the views.
/// Indexes shift, so the details window closes.
fn structure_changed(ui: &MainWindow, state: &mut State, selected: Option<usize>) {
    save_settings(ui, state);
    overlays_changed(state);
    close_inspector(state);
    ui.global::<Editor>()
        .set_selected(selected.map_or(-1, |index| index as i32));
    refresh_rows(ui, state);
    refresh_previews(ui, state);
}

fn preview_selected(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Some(size) = usize::try_from(index)
        .ok()
        .and_then(|i| PREVIEW_SIZES.get(i))
    else {
        return;
    };
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    app.preview_size = *size;
    save_settings(ui, &state);
    sync_all(ui, &mut state);
}

fn select(ui: &MainWindow, _state: &Shared, index: i32) {
    ui.global::<Editor>().set_selected(index);
}

fn enabled_toggled(ui: &MainWindow, state: &Shared, index: i32, enabled: bool) {
    let mut state = state.borrow_mut();
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let Some(overlay) = app_mut(&mut state).and_then(|app| app.overlays.get_mut(index)) else {
        return;
    };
    overlay.enabled = enabled;
    overlay_changed(ui, &mut state, index);
}

fn add_text(ui: &MainWindow, state: &Shared, name: SharedString) {
    let mut state = state.borrow_mut();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let name = unique_name(&name, app);
    app.overlays.push(Overlay::text(name.clone(), name));
    let added = app.overlays.len() - 1;
    structure_changed(ui, &mut state, Some(added));
}

fn add_image(ui: &MainWindow, state: &Shared) {
    let Some(path) = pick_image(ui, state) else {
        return;
    };
    let mut state = state.borrow_mut();
    let stored = match import_image(&state.base_dir, &path) {
        Ok(stored) => stored,
        Err(error) => {
            show_prompt(ui, PromptKind::ImageFailed, &error.to_string());
            return;
        }
    };
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let name = unique_name(&stem, app);
    app.overlays.push(Overlay::image(name, stored));
    let added = app.overlays.len() - 1;
    structure_changed(ui, &mut state, Some(added));
}

fn pick_image(ui: &MainWindow, state: &Shared) -> Option<PathBuf> {
    // The file dialog runs a nested message loop, so the state must not be borrowed here.
    let directory = state
        .borrow()
        .base_dir
        .join(z_overlay::settings::IMAGES_DIR_NAME);
    let mut dialog = file_dialog(ui).add_filter("Images", &IMAGE_EXTENSIONS);
    if directory.is_dir() {
        dialog = dialog.set_directory(directory);
    }
    dialog.pick_file()
}

fn duplicate(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let Some(original) = app.overlays.get(index) else {
        return;
    };
    let name = unique_name(&original.name, app);
    let Some(copy) = app.duplicate_overlay(index, name) else {
        return;
    };
    structure_changed(ui, &mut state, Some(copy));
}

fn remove(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Some((index, name)) = usize::try_from(index)
        .ok()
        .and_then(|index| Some((index, app(&state)?.overlays.get(index)?.name.clone())))
    else {
        return;
    };
    state.pending_remove = Some(Removal::Overlay(index));
    show_prompt(ui, PromptKind::ConfirmRemove, &name);
}

pub fn remove_overlay(ui: &MainWindow, state: &mut State, index: usize) {
    let Some(app) = app_mut(state) else {
        return;
    };
    if index >= app.overlays.len() {
        return;
    }
    app.overlays.remove(index);
    structure_changed(ui, state, None);
}

/// An overlay was dropped in the preview with its top-left corner at (x, y).
fn dragged(ui: &MainWindow, state: &Shared, index: i32, x: f32, y: f32) {
    let mut state = state.borrow_mut();
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let Some(size) = ui
        .global::<Editor>()
        .get_overlays()
        .row_data(index)
        .map(|row| (row.width as i32, row.height as i32))
    else {
        return;
    };
    let Some(area) = app(&state).map(preview_area) else {
        return;
    };
    let position = (x.round() as i32, y.round() as i32);
    move_overlay_to(ui, &mut state, index, area, position, size);
}

/// Sets the offset that puts an overlay of `size` at `position` within `area`.
fn move_overlay_to(
    ui: &MainWindow,
    state: &mut State,
    index: usize,
    area: Rect,
    position: (i32, i32),
    size: (i32, i32),
) {
    let Some(overlay) = app_mut(state).and_then(|app| app.overlays.get_mut(index)) else {
        return;
    };
    overlay.offset = offset_for(area, overlay.align, overlay.pivot(), position, size);
    let offset = overlay.offset;
    // Only the position changed: the windows follow the new offset by themselves.
    save_settings(ui, state);
    refresh_preview(ui, state, index);
    if state.editor.inspected == Some(index)
        && let Some(inspector) = &state.editor.inspector
    {
        let fields = inspector.global::<Inspector>();
        fields.set_offset_x(offset[0]);
        fields.set_offset_y(offset[1]);
    }
}

fn row_dropped(ui: &MainWindow, state: &Shared, index: i32, target: i32) {
    let mut state = state.borrow_mut();
    let (Ok(index), Ok(target)) = (usize::try_from(index), usize::try_from(target)) else {
        return;
    };
    let rows = state.editor.rows.clone();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    if index >= app.overlays.len() {
        return;
    }
    let (before, group) = drop_place(app, &rows, target);
    let moved = app.move_overlay(index, before, group);
    structure_changed(ui, &mut state, Some(moved));
}

fn add_group(ui: &MainWindow, state: &Shared, name: SharedString) {
    let mut state = state.borrow_mut();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let name = unique_group_name(&name, app);
    app.add_group(name);
    save_settings(ui, &state);
    refresh_rows(ui, &mut state);
}

fn group_mut(state: &mut State, index: i32) -> Option<&mut Group> {
    app_mut(state)?.groups.get_mut(usize::try_from(index).ok()?)
}

/// Saves after a group setting changed and shows it.
fn group_changed(ui: &MainWindow, state: &mut State, index: i32) {
    save_settings(ui, state);
    overlays_changed(state);
    if let Ok(index) = usize::try_from(index) {
        update_row(ui, state, Row::Group(index));
    }
}

fn group_enabled(ui: &MainWindow, state: &Shared, index: i32, enabled: bool) {
    let mut state = state.borrow_mut();
    let Some(group) = group_mut(&mut state, index) else {
        return;
    };
    group.enabled = enabled;
    group_changed(ui, &mut state, index);
}

fn group_renamed(ui: &MainWindow, state: &Shared, index: i32, name: SharedString) {
    let mut state = state.borrow_mut();
    let Some(group) = group_mut(&mut state, index) else {
        return;
    };
    group.name = name.into();
    // Only saved: rebuilding the row would take the focus from the field being typed in.
    save_settings(ui, &state);
}

fn group_number(ui: &MainWindow, state: &Shared, index: i32, number: i32) {
    let mut state = state.borrow_mut();
    let Some(number) = u8::try_from(number)
        .ok()
        .filter(|number| (1..=GROUP_NUMBERS).contains(number))
    else {
        return;
    };
    let Some(group) = group_mut(&mut state, index) else {
        return;
    };
    group.number = number;
    group_changed(ui, &mut state, index);
}

fn group_mode_toggled(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Some(group) = group_mut(&mut state, index) else {
        return;
    };
    group.mode = match group.mode {
        GroupMode::Toggle => GroupMode::Hold,
        GroupMode::Hold => GroupMode::Toggle,
    };
    group_changed(ui, &mut state, index);
}

fn remove_group_clicked(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Some((index, name)) = usize::try_from(index)
        .ok()
        .and_then(|index| Some((index, app(&state)?.groups.get(index)?.name.clone())))
    else {
        return;
    };
    state.pending_remove = Some(Removal::Group(index));
    show_prompt(ui, PromptKind::ConfirmRemove, &name);
}

pub fn remove_group(ui: &MainWindow, state: &mut State, index: usize) {
    let Some(app) = app_mut(state) else {
        return;
    };
    app.remove_group(index);
    let selected = usize::try_from(ui.global::<Editor>().get_selected()).ok();
    structure_changed(ui, state, selected);
}

fn toggle_on_screen(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    if state.editor.on_screen.is_some() {
        stop_on_screen(ui, &mut state);
        return;
    }
    let Some(app) = app(&state) else {
        return;
    };
    let Some(window) = find_app_window(app) else {
        let name = app.display_name().to_string();
        show_prompt(ui, PromptKind::AppNotRunning, &name);
        return;
    };
    let hint = ui.global::<Editor>().get_on_screen_hint().to_string();
    let banner = Content::Text(TextStyle {
        size: 18.0,
        ..TextStyle::new(hint)
    });
    let Ok(banner) = state.renderer.render(&banner, &state.base_dir, 1.0) else {
        return;
    };
    state.editor.on_screen = Some(OnScreen {
        window,
        banner: Rc::new(banner),
        escape_down: keys::is_down(VK_ESCAPE),
    });
    ui.global::<Editor>().set_on_screen(true);
    // Bring the application forward, so the overlays are seen where they will be.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
    }
}

fn stop_on_screen(ui: &MainWindow, state: &mut State) {
    if state.editor.on_screen.take().is_some() {
        state.host.reset();
        ui.global::<Editor>().set_on_screen(false);
    }
}

/// Runs the overlays while they are edited on screen; returns whether they are.
pub fn tick(ui: &MainWindow, state: &mut State) -> bool {
    let Some(on_screen) = &mut state.editor.on_screen else {
        return false;
    };
    let escape_down = keys::is_down(VK_ESCAPE);
    let escape_pressed = escape_down && !on_screen.escape_down;
    on_screen.escape_down = escape_down;
    let window = on_screen.window;
    let banner = Rc::clone(&on_screen.banner);
    let Some(app) = app_index(state) else {
        stop_on_screen(ui, state);
        return false;
    };
    let foreground = foreground_of(window, app);
    // Esc finishes, unless it was meant for one of z-overlay's own windows;
    // so does the application closing.
    let closed = foreground.is_none() && unsafe { IsIconic(window) } == 0;
    if (escape_pressed && !own_window_in_front()) || closed {
        stop_on_screen(ui, state);
        return false;
    }

    for event in state.host.take_events() {
        match event {
            EditEvent::Pressed(index) => ui.global::<Editor>().set_selected(index as i32),
            EditEvent::Dropped(index, at) => {
                let Some(foreground) = foreground else {
                    continue;
                };
                let Some(size) = state.host.size_of(index) else {
                    continue;
                };
                let Some(anchor) = app_overlay(state, index).map(|overlay| overlay.anchor) else {
                    continue;
                };
                move_overlay_to(ui, state, index, foreground.area(anchor), at, size);
            }
            EditEvent::Menu(index, at) => {
                ui.global::<Editor>().set_selected(index as i32);
                inspect(ui, state, index, at);
            }
        }
    }

    state.host.update(
        foreground,
        &state.settings,
        Mode::Editing { banner: &banner },
        &mut state.cache,
        &state.renderer,
        &state.base_dir,
    );
    true
}

fn app_overlay(state: &State, index: usize) -> Option<&Overlay> {
    app(state)?.overlays.get(index)
}

fn own_window_in_front() -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid == std::process::id()
}

fn cursor() -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) };
    (point.x, point.y)
}

fn inspect_clicked(ui: &MainWindow, state: &Shared, index: i32) {
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    // The details window needs the state to talk back, so it is made here.
    if state.borrow().editor.inspector.is_none()
        && let Ok(inspector) = InspectorWindow::new()
    {
        connect_inspector(ui, &inspector, Rc::downgrade(state));
        state.borrow_mut().editor.inspector = Some(inspector);
    }
    inspect(ui, &mut state.borrow_mut(), index, cursor());
}

/// Shows the details of an overlay in a window next to `at`.
fn inspect(_ui: &MainWindow, state: &mut State, index: usize, at: (i32, i32)) {
    let Some(overlay) = app_overlay(state, index).cloned() else {
        return;
    };
    let Some(inspector) = &state.editor.inspector else {
        return;
    };
    let fields = inspector.global::<Inspector>();
    fields.set_name(overlay.name.as_str().into());
    match &overlay.content {
        Content::Text(style) => {
            fields.set_is_text(true);
            fields.set_text(style.text.as_str().into());
            fields.set_font(style.font.as_str().into());
            fields.set_font_size(style.size.round() as i32);
            fields.set_color(style.color.to_string().into());
            fields.set_background(
                style
                    .background
                    .map(|color| color.to_string())
                    .unwrap_or_default()
                    .into(),
            );
            fields.set_padding(style.padding.round() as i32);
        }
        Content::Image { path } => {
            fields.set_is_text(false);
            fields.set_image_path(path.as_str().into());
        }
    }
    fields.set_anchor(match overlay.anchor {
        Anchor::Window => 0,
        Anchor::Monitor => 1,
    });
    fields.set_align(align_index(overlay.align));
    fields.set_pivot(align_index(overlay.pivot()));
    fields.set_offset_x(overlay.offset[0]);
    fields.set_offset_y(overlay.offset[1]);
    fields.set_scale((overlay.scale * 100.0).round() as i32);
    fields.set_opacity((overlay.opacity * 100.0).round() as i32);
    inspector
        .window()
        .set_position(PhysicalPosition::new(at.0 + 16, at.1 + 16));
    let _ = inspector.show();
    state.editor.inspected = Some(index);
    refresh_inspector_feedback(state);
}

fn align_index(align: Align) -> i32 {
    Align::ALL.iter().position(|a| *a == align).unwrap_or(0) as i32
}

fn align_from_index(index: i32) -> Align {
    usize::try_from(index)
        .ok()
        .and_then(|index| Align::ALL.get(index).copied())
        .unwrap_or_default()
}

fn close_inspector(state: &mut State) {
    state.editor.inspected = None;
    if let Some(inspector) = &state.editor.inspector {
        let _ = inspector.hide();
    }
}

/// The colour swatches and render error of the inspected overlay.
fn refresh_inspector_feedback(state: &mut State) {
    let Some(overlay) = state
        .editor
        .inspected
        .and_then(|index| app_overlay(state, index).cloned())
    else {
        return;
    };
    let error = state
        .cache
        .get(&state.renderer, &state.base_dir, &overlay)
        .err()
        .unwrap_or_default();
    let Some(inspector) = &state.editor.inspector else {
        return;
    };
    let fields = inspector.global::<Inspector>();
    if let Content::Text(style) = &overlay.content {
        fields.set_color_value(slint_color(Some(style.color)));
        fields.set_background_value(slint_color(style.background));
    }
    fields.set_render_error(error.into());
}

fn slint_color(color: Option<Color>) -> slint::Color {
    let [r, g, b, a] = color.map_or([0, 0, 0, 0], Color::rgba);
    slint::Color::from_argb_u8(a, r, g, b)
}

fn connect_inspector(ui: &MainWindow, inspector: &InspectorWindow, state: Weak<RefCell<State>>) {
    let fields = inspector.global::<Inspector>();
    let (weak_ui, edited_state) = (ui.as_weak(), state.clone());
    fields.on_edited(move || {
        if let (Some(ui), Some(state)) = (weak_ui.upgrade(), edited_state.upgrade()) {
            inspector_edited(&ui, &state);
        }
    });
    let weak_ui = ui.as_weak();
    fields.on_choose_image(move || {
        if let (Some(ui), Some(state)) = (weak_ui.upgrade(), state.upgrade()) {
            choose_image(&ui, &state);
        }
    });
}

/// Reads every field of the details window back into its overlay.
fn inspector_edited(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    let Some(index) = state.editor.inspected else {
        return;
    };
    let Some(inspector) = &state.editor.inspector else {
        return;
    };
    let fields = inspector.global::<Inspector>();
    let name: String = fields.get_name().into();
    let text: String = fields.get_text().into();
    let font: String = fields.get_font().into();
    let font_size = fields.get_font_size();
    let color = fields.get_color();
    let background = fields.get_background();
    let padding = fields.get_padding();
    let anchor = fields.get_anchor();
    let align = align_from_index(fields.get_align());
    let pivot = align_from_index(fields.get_pivot());
    let offset = [fields.get_offset_x(), fields.get_offset_y()];
    let scale = fields.get_scale();
    let opacity = fields.get_opacity();
    let Some(overlay) = app_mut(&mut state).and_then(|app| app.overlays.get_mut(index)) else {
        return;
    };
    overlay.name = name;
    if let Content::Text(style) = &mut overlay.content {
        style.text = text;
        style.font = font;
        style.size = font_size.max(1) as f32;
        // A colour being typed keeps the last valid one until it parses.
        if let Some(color) = Color::parse(&color) {
            style.color = color;
        }
        if background.trim().is_empty() {
            style.background = None;
        } else if let Some(color) = Color::parse(&background) {
            style.background = Some(color);
        }
        style.padding = padding.max(0) as f32;
    }
    overlay.anchor = if anchor == 1 {
        Anchor::Monitor
    } else {
        Anchor::Window
    };
    overlay.align = align;
    // Kept unset while it matches the position, as overlays from before it existed are.
    overlay.pivot = (pivot != align).then_some(pivot);
    overlay.offset = offset;
    overlay.scale = scale.max(1) as f32 / 100.0;
    overlay.opacity = opacity.clamp(0, 100) as f32 / 100.0;
    overlay_changed(ui, &mut state, index);
}

fn choose_image(ui: &MainWindow, state: &Shared) {
    let Some(path) = pick_image(ui, state) else {
        return;
    };
    let mut state = state.borrow_mut();
    let Some(index) = state.editor.inspected else {
        return;
    };
    let stored = match import_image(&state.base_dir, &path) {
        Ok(stored) => stored,
        Err(error) => {
            show_prompt(ui, PromptKind::ImageFailed, &error.to_string());
            return;
        }
    };
    if let Some(inspector) = &state.editor.inspector {
        inspector
            .global::<Inspector>()
            .set_image_path(stored.as_str().into());
    }
    let Some(overlay) = app_mut(&mut state).and_then(|app| app.overlays.get_mut(index)) else {
        return;
    };
    overlay.content = Content::Image { path: stored };
    overlay_changed(ui, &mut state, index);
}

/// `base`, or `base 2`, `base 3`… whichever no overlay uses yet.
fn unique_name(base: &str, app: &ManagedApplication) -> String {
    unique(base, |name| {
        app.overlays.iter().any(|overlay| overlay.name == name)
    })
}

fn unique_group_name(base: &str, app: &ManagedApplication) -> String {
    unique(base, |name| {
        app.groups.iter().any(|group| group.name == name)
    })
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    // "Hint 2" is numbered from "Hint", not "Hint 2".
    let base = match base.rsplit_once(' ') {
        Some((stem, number)) if !stem.is_empty() && number.parse::<u32>().is_ok() => stem,
        _ => base,
    };
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken(name))
        .unwrap()
}
