use crate::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE_NAME: &str = "settings.json";
pub const IMAGES_DIR_NAME: &str = "images";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Settings {
    #[serde(default)]
    pub apps: Vec<ManagedApplication>,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub language: Language,
}

impl Settings {
    pub fn load_from(path: &Path) -> Result<Self> {
        load_json(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        save_json(path, self)
    }
}

/// The folder that holds `settings.json` and `images/`: the one containing the executable.
pub fn portable_dir() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    Ok(executable
        .parent()
        .ok_or("the executable has no parent folder")?
        .to_path_buf())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ManagedApplication {
    pub identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default = "default_preview_size")]
    pub preview_size: [u32; 2],
}

pub fn default_preview_size() -> [u32; 2] {
    [1920, 1080]
}

impl ManagedApplication {
    pub fn new(identity: String, name: Option<String>) -> Self {
        Self {
            identity,
            name,
            groups: Vec::new(),
            overlays: Vec::new(),
            preview_size: default_preview_size(),
        }
    }

    pub fn matches(&self, process_name: &str, executable_path: Option<&Path>) -> bool {
        if is_path(&self.identity) {
            return executable_path
                .map(|path| {
                    normalize_path(&self.identity) == normalize_path(&path.to_string_lossy())
                })
                .unwrap_or_else(|| normalize_name(&self.identity) == normalize_name(process_name));
        }

        normalize_name(&self.identity) == normalize_name(process_name)
    }

    pub fn same_identity(&self, other: &str) -> bool {
        match (is_path(&self.identity), is_path(other)) {
            (true, true) => normalize_path(&self.identity) == normalize_path(other),
            (false, false) => normalize_name(&self.identity) == normalize_name(other),
            _ => false,
        }
    }

    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.identity)
    }

    pub fn group(&self, id: u32) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Adds a group with the next free id and returns its index.
    pub fn add_group(&mut self, name: String) -> usize {
        let id = self.groups.iter().map(|group| group.id).max().unwrap_or(0) + 1;
        self.groups.push(Group {
            id,
            name,
            hotkey: None,
        });
        self.groups.len() - 1
    }

    /// Removes a group; its overlays stay, no longer in any group.
    pub fn remove_group(&mut self, index: usize) {
        if index >= self.groups.len() {
            return;
        }
        let id = self.groups.remove(index).id;
        for overlay in &mut self.overlays {
            if overlay.group == Some(id) {
                overlay.group = None;
            }
        }
    }

    /// Copies an overlay right after the original and returns the copy's index.
    pub fn duplicate_overlay(&mut self, index: usize, name: String) -> Option<usize> {
        let mut copy = self.overlays.get(index)?.clone();
        copy.name = name;
        self.overlays.insert(index + 1, copy);
        Some(index + 1)
    }
}

fn is_path(value: &str) -> bool {
    value.contains(['\\', '/']) || value.contains(':')
}

fn normalize_path(value: &str) -> String {
    value.trim().replace('/', "\\").to_lowercase()
}

fn normalize_name(value: &str) -> String {
    let file_name = value.trim().rsplit(['\\', '/']).next().unwrap_or_default();
    let lower = file_name.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Group {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<Hotkey>,
}

/// A virtual-key code plus the modifiers that must be held with it; every
/// other modifier must be released.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Hotkey {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    pub key: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Overlay {
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
    pub content: Content,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub align: Align,
    #[serde(default)]
    pub offset: [i32; 2],
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

fn enabled() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl Overlay {
    pub fn text(name: String, text: String) -> Self {
        Self::with_content(name, Content::Text(TextStyle::new(text)))
    }

    pub fn image(name: String, path: String) -> Self {
        Self::with_content(name, Content::Image { path })
    }

    fn with_content(name: String, content: Content) -> Self {
        Self {
            name,
            enabled: true,
            group: None,
            content,
            anchor: Anchor::Window,
            align: Align::TopLeft,
            offset: [16, 16],
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Content {
    Text(TextStyle),
    Image { path: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextStyle {
    pub text: String,
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default = "Color::white")]
    pub color: Color,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    #[serde(default = "default_padding")]
    pub padding: f32,
}

fn default_font() -> String {
    "Segoe UI".into()
}

fn default_font_size() -> f32 {
    24.0
}

fn default_padding() -> f32 {
    8.0
}

impl TextStyle {
    pub fn new(text: String) -> Self {
        Self {
            text,
            font: default_font(),
            size: default_font_size(),
            color: Color::white(),
            background: Some(Color(0x000000a0)),
            padding: default_padding(),
        }
    }
}

/// An sRGB colour with straight alpha, stored as `#RRGGBBAA`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color(pub u32);

impl Color {
    pub fn white() -> Self {
        Self(0xffffffff)
    }

    pub fn rgba(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// Accepts `#RGB`, `#RRGGBB` or `#RRGGBBAA`, with or without the `#`.
    pub fn parse(value: &str) -> Option<Self> {
        let hex = value.trim().trim_start_matches('#');
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let expanded = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect::<String>() + "ff",
            6 => format!("{hex}ff"),
            8 => hex.to_string(),
            _ => return None,
        };
        u32::from_str_radix(&expanded, 16).ok().map(Self)
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:08X}", self.0)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Color::parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid colour {value:?}")))
    }
}

/// The area an overlay is positioned in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Anchor {
    /// The content area of the application window.
    #[default]
    Window,
    /// The monitor that shows the application window.
    Monitor,
}

/// Which point of the anchor area and of the overlay are lined up before the
/// offset is added.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Align {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Align {
    pub const ALL: [Align; 9] = [
        Align::TopLeft,
        Align::Top,
        Align::TopRight,
        Align::Left,
        Align::Center,
        Align::Right,
        Align::BottomLeft,
        Align::Bottom,
        Align::BottomRight,
    ];

    /// Horizontal and vertical position of the point, each 0, 1 or 2.
    pub fn cell(self) -> (i32, i32) {
        let index = Self::ALL.iter().position(|align| *align == self).unwrap() as i32;
        (index % 3, index / 3)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
}

// Folder names under `lang/`; English is the untranslated default.
const TRANSLATIONS: [&str; 1] = ["zh-TW"];

impl Language {
    // Returns the bundled translation to select, where "" means English.
    pub fn translation(self, system_locale: Option<&str>) -> &'static str {
        match self {
            Language::System => system_locale.map_or("", bundled_language),
            Language::English => "",
            Language::TraditionalChinese => "zh-TW",
        }
    }
}

// Matches a locale the same way Slint does: exact name first, then the language part.
fn bundled_language(locale: &str) -> &'static str {
    fn base(locale: &str) -> &str {
        locale
            .find(['-', '_', '@'])
            .map_or(locale, |index| &locale[..index])
    }
    TRANSLATIONS
        .iter()
        .find(|translation| **translation == locale)
        .or_else(|| {
            TRANSLATIONS
                .iter()
                .find(|translation| base(translation) == base(locale))
        })
        .copied()
        .unwrap_or("")
}

/// Copies an image into `images/` beside the settings and returns the path to
/// store, relative to `base_dir`. A file with the same name and content is reused.
pub fn import_image(base_dir: &Path, source: &Path) -> Result<String> {
    let images = base_dir.join(IMAGES_DIR_NAME);
    if source.parent() == Some(images.as_path()) {
        let name = source.file_name().ok_or("the image has no file name")?;
        return Ok(format!("{IMAGES_DIR_NAME}/{}", name.to_string_lossy()));
    }
    let data = std::fs::read(source)?;
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let extension = source
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    std::fs::create_dir_all(&images)?;
    for attempt in 0.. {
        let name = match attempt {
            0 => format!("{stem}{extension}"),
            n => format!("{stem}-{n}{extension}"),
        };
        let target = images.join(&name);
        match std::fs::read(&target) {
            Ok(existing) if existing == data => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&target, &data)?;
            }
            Err(error) => return Err(error.into()),
        }
        return Ok(format!("{IMAGES_DIR_NAME}/{name}"));
    }
    unreachable!()
}

fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write beside the file first, so a failed write never leaves it half written.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("z-overlay-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn one_identity_matches_path_or_process_name() {
        let by_path = ManagedApplication::new(r"C:\Games\ZZZ\zzz.exe".into(), None);
        let by_name = ManagedApplication::new("ZZZ.EXE".into(), None);

        assert!(by_path.matches("zzz.exe", Some(Path::new(r"c:\games\zzz\ZZZ.EXE"))));
        assert!(by_path.matches("zzz.exe", None));
        assert!(!by_path.matches("zzz.exe", Some(Path::new(r"D:\Other\zzz.exe"))));
        assert!(by_name.matches("zzz.exe", Some(Path::new(r"D:\Other\zzz.exe"))));
        assert!(by_name.matches("ZZZ", None));
        assert!(!by_name.matches("yyy.exe", None));
    }

    #[test]
    fn display_name_falls_back_to_identity() {
        let mut app = ManagedApplication::new("game.exe".into(), Some("  ".into()));
        assert_eq!(app.display_name(), "game.exe");
        app.name = Some(" Game ".into());
        assert_eq!(app.display_name(), "Game");
    }

    #[test]
    fn settings_round_trip_as_json() {
        let mut app = ManagedApplication::new(r"C:\Games\game.exe".into(), Some("Game".into()));
        let group = app.add_group("Map".into());
        app.groups[group].hotkey = Some(Hotkey {
            ctrl: true,
            alt: false,
            shift: false,
            key: 0x70,
        });
        let mut text = Overlay::text("Hint".into(), "Hello\n世界".into());
        text.group = Some(app.groups[group].id);
        text.align = Align::BottomRight;
        text.offset = [-8, -8];
        app.overlays.push(text);
        app.overlays
            .push(Overlay::image("Map".into(), "images/map.png".into()));
        let settings = Settings {
            apps: vec![app],
            start_with_windows: true,
            start_minimized: true,
            language: Language::TraditionalChinese,
        };

        let dir = temp_dir("round-trip");
        let path = dir.join(SETTINGS_FILE_NAME);
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path).unwrap(), settings);
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"#000000A0\""), "{json}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_settings_file_loads_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(
            Settings::load_from(&dir.join(SETTINGS_FILE_NAME)).unwrap(),
            Settings::default()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sparse_json_fills_in_defaults() {
        let settings: Settings = serde_json::from_str(
            r#"{"apps":[{"identity":"a.exe","overlays":[{"name":"x","content":{"Text":{"text":"hi"}}}]}]}"#,
        )
        .unwrap();
        let app = &settings.apps[0];
        assert_eq!(app.preview_size, [1920, 1080]);
        let overlay = &app.overlays[0];
        assert!(overlay.enabled);
        assert_eq!(overlay.scale, 1.0);
        assert_eq!(overlay.opacity, 1.0);
        assert_eq!(overlay.anchor, Anchor::Window);
        let Content::Text(style) = &overlay.content else {
            panic!("expected text");
        };
        assert_eq!(style.color, Color::white());
        assert_eq!(style.background, None);
    }

    #[test]
    fn colours_parse_short_long_and_alpha_forms() {
        assert_eq!(Color::parse("#fff"), Some(Color(0xffffffff)));
        assert_eq!(Color::parse("102030"), Some(Color(0x102030ff)));
        assert_eq!(Color::parse("#10203040"), Some(Color(0x10203040)));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("#ggg"), None);
        assert_eq!(Color(0x10203040).to_string(), "#10203040");
        assert_eq!(Color(0x10203040).rgba(), [0x10, 0x20, 0x30, 0x40]);
    }

    #[test]
    fn removing_a_group_ungroups_its_overlays() {
        let mut app = ManagedApplication::new("a.exe".into(), None);
        app.add_group("One".into());
        let two = app.add_group("Two".into());
        let mut overlay = Overlay::text("x".into(), "x".into());
        overlay.group = Some(app.groups[two].id);
        app.overlays.push(overlay);

        app.remove_group(two);
        assert_eq!(app.groups.len(), 1);
        assert_eq!(app.overlays[0].group, None);
        // Ids are not reused while a higher one exists.
        let three = app.add_group("Three".into());
        assert_eq!(app.groups[three].id, 2);
    }

    #[test]
    fn duplicating_an_overlay_inserts_a_copy_after_it() {
        let mut app = ManagedApplication::new("a.exe".into(), None);
        app.overlays.push(Overlay::text("a".into(), "A".into()));
        app.overlays.push(Overlay::text("b".into(), "B".into()));

        assert_eq!(app.duplicate_overlay(0, "a copy".into()), Some(1));
        let names: Vec<_> = app.overlays.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["a", "a copy", "b"]);
        assert_eq!(app.overlays[1].content, app.overlays[0].content);
        assert_eq!(app.duplicate_overlay(5, "none".into()), None);
    }

    #[test]
    fn imported_images_are_copied_once_and_never_overwritten() {
        let dir = temp_dir("import");
        let source_dir = dir.join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        let first = source_dir.join("Map.PNG");
        std::fs::write(&first, b"one").unwrap();

        assert_eq!(import_image(&dir, &first).unwrap(), "images/Map.png");
        assert_eq!(import_image(&dir, &first).unwrap(), "images/Map.png");
        std::fs::write(&first, b"two").unwrap();
        assert_eq!(import_image(&dir, &first).unwrap(), "images/Map-1.png");
        assert_eq!(std::fs::read(dir.join("images/Map.png")).unwrap(), b"one");
        // A file already inside images/ is used where it is.
        assert_eq!(
            import_image(&dir, &dir.join("images").join("Map-1.png")).unwrap(),
            "images/Map-1.png"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn language_selects_bundled_translation() {
        assert_eq!(Language::System.translation(Some("zh-TW")), "zh-TW");
        assert_eq!(Language::System.translation(Some("zh-HK")), "zh-TW");
        assert_eq!(Language::System.translation(Some("en-US")), "");
        assert_eq!(Language::System.translation(None), "");
        assert_eq!(Language::English.translation(Some("zh-TW")), "");
        assert_eq!(Language::TraditionalChinese.translation(None), "zh-TW");
    }
}
