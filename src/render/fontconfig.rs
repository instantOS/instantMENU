//! Minimal in-process fontconfig integration. Fontconfig already maintains a
//! cache of installed-font metadata, so querying it avoids reparsing every
//! font file for each short-lived menu process.

use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_uchar, CStr, CString};
use std::path::{Path, PathBuf};

use super::font::FontSpec;

enum FcConfig {}
enum FcPattern {}
enum FcCharSet {}

#[repr(C)]
struct FcFontSet {
    nfont: c_int,
    _sfont: c_int,
    fonts: *mut *mut FcPattern,
}

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcInitLoadConfigAndFonts() -> *mut FcConfig;
    fn FcConfigDestroy(config: *mut FcConfig);
    fn FcPatternCreate() -> *mut FcPattern;
    fn FcPatternDestroy(pattern: *mut FcPattern);
    fn FcPatternAddString(
        pattern: *mut FcPattern,
        object: *const c_char,
        value: *const c_uchar,
    ) -> c_int;
    fn FcPatternAddCharSet(
        pattern: *mut FcPattern,
        object: *const c_char,
        charset: *const FcCharSet,
    ) -> c_int;
    fn FcPatternGetString(
        pattern: *const FcPattern,
        object: *const c_char,
        index: c_int,
        value: *mut *mut c_uchar,
    ) -> c_int;
    fn FcPatternGetCharSet(
        pattern: *const FcPattern,
        object: *const c_char,
        index: c_int,
        charset: *mut *mut FcCharSet,
    ) -> c_int;
    fn FcCharSetCreate() -> *mut FcCharSet;
    fn FcCharSetDestroy(charset: *mut FcCharSet);
    fn FcCharSetAddChar(charset: *mut FcCharSet, codepoint: u32) -> c_int;
    fn FcCharSetHasChar(charset: *const FcCharSet, codepoint: u32) -> c_int;
    fn FcConfigSubstitute(config: *mut FcConfig, pattern: *mut FcPattern, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(pattern: *mut FcPattern);
    fn FcFontMatch(
        config: *mut FcConfig,
        pattern: *mut FcPattern,
        result: *mut c_int,
    ) -> *mut FcPattern;
    fn FcFontSort(
        config: *mut FcConfig,
        pattern: *mut FcPattern,
        trim: c_int,
        charset: *mut *mut FcCharSet,
        result: *mut c_int,
    ) -> *mut FcFontSet;
    fn FcFontSetDestroy(font_set: *mut FcFontSet);
}

const MATCH_PATTERN: c_int = 0;
const RESULT_MATCH: c_int = 0;
const FAMILY: &[u8] = b"family\0";
const CHARSET: &[u8] = b"charset\0";
const FILE: &[u8] = b"file\0";
const FONTFORMAT: &[u8] = b"fontformat\0";

/// Whether fontdb can load a font of this fontconfig `fontformat`: ttf-parser
/// reads TrueType and OpenType/CFF outlines only. Bitmap and legacy formats
/// (PCF, BDF, Type 1, ...) are reported by fontconfig too, and may be the
/// only fonts covering a script (e.g. Hangul in the X11 `misc` fonts).
fn loadable_format(format: &[u8]) -> bool {
    matches!(format, b"TrueType" | b"CFF")
}

struct Config(*mut FcConfig);

impl Config {
    fn new() -> Option<Self> {
        let config = unsafe { FcInitLoadConfigAndFonts() };
        (!config.is_null()).then_some(Self(config))
    }

    fn match_family(&self, family: &str) -> Option<PathBuf> {
        let family = CString::new(family).ok()?;
        let pattern = unsafe { FcPatternCreate() };
        if pattern.is_null() {
            return None;
        }
        unsafe {
            FcPatternAddString(pattern, FAMILY.as_ptr().cast(), family.as_ptr().cast());
            prepare(self.0, pattern);
        }
        let mut result = 0;
        let matched = unsafe { FcFontMatch(self.0, pattern, &mut result) };
        let path = if result == RESULT_MATCH {
            pattern_file(matched)
        } else {
            None
        };
        unsafe {
            if !matched.is_null() {
                FcPatternDestroy(matched);
            }
            FcPatternDestroy(pattern);
        }
        path
    }

    /// Try loadable fonts in preference order until `chars` is covered.
    /// The callback removes characters only after the loaded font actually
    /// provides them. Returns characters no listed loadable font claims.
    fn visit_fallbacks(
        &self,
        chars: &mut HashSet<char>,
        mut try_font: impl FnMut(PathBuf, &mut HashSet<char>),
    ) -> HashSet<char> {
        let pattern = unsafe { FcPatternCreate() };
        let charset = unsafe { FcCharSetCreate() };
        if pattern.is_null() || charset.is_null() {
            unsafe {
                if !pattern.is_null() {
                    FcPatternDestroy(pattern)
                };
                if !charset.is_null() {
                    FcCharSetDestroy(charset)
                };
            }
            return HashSet::new();
        }
        unsafe {
            for &ch in chars.iter() {
                FcCharSetAddChar(charset, ch as u32);
            }
            FcPatternAddCharSet(pattern, CHARSET.as_ptr().cast(), charset);
            prepare(self.0, pattern);
        }
        let mut result = 0;
        let set = unsafe { FcFontSort(self.0, pattern, 0, std::ptr::null_mut(), &mut result) };
        // FcFontSort returns every installed font. Consult its cached
        // charsets before loading files, but only remove a character once
        // the callback has verified coverage in the loaded database.
        let mut claimed = HashSet::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        if !set.is_null() {
            let set_ref = unsafe { &*set };
            for index in 0..set_ref.nfont.max(0) as usize {
                if chars.is_empty() {
                    break;
                }
                let font = unsafe { *set_ref.fonts.add(index) };
                if !pattern_string(font, FONTFORMAT).is_some_and(|f| loadable_format(f.to_bytes()))
                {
                    continue;
                }
                let mut font_chars = std::ptr::null_mut();
                let found = unsafe {
                    FcPatternGetCharSet(font, CHARSET.as_ptr().cast(), 0, &mut font_chars)
                };
                if found != RESULT_MATCH || font_chars.is_null() {
                    continue;
                }
                // font_chars is owned by the pattern; it is not destroyed here
                let mut relevant = false;
                for &ch in chars.iter() {
                    if unsafe { FcCharSetHasChar(font_chars, ch as u32) } != 0 {
                        claimed.insert(ch);
                        relevant = true;
                    }
                }
                if relevant {
                    if let Some(path) = pattern_file(font).filter(|path| seen.insert(path.clone()))
                    {
                        try_font(path, chars);
                    }
                }
            }
        }
        let unavailable = chars.difference(&claimed).copied().collect();
        unsafe {
            if !set.is_null() {
                FcFontSetDestroy(set)
            };
            FcPatternDestroy(pattern);
            FcCharSetDestroy(charset);
        }
        unavailable
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        unsafe { FcConfigDestroy(self.0) };
    }
}

unsafe fn prepare(config: *mut FcConfig, pattern: *mut FcPattern) {
    FcConfigSubstitute(config, pattern, MATCH_PATTERN);
    FcDefaultSubstitute(pattern);
}

/// A string property of `pattern`, copied so it cannot outlive the pattern.
fn pattern_string(pattern: *mut FcPattern, object: &[u8]) -> Option<CString> {
    if pattern.is_null() {
        return None;
    }
    let mut value = std::ptr::null_mut();
    let result = unsafe { FcPatternGetString(pattern, object.as_ptr().cast(), 0, &mut value) };
    if result != RESULT_MATCH || value.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(value.cast()) }.to_owned())
}

fn pattern_file(pattern: *mut FcPattern) -> Option<PathBuf> {
    let value = pattern_string(pattern, FILE)?;
    let path = value.to_string_lossy();
    Some(PathBuf::from(path.as_ref()))
}

/// Codepoints that must not count as "needing font coverage": the Unicode
/// Default_Ignorable_Code_Point set plus other shaper-consumed format
/// controls. The shaper consumes them without ever looking up a glyph — the
/// emoji font implements variation selectors (U+FE0F and friends) through
/// variation-sequence tables instead of the base cmap, so almost no font
/// "maps" them. Requiring coverage anyway made the fallback machinery sort
/// every installed font to cover one invisible codepoint.
fn needs_coverage(ch: char) -> bool {
    if ch.is_control() || ch.is_whitespace() {
        return false;
    }
    !matches!(
        ch,
        '\u{00AD}'                      // soft hyphen
            | '\u{034F}'                // combining grapheme joiner
            | '\u{061C}'                // arabic letter mark
            | '\u{115F}'..='\u{1160}'   // hangul choseong/jungseong filler
            | '\u{17B4}'..='\u{17B5}'   // khmer vowel inherent
            | '\u{180B}'..='\u{180F}'   // mongolian free variation selectors
            | '\u{200B}'..='\u{200F}'   // zero-width spaces, joiners, dir marks
            | '\u{202A}'..='\u{202E}'   // bidi embedding controls
            | '\u{2060}'..='\u{206F}'   // invisible operators, bidi isolates
            | '\u{3164}'                // hangul filler
            | '\u{FE00}'..='\u{FE0F}'   // variation selectors
            | '\u{FEFF}'                // zero width no-break space
            | '\u{FFA0}'                // halfwidth hangul filler
            | '\u{FFF0}'..='\u{FFF8}'   // interlinear annotation
            | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
            | '\u{1D173}'..='\u{1D17A}' // musical format controls
            | '\u{E0000}'..='\u{E0FFF}' // tags, variation selectors supplement
    )
}

/// Build a small database containing configured fonts plus enough fontconfig
/// fallbacks to cover every renderable character in the current corpus.
pub(super) fn database_for(
    specs: &[FontSpec],
    required_chars: &HashSet<char>,
) -> Option<fontdb::Database> {
    let mut cache = FontCache::load();
    let mut db = fontdb::Database::new();
    let mut loaded = HashSet::new();
    let mut missing_families = Vec::new();

    for spec in specs {
        if let Some(path) = cache.family(&spec.family) {
            load_file(&mut db, &mut loaded, path);
        } else {
            missing_families.push(spec.family.clone());
        }
    }
    let mut uncovered: HashSet<char> = required_chars
        .iter()
        .copied()
        .filter(|ch| needs_coverage(*ch))
        .collect();
    remove_covered(&db, &mut uncovered);
    for path in cache.fallbacks.clone() {
        if uncovered.is_empty() {
            break;
        }
        load_file(&mut db, &mut loaded, path);
        remove_covered(&db, &mut uncovered);
    }

    uncovered.retain(|ch| !cache.missing.contains(ch));
    if !missing_families.is_empty() || !uncovered.is_empty() {
        let config = Config::new()?;
        for family in missing_families {
            let path = config.match_family(&family)?;
            load_file(&mut db, &mut loaded, path.clone());
            cache.set_family(family, path);
        }
        remove_covered(&db, &mut uncovered);
        let unavailable = config.visit_fallbacks(&mut uncovered, |path, uncovered| {
            let before = uncovered.len();
            if load_file(&mut db, &mut loaded, path.clone()) {
                remove_covered(&db, uncovered);
                if uncovered.len() < before {
                    cache.add_fallback(path);
                }
            }
        });
        for ch in unavailable {
            cache.add_missing(ch);
            uncovered.remove(&ch);
        }
        if !uncovered.is_empty() {
            return None;
        }
        cache.save();
    }
    (!db.is_empty()).then_some(db)
}

/// Extend an existing small database when interactive input introduces
/// characters that were not present in the startup corpus.
pub(super) fn add_fallbacks(db: &mut fontdb::Database, required_chars: &HashSet<char>) {
    let mut uncovered: HashSet<char> = required_chars
        .iter()
        .copied()
        .filter(|ch| needs_coverage(*ch))
        .collect();
    remove_covered(db, &mut uncovered);
    if uncovered.is_empty() {
        return;
    }

    let mut cache = FontCache::load();
    let mut loaded: HashSet<PathBuf> = db
        .faces()
        .filter_map(|face| match &face.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            fontdb::Source::Binary(_) => None,
        })
        .collect();
    // Like database_for: fallbacks that covered something before, and
    // codepoints known to be unavailable, are settled without starting
    // fontconfig, whose initialization walks every font directory.
    for path in cache.fallbacks.clone() {
        if uncovered.is_empty() {
            break;
        }
        if load_file(db, &mut loaded, path) {
            remove_covered(db, &mut uncovered);
        }
    }
    uncovered.retain(|ch| !cache.missing.contains(ch));
    if uncovered.is_empty() {
        return;
    }

    let Some(config) = Config::new() else { return };
    let unavailable = config.visit_fallbacks(&mut uncovered, |path, uncovered| {
        let before = uncovered.len();
        if load_file(db, &mut loaded, path.clone()) {
            remove_covered(db, uncovered);
            if uncovered.len() < before {
                cache.add_fallback(path);
            }
        }
    });
    for ch in unavailable {
        cache.add_missing(ch);
    }
    cache.save();
}

fn load_file(db: &mut fontdb::Database, loaded: &mut HashSet<PathBuf>, path: PathBuf) -> bool {
    loaded.insert(path.clone()) && db.load_font_file(path).is_ok()
}

fn remove_covered(db: &fontdb::Database, chars: &mut HashSet<char>) {
    for face in db.faces() {
        db.with_face_data(face.id, |data, index| {
            if let Ok(face) = ttf_parser::Face::parse(data, index) {
                chars.retain(|&ch| face.glyph_index(ch).is_none());
            }
        });
        if chars.is_empty() {
            break;
        }
    }
}

#[derive(Default)]
struct FontCache {
    path: Option<PathBuf>,
    families: Vec<(String, PathBuf)>,
    fallbacks: Vec<PathBuf>,
    missing: HashSet<char>,
    changed: bool,
}

impl FontCache {
    fn load() -> Self {
        let path = cache_path();
        let mut cache = Self {
            path: path.clone(),
            ..Self::default()
        };
        let Some(path) = path else { return cache };
        let Ok(contents) = std::fs::read_to_string(path) else {
            return cache;
        };
        let mut lines = contents.lines();
        let expected_header = format!("# instantmenu font cache v2 {}", fontconfig_stamp());
        if lines.next() != Some(expected_header.as_str()) {
            cache.changed = true;
            return cache;
        }
        for line in lines {
            let mut fields = line.splitn(3, '\t');
            match (fields.next(), fields.next(), fields.next()) {
                (Some("family"), Some(family), Some(path)) if Path::new(path).is_file() => {
                    cache
                        .families
                        .push((family.to_string(), PathBuf::from(path)));
                }
                (Some("fallback"), Some(path), None) if Path::new(path).is_file() => {
                    cache.fallbacks.push(PathBuf::from(path));
                }
                (Some("missing"), Some(codepoint), None) => {
                    if let Ok(codepoint) = u32::from_str_radix(codepoint, 16) {
                        if let Some(ch) = char::from_u32(codepoint) {
                            cache.missing.insert(ch);
                        }
                    }
                }
                _ => {}
            }
        }
        cache
    }

    fn family(&self, family: &str) -> Option<PathBuf> {
        self.families
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
            .map(|(_, path)| path.clone())
    }

    fn set_family(&mut self, family: String, path: PathBuf) {
        self.families
            .retain(|(name, _)| !name.eq_ignore_ascii_case(&family));
        self.families.push((family, path));
        self.changed = true;
    }

    fn add_fallback(&mut self, path: PathBuf) {
        if !self.fallbacks.contains(&path) {
            self.fallbacks.push(path);
            self.changed = true;
        }
    }

    fn add_missing(&mut self, ch: char) {
        if self.missing.insert(ch) {
            self.changed = true;
        }
    }

    fn save(&self) {
        if !self.changed {
            return;
        }
        let Some(path) = &self.path else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let mut contents = format!("# instantmenu font cache v2 {}\n", fontconfig_stamp());
        for (family, path) in &self.families {
            contents.push_str(&format!("family\t{family}\t{}\n", path.display()));
        }
        for path in &self.fallbacks {
            contents.push_str(&format!("fallback\t{}\n", path.display()));
        }
        for ch in &self.missing {
            contents.push_str(&format!("missing\t{:x}\n", *ch as u32));
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        if std::fs::write(&temporary, contents).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|root| root.join("instantmenu/font-paths-v2"))
}

fn fontconfig_stamp() -> u128 {
    let mut roots = vec![PathBuf::from("/var/cache/fontconfig")];
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        roots.push(PathBuf::from(path).join("fontconfig"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".cache/fontconfig"));
    }
    roots
        .into_iter()
        .flat_map(|root| {
            std::iter::once(root.clone()).chain(
                std::fs::read_dir(root)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path()),
            )
        })
        .filter_map(|path| std::fs::metadata(path).ok()?.modified().ok())
        .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{loadable_format, needs_coverage, Config};
    use std::collections::HashSet;

    /// Only formats ttf-parser can read may be offered as fallbacks.
    #[test]
    fn only_outline_formats_are_loadable() {
        assert!(loadable_format(b"TrueType"));
        assert!(loadable_format(b"CFF"));
        for format in [&b"PCF"[..], b"BDF", b"Type 1", b"PFR", b""] {
            assert!(!loadable_format(format), "{format:?}");
        }
    }

    /// A codepoint no loadable font covers is reported unavailable, without
    /// loading every installed font.
    #[test]
    fn uncoverable_codepoints_yield_no_fallbacks() {
        let Some(config) = Config::new() else { return };
        let mut chars: HashSet<char> = ['\u{FDD0}'].into(); // noncharacter
        let missing = config.visit_fallbacks(&mut chars, |_, _| panic!("no candidate expected"));
        assert_eq!(missing, chars);
    }

    /// Stop once a font actually supplies every requested character.
    #[test]
    fn fallbacks_stop_once_everything_is_covered() {
        let Some(config) = Config::new() else { return };
        let mut chars: HashSet<char> = ['a', '\u{FDD0}'].into();
        let mut attempts = 0;
        let missing = config.visit_fallbacks(&mut chars, |_, chars| {
            attempts += 1;
            chars.remove(&'a');
        });
        assert_eq!(chars, ['\u{FDD0}'].into());
        assert_eq!(missing, chars);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn a_claimed_but_unloaded_character_stays_unresolved() {
        let Some(config) = Config::new() else { return };
        let mut chars: HashSet<char> = ['a'].into();
        let mut attempts = 0;
        let missing = config.visit_fallbacks(&mut chars, |_, _| attempts += 1);
        assert!(attempts > 0);
        assert!(missing.is_empty());
        assert_eq!(chars, ['a'].into());
    }

    /// Emoji selectors and joiners are consumed by the shaper and must not
    /// trigger the font-fallback machinery (the U+FE0F regression: the emoji
    /// picker paid a full-system fontconfig sort for one invisible selector).
    #[test]
    fn shaper_consumed_codepoints_need_no_coverage() {
        for ch in [
            '\u{FE0F}',  // emoji presentation selector (❤️)
            '\u{FE0E}',  // text presentation selector
            '\u{200D}',  // zero width joiner (👨‍👩‍👧‍👦)
            '\u{200C}',  // zero width non-joiner
            '\u{200E}',  // left-to-right mark
            '\u{E0067}', // England flag tag character
            '\u{180B}',  // mongolian variation selector
            '\u{00AD}',  // soft hyphen
            '\u{2060}',  // word joiner
        ] {
            assert!(!needs_coverage(ch), "U+{:04X} needs no coverage", ch as u32);
        }
    }

    /// Visible characters keep requiring coverage — including combining
    /// marks, which are shaper-attached but still need their glyph.
    #[test]
    fn visible_codepoints_still_need_coverage() {
        for ch in ['a', 'ß', '😀', '\u{0301}', '\u{20E3}', '#'] {
            assert!(needs_coverage(ch), "U+{:04X} needs coverage", ch as u32);
        }
    }

    /// Control characters and whitespace are excluded as before.
    #[test]
    fn control_and_whitespace_need_no_coverage() {
        for ch in ['\n', '\t', '\u{0}', ' '] {
            assert!(!needs_coverage(ch), "U+{:04X} needs no coverage", ch as u32);
        }
    }
}
