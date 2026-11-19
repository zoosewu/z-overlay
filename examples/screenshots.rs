//! Draws the screenshots in `docs/screenshots/` from demo data, without
//! opening a window: `cargo run --example screenshots`.

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{
    ComponentHandle, Image, ModelRc, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, VecModel,
};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use z_overlay::placement::{Rect, place};
use z_overlay::render::Renderer;
use z_overlay::settings::{Align, Color, Content, Overlay, TextStyle};

slint::include_modules!();

const SIZE: (u32, u32) = (1280, 800);
const OUT: &str = "docs/screenshots";

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };
}

/// Gives every window an off-screen software renderer.
struct Headless;

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOWS.with(|windows| windows.borrow_mut().push(Rc::clone(&window)));
        Ok(window)
    }
}

/// The text of the demo, in one language.
struct Words {
    language: &'static str,
    game: &'static str,
    player: &'static str,
    notes: &'static str,
    crosshair: &'static str,
    cooldowns: &'static str,
    cooldowns_text: &'static str,
    map_tips: &'static str,
    map_tips_text: &'static str,
    logo: &'static str,
    combat: &'static str,
    map: &'static str,
}

const ENGLISH: Words = Words {
    language: "en",
    game: "Demo Game",
    player: "Media Player",
    notes: "Notes",
    crosshair: "Crosshair",
    cooldowns: "Cooldowns",
    cooldowns_text: "Cooldowns\nQ  4 s   E  12 s   R  60 s",
    map_tips: "Map tips",
    map_tips_text: "M  map   Tab  objectives",
    logo: "Logo",
    combat: "Combat",
    map: "Map",
};

const CHINESE: Words = Words {
    language: "zh-TW",
    game: "示範遊戲",
    player: "媒體播放器",
    notes: "記事本",
    crosshair: "準心",
    cooldowns: "技能冷卻",
    cooldowns_text: "技能冷卻\nQ  4 秒   E  12 秒   R  60 秒",
    map_tips: "地圖提示",
    map_tips_text: "M 地圖 · Tab 任務目標",
    logo: "標誌",
    combat: "戰鬥",
    map: "地圖",
};

/// The demo overlays: crosshair, two texts in groups, and an image.
fn overlays(words: &Words) -> Vec<(Overlay, Option<u32>)> {
    let image = |name: &str, path: &str| {
        let mut overlay = Overlay::image(name.into(), path.into());
        overlay.scale = 0.3;
        overlay
    };
    let text = |name: &str, text: &str, align: Align, offset: [i32; 2]| {
        let mut overlay = Overlay::text(name.into(), text.into());
        if let Content::Text(style) = &mut overlay.content {
            *style = TextStyle {
                color: Color(0xffd966ff),
                size: 44.0,
                ..TextStyle::new(text.into())
            };
        }
        overlay.align = align;
        overlay.offset = offset;
        overlay
    };
    let mut crosshair = Overlay::image(words.crosshair.into(), "samples/crosshair.svg".into());
    crosshair.align = Align::Center;
    crosshair.offset = [0, 0];
    let mut logo = image(words.logo, "assets/z-overlay.png");
    logo.offset = [32, 32];
    vec![
        (crosshair, None),
        (logo, None),
        (
            text(
                words.cooldowns,
                words.cooldowns_text,
                Align::TopRight,
                [-32, 32],
            ),
            Some(1),
        ),
        (
            text(
                words.map_tips,
                words.map_tips_text,
                Align::BottomLeft,
                [32, -32],
            ),
            Some(2),
        ),
    ]
}

fn main() {
    slint::platform::set_platform(Box::new(Headless)).expect("no other platform is set");
    std::fs::create_dir_all(OUT).unwrap();
    let renderer = Renderer::new();
    for words in [&CHINESE, &ENGLISH] {
        let suffix = words.language;
        let ui = main_window(words, &renderer);
        // Translations are only available once a window exists.
        let translation = if words.language == "en" {
            ""
        } else {
            words.language
        };
        slint::select_bundled_translation(translation).unwrap();

        ui.set_overlay_editor_open(true);
        let editor = capture(ui.window(), SIZE);
        save(&editor, &format!("01-overlay-editor.{suffix}.png"));

        let details = inspector(words);
        let panel = framed(&capture(details.window(), (380, 600)));
        let mut combined = editor.clone();
        // Over the preview, where it opens next to a right-clicked overlay.
        image::imageops::overlay(&mut combined, &panel, 884, 120);
        save(&combined, &format!("02-overlay-details.{suffix}.png"));

        ui.set_overlay_editor_open(false);
        ui.set_tab(0);
        save(
            &capture(ui.window(), SIZE),
            &format!("03-applications.{suffix}.png"),
        );
        ui.set_tab(1);
        save(
            &capture(ui.window(), SIZE),
            &format!("04-settings.{suffix}.png"),
        );
    }
}

fn main_window(words: &Words, renderer: &Renderer) -> MainWindow {
    let ui = MainWindow::new().unwrap();
    ui.global::<Palette>()
        .set_color_scheme(slint::language::ColorScheme::Dark);
    let apps = [
        (words.game, 4, 2),
        (words.player, 1, 0),
        (words.notes, 2, 1),
    ]
    .map(|(label, overlays, groups)| ManagedEntry {
        label: label.into(),
        overlays,
        groups,
    });
    ui.set_managed_apps(ModelRc::new(VecModel::from(apps.to_vec())));
    ui.set_managed_current(0);
    ui.set_active_name(words.game.into());
    ui.set_start_with_windows(true);
    ui.set_start_minimized(true);
    ui.set_language_index(0);
    ui.set_settings_folder(r"C:\Tools\z-overlay".into());
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let hotkeys: Vec<slint::SharedString> = (1..=9).map(|n| format!("Ctrl+{n}").into()).collect();
    ui.set_number_hotkeys(ModelRc::new(VecModel::from(hotkeys)));

    let editor = ui.global::<Editor>();
    editor.set_title(words.game.into());
    editor.set_preview_index(1);
    let overlays = overlays(words);
    let tree = |kind, index, nested, name: &str| TreeRow {
        kind,
        index,
        nested,
        name: name.into(),
        enabled: true,
        ..TreeRow::default()
    };
    let group = |index, number, name: &str, hold| TreeRow {
        number,
        hotkey: format!("Ctrl+{number}").into(),
        hold,
        ..tree(1, index, false, name)
    };
    let rows = vec![
        tree(0, 0, false, ""),
        tree(2, 0, false, words.crosshair),
        tree(2, 1, false, words.logo),
        group(0, 1, words.combat, false),
        tree(2, 2, true, words.cooldowns),
        group(1, 2, words.map, true),
        tree(2, 3, true, words.map_tips),
    ];
    editor.set_rows(ModelRc::new(VecModel::from(rows)));
    editor.set_selected(2);
    let area = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    let previews: Vec<OverlayRow> = overlays
        .iter()
        .map(|(overlay, _)| {
            let pixmap = renderer
                .render(&overlay.content, Path::new("."), overlay.scale)
                .unwrap();
            let size = (pixmap.width() as i32, pixmap.height() as i32);
            let (x, y) = place(area, overlay.align, overlay.pivot(), overlay.offset, size);
            let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                pixmap.data(),
                pixmap.width(),
                pixmap.height(),
            );
            OverlayRow {
                name: overlay.name.as_str().into(),
                enabled: true,
                preview: Image::from_rgba8_premultiplied(buffer),
                x: x as f32,
                y: y as f32,
                width: size.0 as f32,
                height: size.1 as f32,
                opacity: overlay.opacity,
                failed: false,
            }
        })
        .collect();
    editor.set_overlays(ModelRc::new(VecModel::from(previews)));
    ui
}

/// The details of the cooldowns overlay.
fn inspector(words: &Words) -> InspectorWindow {
    let window = InspectorWindow::new().unwrap();
    window
        .global::<Palette>()
        .set_color_scheme(slint::language::ColorScheme::Dark);
    let fields = window.global::<Inspector>();
    fields.set_name(words.cooldowns.into());
    fields.set_is_text(true);
    fields.set_text(words.cooldowns_text.into());
    fields.set_font("Segoe UI".into());
    fields.set_font_size(28);
    fields.set_color("#FFD966FF".into());
    fields.set_color_value(slint::Color::from_rgb_u8(0xff, 0xd9, 0x66));
    fields.set_background("#000000A0".into());
    fields.set_background_value(slint::Color::from_argb_u8(0xa0, 0, 0, 0));
    fields.set_padding(8);
    fields.set_anchor(0);
    fields.set_align(2);
    fields.set_pivot(2);
    fields.set_offset_x(-32);
    fields.set_offset_y(32);
    fields.set_scale(100);
    fields.set_opacity(100);
    window
}

/// Draws a window at `size` and returns its pixels.
fn capture(window: &slint::Window, size: (u32, u32)) -> image::RgbaImage {
    let adapter = WINDOWS
        .with(|windows| {
            windows
                .borrow()
                .iter()
                .find(|adapter| std::ptr::eq(adapter.window(), window))
                .cloned()
        })
        .expect("the window was made by this platform");
    window.set_size(PhysicalSize::new(size.0, size.1));
    let _ = window.show();
    let mut pixels = vec![PremultipliedRgbaColor::default(); (size.0 * size.1) as usize];
    // The first pass lays out lists; the second draws them filled in.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        adapter.request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size.0 as usize);
        });
    }
    image::RgbaImage::from_fn(size.0, size.1, |x, y| {
        let pixel = pixels[(y * size.0 + x) as usize];
        image::Rgba([pixel.red, pixel.green, pixel.blue, 255])
    })
}

/// A window's pixels inside a one-pixel frame, so it stands out from what is behind it.
fn framed(content: &image::RgbaImage) -> image::RgbaImage {
    let (width, height) = (content.width() + 2, content.height() + 2);
    let mut framed =
        image::RgbaImage::from_pixel(width, height, image::Rgba([0x60, 0x60, 0x60, 255]));
    image::imageops::overlay(&mut framed, content, 1, 1);
    framed
}

fn save(image: &image::RgbaImage, name: &str) {
    let path = Path::new(OUT).join(name);
    image.save(&path).unwrap();
    println!("{}", path.display());
}
