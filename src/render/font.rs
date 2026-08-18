//! Font spec parsing and system font resolution.

use cosmic_text::FontSystem;

/// A parsed Xft-style font description: "Family:size=12:pixelsize=20:..."
#[derive(Debug, Clone)]
pub struct FontSpec {
    pub family: String,
    /// Pixel size (already converted from points for `size=`).
    pub px: f32,
}

pub fn parse_font_name(name: &str) -> FontSpec {
    let mut family = name.to_string();
    let mut px: f32 = 0.0;
    for (i, part) in name.split(':').enumerate() {
        if i == 0 {
            family = part.to_string();
            continue;
        }
        if let Some(value) = part.strip_prefix("pixelsize=") {
            if let Ok(v) = value.parse::<f32>() {
                px = v;
            }
        } else if let Some(value) = part.strip_prefix("size=") {
            if let Ok(v) = value.parse::<f32>() {
                // Xft converts points to pixels via dpi (96 by default).
                px = (v * 96.0 / 72.0).round();
            }
        }
    }
    if px <= 0.0 {
        px = 12.0 * 96.0 / 72.0;
    }
    FontSpec { family, px }
}

/// Resolve an Xft-style family name to a family present in the font database
/// (fontconfig-style loose matching, like the instantWM text rasterizer).
pub(super) fn resolve_family(font_system: &FontSystem, configured: &str) -> String {
    let db = font_system.db();
    // exact match first
    let exact = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name))
        .find(|name| name.eq_ignore_ascii_case(configured))
        .cloned();
    if let Some(name) = exact {
        return name;
    }
    let wanted = normalized_family(configured);
    let loose = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name))
        .find(|name| normalized_family(name) == wanted)
        .cloned();
    if let Some(name) = loose {
        return name;
    }
    // family names may embed the style: "Inter-Regular" -> try "Inter"
    if let Some((base, _rest)) = configured.rsplit_once('-') {
        if !base.is_empty() {
            let base_wanted = normalized_family(base);
            let base_hit = db
                .faces()
                .flat_map(|face| face.families.iter().map(|(name, _)| name))
                .find(|name| normalized_family(name) == base_wanted)
                .cloned();
            if let Some(name) = base_hit {
                return name;
            }
        }
    }
    configured.to_string()
}

fn normalized_family(family: &str) -> String {
    family
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Height (ascent + descent) of the primary font at `px`, matching Xft's
/// `font->h = ascent + descent`.
pub(super) fn primary_font_height(font_system: &mut FontSystem, families: &[String], px: f32) -> i32 {
    let db = font_system.db();
    let mut height = (px * 1.2).ceil() as i32;
    for family in families {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(face_id) = db.query(&query) else { continue };
        /* FaceInfo carries no metrics; read them from the face data itself */
        let h = db
            .with_face_data(face_id, |data, index| {
                ttf_parser::Face::parse(data, index).map_or(0, |face| {
                    let upem = face.units_per_em().max(1) as f32;
                    let ascent = face.ascender() as f32;
                    let descent = face.descender() as f32; // negative
                    ((ascent - descent) / upem * px).round() as i32
                })
            })
            .unwrap_or(0);
        if h <= 0 {
            continue;
        }
        height = h;
        break;
    }
    height
}
