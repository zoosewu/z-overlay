//! Draws overlay content into premultiplied RGBA pixmaps. The overlay windows
//! and the editor preview both use these pixmaps, so they always match.

use crate::Result;
use crate::settings::{Color, Content, TextStyle};
use resvg::tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
use resvg::usvg;
use std::path::Path;
use std::sync::Arc;

/// Overlays larger than this on either side are refused.
pub const MAX_SIDE: u32 = 8192;

const LINE_HEIGHT: f32 = 1.25;

/// Fonts tried, in order, for characters the chosen font does not have.
const FALLBACK_FONTS: [&str; 7] = [
    "Segoe UI",
    "Microsoft JhengHei UI",
    "Microsoft YaHei UI",
    "Yu Gothic UI",
    "Malgun Gothic",
    "Segoe UI Emoji",
    "Segoe UI Symbol",
];

pub struct Renderer {
    options: usvg::Options<'static>,
}

impl Renderer {
    /// Loads the system fonts, which takes a moment; create one and keep it.
    pub fn new() -> Self {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        fonts.set_sans_serif_family("Segoe UI");
        Self {
            options: usvg::Options {
                fontdb: Arc::new(fonts),
                font_family: "Segoe UI".into(),
                font_resolver: no_fallback(),
                ..usvg::Options::default()
            },
        }
    }

    /// Renders content at `scale`. Image paths are relative to `base_dir`.
    pub fn render(&self, content: &Content, base_dir: &Path, scale: f32) -> Result<Pixmap> {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        match content {
            Content::Text(style) => self.render_text(style, scale),
            Content::Image { path } => self.render_image(&base_dir.join(path), scale),
        }
    }

    fn render_text(&self, style: &TextStyle, scale: f32) -> Result<Pixmap> {
        let text = self.text_element(style);
        let probe =
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">{text}</svg>"#);
        let bounds = usvg::Tree::from_str(&probe, &self.options)?
            .root()
            .abs_bounding_box();
        let padding = style.padding.max(0.0);
        let (width, height) = if bounds.width() > 0.0 && bounds.height() > 0.0 {
            (
                bounds.width() + 2.0 * padding,
                bounds.height() + 2.0 * padding,
            )
        } else {
            // Nothing visible: keep a small, empty box so it can still be found and moved.
            (style.size.max(1.0), style.size.max(1.0))
        };
        let background = style
            .background
            .map(|color| {
                format!(
                    r#"<rect width="{width}" height="{height}" rx="{radius}" {fill}/>"#,
                    radius = padding.min(8.0),
                    fill = fill(color)
                )
            })
            .unwrap_or_default();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{background}<g transform="translate({x} {y})">{text}</g></svg>"#,
            x = padding - bounds.x(),
            y = padding - bounds.y(),
        );
        self.render_svg(&usvg::Tree::from_str(&svg, &self.options)?, scale)
    }

    fn render_image(&self, path: &Path, scale: f32) -> Result<Pixmap> {
        let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let is_svg = path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("svgz")
        });
        if is_svg {
            let options = usvg::Options {
                resources_dir: path.parent().map(Path::to_path_buf),
                fontdb: Arc::clone(&self.options.fontdb),
                font_family: self.options.font_family.clone(),
                // SVG files are drawn as they are, with the usual font fallback.
                ..usvg::Options::default()
            };
            return self.render_svg(&usvg::Tree::from_data(&data, &options)?, scale);
        }

        let image = image::load_from_memory(&data)?.into_rgba8();
        let size = IntSize::from_wh(image.width(), image.height()).ok_or("the image is empty")?;
        let mut pixels = image.into_raw();
        premultiply(&mut pixels);
        let source = Pixmap::from_vec(pixels, size).ok_or("the image is empty")?;
        if scale == 1.0 {
            return check_size(source);
        }
        let mut target = new_pixmap(
            source.width() as f32 * scale,
            source.height() as f32 * scale,
        )?;
        target.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            },
            Transform::from_scale(scale, scale),
            None,
        );
        Ok(target)
    }

    fn render_svg(&self, tree: &usvg::Tree, scale: f32) -> Result<Pixmap> {
        let size = tree.size();
        let mut pixmap = new_pixmap(size.width() * scale, size.height() * scale)?;
        resvg::render(
            tree,
            Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap)
    }
}

impl Renderer {
    /// A `<text>` element with one `<tspan>` per line. usvg shapes a whole line
    /// with a single fallback font when the chosen font misses a character, so
    /// each line is split into runs that each name a font that has them.
    fn text_element(&self, style: &TextStyle) -> String {
        let lines: String = style
            .text
            .lines()
            .enumerate()
            .map(|(index, line)| {
                // An empty line still takes up its height.
                let line = if line.is_empty() { "\u{a0}" } else { line };
                let dy = if index == 0 { 0.0 } else { LINE_HEIGHT };
                let runs: String = self
                    .font_runs(line, &style.font)
                    .into_iter()
                    .map(|(family, run)| {
                        format!(
                            r#"<tspan font-family="'{family}'">{}</tspan>"#,
                            escape(&run)
                        )
                    })
                    .collect();
                format!(r#"<tspan x="0" dy="{dy}em">{runs}</tspan>"#)
            })
            .collect();
        format!(
            r#"<text font-size="{size}" {fill} xml:space="preserve">{lines}</text>"#,
            size = style.size.clamp(1.0, 1000.0),
            fill = fill(style.color),
        )
    }

    /// Splits `line` into runs of characters that one font can draw,
    /// preferring `font` and then each of the fallback fonts in turn.
    fn font_runs(&self, line: &str, font: &str) -> Vec<(String, String)> {
        let font = escape(&font.trim().replace(['\'', '"'], ""));
        let characters: Vec<char> = line.chars().collect();
        let fonts = &self.options.fontdb;
        let candidates: Vec<(String, Vec<bool>)> = std::iter::once(font.as_str())
            .chain(FALLBACK_FONTS)
            .filter_map(|family| {
                let id = fonts.query(&usvg::fontdb::Query {
                    families: &[usvg::fontdb::Family::Name(family)],
                    ..Default::default()
                })?;
                let coverage = fonts.with_face_data(id, |data, index| {
                    let face = ttf_parser::Face::parse(data, index).ok()?;
                    Some(
                        characters
                            .iter()
                            .map(|c| face.glyph_index(*c).is_some())
                            .collect(),
                    )
                })??;
                Some((family.to_string(), coverage))
            })
            .collect();
        let preferred = candidates
            .first()
            .map_or_else(|| font.clone(), |(family, _)| family.clone());

        let mut runs: Vec<(String, String)> = Vec::new();
        for (index, character) in characters.iter().enumerate() {
            let family = if character.is_whitespace() {
                // Spaces stay in the run they are in rather than splitting it.
                runs.last().map(|(family, _)| family.clone())
            } else {
                candidates
                    .iter()
                    .find(|(_, coverage)| coverage[index])
                    .map(|(family, _)| family.clone())
            }
            .unwrap_or_else(|| preferred.clone());
            match runs.last_mut() {
                Some((last, run)) if *last == family => run.push(*character),
                _ => runs.push((family, character.to_string())),
            }
        }
        runs
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// usvg shapes a whole line with every run's font and, where a font misses a
/// character, swaps in a fallback that can take over neighbouring glyphs too.
/// `font_runs` already gives every character a font that has it, so the
/// fallback is turned off.
fn no_fallback() -> usvg::FontResolver<'static> {
    usvg::FontResolver {
        select_fallback: Box::new(|_, _, _| None),
        ..usvg::FontResolver::default()
    }
}

fn fill(color: Color) -> String {
    let [r, g, b, a] = color.rgba();
    format!(
        r##"fill="#{r:02x}{g:02x}{b:02x}" fill-opacity="{}""##,
        a as f32 / 255.0
    )
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    escaped
}

fn new_pixmap(width: f32, height: f32) -> Result<Pixmap> {
    let (width, height) = (width.ceil().max(1.0) as u32, height.ceil().max(1.0) as u32);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(too_large(width, height));
    }
    Ok(Pixmap::new(width, height).ok_or("could not allocate the overlay")?)
}

fn check_size(pixmap: Pixmap) -> Result<Pixmap> {
    if pixmap.width() > MAX_SIDE || pixmap.height() > MAX_SIDE {
        return Err(too_large(pixmap.width(), pixmap.height()));
    }
    Ok(pixmap)
}

fn too_large(width: u32, height: u32) -> Box<dyn std::error::Error + Send + Sync> {
    format!("{width} × {height} is larger than {MAX_SIDE} × {MAX_SIDE} pixels").into()
}

fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u16;
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * alpha + 127) / 255) as u8;
        }
    }
}

/// Premultiplied BGRA rows, as a Windows 32-bit DIB with alpha expects them.
pub fn to_bgra(pixmap: &Pixmap) -> Vec<u8> {
    let mut data = pixmap.data().to_vec();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::TextStyle;
    use std::sync::OnceLock;

    fn renderer() -> &'static Renderer {
        static RENDERER: OnceLock<Renderer> = OnceLock::new();
        RENDERER.get_or_init(Renderer::new)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("z-overlay-render-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn opaque_pixels(pixmap: &Pixmap) -> usize {
        pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count()
    }

    fn text(value: &str) -> Content {
        Content::Text(TextStyle {
            background: None,
            padding: 0.0,
            ..TextStyle::new(value.into())
        })
    }

    #[test]
    fn text_grows_with_lines_and_scale() {
        let one = renderer()
            .render(&text("Hello"), Path::new("."), 1.0)
            .unwrap();
        let two = renderer()
            .render(&text("Hello\nHello"), Path::new("."), 1.0)
            .unwrap();
        let big = renderer()
            .render(&text("Hello"), Path::new("."), 2.0)
            .unwrap();
        assert!(opaque_pixels(&one) > 0);
        assert!(
            two.height() > one.height() * 3 / 2,
            "{} vs {}",
            two.height(),
            one.height()
        );
        assert!(big.width() >= one.width() * 2 - 1 && big.width() <= one.width() * 2 + 1);
    }

    #[test]
    fn chinese_text_falls_back_to_a_font_that_has_it() {
        let pixmap = renderer()
            .render(&text("中文"), Path::new("."), 1.0)
            .unwrap();
        assert!(opaque_pixels(&pixmap) > 50);
    }

    #[test]
    fn mixed_text_keeps_latin_in_the_chosen_font() {
        let runs = renderer().font_runs("技能 Skill 1", "Segoe UI");
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_ne!(runs[0].0, "Segoe UI");
        assert_eq!(runs[0].1, "技能 ");
        assert_eq!(runs[1], ("Segoe UI".to_string(), "Skill 1".to_string()));
    }

    #[test]
    fn an_unknown_font_falls_back_to_segoe_ui() {
        let runs = renderer().font_runs("Hi", "No Such Font");
        assert_eq!(runs, [("Segoe UI".to_string(), "Hi".to_string())]);
    }

    #[test]
    fn markup_in_text_is_drawn_literally() {
        let pixmap = renderer()
            .render(&text("<b>&\"'</b>"), Path::new("."), 1.0)
            .unwrap();
        assert!(opaque_pixels(&pixmap) > 0);
    }

    #[test]
    fn background_fills_the_padded_box() {
        let content = Content::Text(TextStyle {
            background: Some(Color(0x000000ff)),
            padding: 10.0,
            ..TextStyle::new("Hi".into())
        });
        let pixmap = renderer().render(&content, Path::new("."), 1.0).unwrap();
        // The centre of the top padding is solid background.
        let index = (5 * pixmap.width() + pixmap.width() / 2) as usize * 4;
        assert_eq!(&pixmap.data()[index..index + 4], [0, 0, 0, 255]);
    }

    #[test]
    fn png_and_svg_images_render_at_scale() {
        let dir = temp_dir("images");
        let mut image = image::RgbaImage::new(10, 4);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 128]));
        image.save(dir.join("a.png")).unwrap();
        std::fs::write(
            dir.join("b.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#00f"/></svg>"##,
        )
        .unwrap();

        let png = |scale| {
            renderer()
                .render(
                    &Content::Image {
                        path: "a.png".into(),
                    },
                    &dir,
                    scale,
                )
                .unwrap()
        };
        let one = png(1.0);
        assert_eq!((one.width(), one.height()), (10, 4));
        // Premultiplied: half-transparent red is stored at half intensity.
        assert_eq!(&one.data()[..4], [128, 0, 0, 128]);
        let half = png(0.5);
        assert_eq!((half.width(), half.height()), (5, 2));

        let svg = renderer()
            .render(
                &Content::Image {
                    path: "b.svg".into(),
                },
                &dir,
                3.0,
            )
            .unwrap();
        assert_eq!((svg.width(), svg.height()), (60, 30));
        assert_eq!(&svg.data()[..4], [0, 0, 255, 255]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_image_is_an_error_naming_the_file() {
        let error = renderer()
            .render(
                &Content::Image {
                    path: "nope.png".into(),
                },
                Path::new("."),
                1.0,
            )
            .unwrap_err();
        assert!(error.to_string().contains("nope.png"));
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let mut pixmap = Pixmap::new(1, 1).unwrap();
        pixmap.data_mut().copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(to_bgra(&pixmap), [3, 2, 1, 4]);
    }
}
