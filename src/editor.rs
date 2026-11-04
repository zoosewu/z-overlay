//! The overlay editor: one managed application's overlays and groups, with a
//! preview canvas on which overlays are dragged into place.

use crate::app::{
    Removal, Shared, State, file_dialog, overlays_changed, save_settings, show_prompt, sync_apps,
};
use crate::keys::{HotkeyRecorder, Recorded, hotkey_label};
use crate::{Editor, GroupRow, MainWindow, OverlayRow, PromptKind};
use resvg::tiny_skia::Pixmap;
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::path::PathBuf;
use std::rc::Rc;
use z_overlay::placement::{Rect, offset_for, place};
use z_overlay::settings::{
    Align, Anchor, Color, Content, ManagedApplication, Overlay, import_image,
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

#[derive(Default)]
pub struct EditorState {
    /// The managed application being edited.
    app: Option<usize>,
    recording: Option<(usize, HotkeyRecorder)>,
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
    editor_on!(ui, state, on_select, select, index);
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
    editor_on!(ui, state, on_duplicate, duplicate, index);
    editor_on!(ui, state, on_remove, remove, index);
    editor_on!(ui, state, on_edited, edited);
    editor_on!(ui, state, on_choose_image, choose_image);
    editor_on!(ui, state, on_dragged, dragged, index, x, y);
    editor_on!(ui, state, on_select_group, select_group, index);
    editor_on!(ui, state, on_add_group, add_group, name);
    editor_on!(ui, state, on_remove_group, remove_group_clicked, index);
    editor_on!(ui, state, on_group_renamed, group_renamed, index, name);
    editor_on!(ui, state, on_record_hotkey, record_hotkey, index);
    editor_on!(ui, state, on_clear_hotkey, clear_hotkey, index);
}

pub fn open(ui: &MainWindow, state: &mut State, app: usize) {
    if app >= state.settings.apps.len() {
        return;
    }
    state.editor = EditorState {
        app: Some(app),
        recording: None,
    };
    // Images may have changed on disk since they were last drawn.
    state.cache.clear();
    let editor = ui.global::<Editor>();
    editor.set_selected(-1);
    editor.set_selected_group(-1);
    editor.set_recording_group(-1);
    sync_all(ui, state);
    ui.set_overlay_editor_open(true);
}

fn close(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    state.editor = EditorState::default();
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
    sync_groups(ui, state);
    sync_rows(ui, state);
    sync_selection(ui, state);
}

fn sync_rows(ui: &MainWindow, state: &mut State) {
    let count = app(state).map_or(0, |app| app.overlays.len());
    let rows: Vec<OverlayRow> = (0..count)
        .filter_map(|index| overlay_row(state, index))
        .collect();
    ui.global::<Editor>()
        .set_overlays(ModelRc::new(VecModel::from(rows)));
}

fn sync_row(ui: &MainWindow, state: &mut State, index: usize) {
    let overlays = ui.global::<Editor>().get_overlays();
    if index < overlays.row_count()
        && let Some(row) = overlay_row(state, index)
    {
        overlays.set_row_data(index, row);
    }
}

fn overlay_row(state: &mut State, index: usize) -> Option<OverlayRow> {
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
    let (x, y) = place(preview_area(app), overlay.align, overlay.offset, size);
    Some(OverlayRow {
        name: overlay.name.as_str().into(),
        enabled: overlay.enabled,
        group: overlay
            .group
            .and_then(|id| app.group(id))
            .map_or("", |group| group.name.as_str())
            .into(),
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

fn sync_groups(ui: &MainWindow, state: &State) {
    let Some(app) = app(state) else {
        return;
    };
    let editor = ui.global::<Editor>();
    let rows: Vec<GroupRow> = app
        .groups
        .iter()
        .map(|group| GroupRow {
            name: group.name.as_str().into(),
            hotkey: group
                .hotkey
                .as_ref()
                .map(hotkey_label)
                .unwrap_or_default()
                .into(),
        })
        .collect();
    editor.set_groups(ModelRc::new(VecModel::from(rows)));
    let selected = usize::try_from(editor.get_selected_group())
        .ok()
        .and_then(|index| app.groups.get(index));
    match selected {
        Some(group) => editor.set_group_name(group.name.as_str().into()),
        None => editor.set_selected_group(-1),
    }
    let mut choices = vec![no_group_label(ui)];
    choices.extend(app.groups.iter().map(|group| group.name.as_str().into()));
    editor.set_group_choices(ModelRc::new(VecModel::from(choices)));
}

/// The first group choice keeps whatever translated label the UI gave it.
fn no_group_label(ui: &MainWindow) -> slint::SharedString {
    let label = ui.global::<Editor>().get_no_group_label();
    if label.is_empty() {
        "—".into()
    } else {
        label
    }
}

/// Fills the property fields from the selected overlay.
fn sync_selection(ui: &MainWindow, state: &mut State) {
    let editor = ui.global::<Editor>();
    let Some((app, overlay)) = selected(ui, state) else {
        editor.set_selected(-1);
        return;
    };
    editor.set_name(overlay.name.as_str().into());
    match &overlay.content {
        Content::Text(style) => {
            editor.set_is_text(true);
            editor.set_text(style.text.as_str().into());
            editor.set_font(style.font.as_str().into());
            editor.set_font_size(style.size.round() as i32);
            editor.set_color(style.color.to_string().into());
            editor.set_background(
                style
                    .background
                    .map(|c| c.to_string())
                    .unwrap_or_default()
                    .into(),
            );
            editor.set_padding(style.padding.round() as i32);
        }
        Content::Image { path } => {
            editor.set_is_text(false);
            editor.set_image_path(path.as_str().into());
        }
    }
    editor.set_group_index(
        overlay
            .group
            .and_then(|id| app.groups.iter().position(|group| group.id == id))
            .map_or(0, |index| index as i32 + 1),
    );
    editor.set_anchor(match overlay.anchor {
        Anchor::Window => 0,
        Anchor::Monitor => 1,
    });
    editor.set_align(
        Align::ALL
            .iter()
            .position(|a| *a == overlay.align)
            .unwrap_or(0) as i32,
    );
    editor.set_offset_x(overlay.offset[0]);
    editor.set_offset_y(overlay.offset[1]);
    editor.set_scale((overlay.scale * 100.0).round() as i32);
    editor.set_opacity((overlay.opacity * 100.0).round() as i32);
    sync_feedback(ui, state);
}

/// The colour swatches and render error of the selected overlay.
fn sync_feedback(ui: &MainWindow, state: &mut State) {
    let editor = ui.global::<Editor>();
    let Some(index) = selected_index(ui, state) else {
        return;
    };
    let Some(app) = app(state) else {
        return;
    };
    let overlay = &app.overlays[index];
    if let Content::Text(style) = &overlay.content {
        editor.set_color_value(slint_color(Some(style.color)));
        editor.set_background_value(slint_color(style.background));
    }
    let overlay = overlay.clone();
    let error = state
        .cache
        .get(&state.renderer, &state.base_dir, &overlay)
        .err()
        .unwrap_or_default();
    editor.set_render_error(error.into());
}

fn slint_color(color: Option<Color>) -> slint::Color {
    let [r, g, b, a] = color.map_or([0, 0, 0, 0], Color::rgba);
    slint::Color::from_argb_u8(a, r, g, b)
}

fn selected_index(ui: &MainWindow, state: &State) -> Option<usize> {
    let index = usize::try_from(ui.global::<Editor>().get_selected()).ok()?;
    (index < app(state)?.overlays.len()).then_some(index)
}

fn selected<'a>(
    ui: &MainWindow,
    state: &'a State,
) -> Option<(&'a ManagedApplication, &'a Overlay)> {
    let index = selected_index(ui, state)?;
    let app = app(state)?;
    Some((app, &app.overlays[index]))
}

/// Saves after an overlay changed and redraws it.
fn overlay_changed(ui: &MainWindow, state: &mut State, index: usize) {
    save_settings(ui, state);
    overlays_changed(state);
    sync_row(ui, state, index);
    sync_feedback(ui, state);
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

fn select(ui: &MainWindow, state: &Shared, index: i32) {
    ui.global::<Editor>().set_selected(index);
    sync_selection(ui, &mut state.borrow_mut());
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

fn add_text(ui: &MainWindow, state: &Shared, name: slint::SharedString) {
    let mut state = state.borrow_mut();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let name = unique_name(&name, app);
    app.overlays.push(Overlay::text(name.clone(), name));
    added(ui, &mut state);
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
    added(ui, &mut state);
}

/// Selects the overlay just added at the end of the list.
fn added(ui: &MainWindow, state: &mut State) {
    let count = app(state).map_or(0, |app| app.overlays.len());
    save_settings(ui, state);
    overlays_changed(state);
    sync_rows(ui, state);
    ui.global::<Editor>().set_selected(count as i32 - 1);
    sync_selection(ui, state);
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
    save_settings(ui, &state);
    overlays_changed(&mut state);
    sync_rows(ui, &mut state);
    ui.global::<Editor>().set_selected(copy as i32);
    sync_selection(ui, &mut state);
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
    save_settings(ui, state);
    overlays_changed(state);
    ui.global::<Editor>().set_selected(-1);
    sync_rows(ui, state);
    sync_selection(ui, state);
}

/// Reads every property field back into the selected overlay.
fn edited(ui: &MainWindow, state: &Shared) {
    let mut state = state.borrow_mut();
    let editor = ui.global::<Editor>();
    let Some(index) = selected_index(ui, &state) else {
        return;
    };
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let group = usize::try_from(editor.get_group_index() - 1)
        .ok()
        .and_then(|index| app.groups.get(index))
        .map(|group| group.id);
    let overlay = &mut app.overlays[index];
    overlay.name = editor.get_name().into();
    if let Content::Text(style) = &mut overlay.content {
        style.text = editor.get_text().into();
        style.font = editor.get_font().into();
        style.size = editor.get_font_size().max(1) as f32;
        // A colour being typed keeps the last valid one until it parses.
        if let Some(color) = Color::parse(&editor.get_color()) {
            style.color = color;
        }
        let background = editor.get_background();
        if background.trim().is_empty() {
            style.background = None;
        } else if let Some(color) = Color::parse(&background) {
            style.background = Some(color);
        }
        style.padding = editor.get_padding().max(0) as f32;
    }
    overlay.group = group;
    overlay.anchor = if editor.get_anchor() == 1 {
        Anchor::Monitor
    } else {
        Anchor::Window
    };
    overlay.align = usize::try_from(editor.get_align())
        .ok()
        .and_then(|index| Align::ALL.get(index).copied())
        .unwrap_or_default();
    overlay.offset = [editor.get_offset_x(), editor.get_offset_y()];
    overlay.scale = editor.get_scale().max(1) as f32 / 100.0;
    overlay.opacity = editor.get_opacity().clamp(0, 100) as f32 / 100.0;
    overlay_changed(ui, &mut state, index);
}

fn choose_image(ui: &MainWindow, state: &Shared) {
    let Some(path) = pick_image(ui, state) else {
        return;
    };
    let mut state = state.borrow_mut();
    let Some(index) = selected_index(ui, &state) else {
        return;
    };
    let stored = match import_image(&state.base_dir, &path) {
        Ok(stored) => stored,
        Err(error) => {
            show_prompt(ui, PromptKind::ImageFailed, &error.to_string());
            return;
        }
    };
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    app.overlays[index].content = Content::Image { path: stored };
    ui.global::<Editor>()
        .set_image_path(match &app.overlays[index].content {
            Content::Image { path } => path.as_str().into(),
            Content::Text(_) => "".into(),
        });
    overlay_changed(ui, &mut state, index);
}

/// An overlay was dropped with its top-left corner at (x, y) of the preview frame.
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
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let area = preview_area(app);
    let Some(overlay) = app.overlays.get_mut(index) else {
        return;
    };
    overlay.offset = offset_for(
        area,
        overlay.align,
        (x.round() as i32, y.round() as i32),
        size,
    );
    let offset = overlay.offset;
    let editor = ui.global::<Editor>();
    if selected_index(ui, &state) == Some(index) {
        editor.set_offset_x(offset[0]);
        editor.set_offset_y(offset[1]);
    }
    overlay_changed(ui, &mut state, index);
}

fn select_group(ui: &MainWindow, state: &Shared, index: i32) {
    let state = state.borrow();
    let editor = ui.global::<Editor>();
    let Some(group) = usize::try_from(index)
        .ok()
        .and_then(|index| app(&state)?.groups.get(index))
    else {
        return;
    };
    editor.set_selected_group(index);
    editor.set_group_name(group.name.as_str().into());
}

fn add_group(ui: &MainWindow, state: &Shared, name: slint::SharedString) {
    let mut state = state.borrow_mut();
    let Some(app) = app_mut(&mut state) else {
        return;
    };
    let name = unique_group_name(&name, app);
    let index = app.add_group(name);
    save_settings(ui, &state);
    ui.global::<Editor>().set_selected_group(index as i32);
    groups_changed(ui, &mut state);
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
    save_settings(ui, state);
    stop_recording(ui, state);
    ui.global::<Editor>().set_selected_group(-1);
    groups_changed(ui, state);
}

fn group_renamed(ui: &MainWindow, state: &Shared, index: i32, name: slint::SharedString) {
    let mut state = state.borrow_mut();
    let Some(group) = usize::try_from(index)
        .ok()
        .and_then(|index| app_mut(&mut state)?.groups.get_mut(index))
    else {
        return;
    };
    group.name = name.into();
    save_settings(ui, &state);
    groups_changed(ui, &mut state);
}

/// Group names show in the group list, the group choices and the overlay list.
fn groups_changed(ui: &MainWindow, state: &mut State) {
    overlays_changed(state);
    sync_groups(ui, state);
    sync_rows(ui, state);
    sync_selection(ui, state);
}

fn record_hotkey(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    // A second click stops recording.
    if state
        .editor
        .recording
        .as_ref()
        .is_some_and(|(group, _)| *group == index)
    {
        stop_recording(ui, &mut state);
        return;
    }
    state.editor.recording = Some((index, HotkeyRecorder::default()));
    ui.global::<Editor>().set_recording_group(index as i32);
}

fn stop_recording(ui: &MainWindow, state: &mut State) {
    state.editor.recording = None;
    ui.global::<Editor>().set_recording_group(-1);
}

/// Called every frame; finishes recording once a hotkey is pressed.
pub fn poll_recording(ui: &MainWindow, state: &mut State) {
    let Some((group, recorder)) = &mut state.editor.recording else {
        return;
    };
    let group = *group;
    match recorder.poll() {
        Recorded::Waiting => {}
        Recorded::Canceled => stop_recording(ui, state),
        Recorded::Key(hotkey) => {
            if let Some(slot) = app_mut(state).and_then(|app| app.groups.get_mut(group)) {
                slot.hotkey = Some(hotkey);
            }
            stop_recording(ui, state);
            save_settings(ui, state);
            sync_groups(ui, state);
        }
    }
}

fn clear_hotkey(ui: &MainWindow, state: &Shared, index: i32) {
    let mut state = state.borrow_mut();
    stop_recording(ui, &mut state);
    let Some(group) = usize::try_from(index)
        .ok()
        .and_then(|index| app_mut(&mut state)?.groups.get_mut(index))
    else {
        return;
    };
    group.hotkey = None;
    save_settings(ui, &state);
    sync_groups(ui, &state);
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
