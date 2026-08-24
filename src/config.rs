//! Port of `config.def.h` — default settings; can be overridden by command
//! line and X resources.

use std::path::PathBuf;

use crate::render::SchemeStrings;
use clap::ValueEnum;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `xresname`
pub const XRES_NAME: &str = "instantmenu";

/// Where the menu appears on screen (`--position`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Position {
    /// Top-left corner.
    TopLeft,
    /// Top edge, centered horizontally (the default).
    Top,
    /// Top-right corner.
    TopRight,
    /// Left edge, centered vertically.
    Left,
    /// Centered on the screen.
    Center,
    /// Right edge, centered vertically.
    Right,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom edge, centered horizontally.
    Bottom,
    /// Bottom-right corner.
    BottomRight,
}

/// Window width (`--width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// Not set: each placement mode picks its own default width.
    Default,
    /// A fixed width in pixels.
    Fixed(i32),
    /// Fit the content: measure the items and size the menu to match.
    Auto,
}

/// Minimum height of one menu line (`--line-height`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineHeight {
    /// No minimum beyond the font height plus padding.
    Default,
    /// Derived from the font (2.5x the font height).
    FromFont,
    /// At least this many pixels.
    Pixels(i32),
}

impl LineHeight {
    /// The pixel value (0 = no minimum). [`LineHeight::FromFont`] counts as
    /// unresolved here; main() resolves it into [`LineHeight::Pixels`]
    /// before the menu is constructed.
    pub fn pixels(self) -> i32 {
        match self {
            LineHeight::Default | LineHeight::FromFont => 0,
            LineHeight::Pixels(n) => n,
        }
    }
}

/// Monitor selection (`--monitor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorChoice {
    /// Follow keyboard focus, then use the first monitor.
    Auto,
    /// A specific monitor index (0-based).
    Index(u32),
}

/// Item matching algorithm (`--match-mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MatchMode {
    /// Fuzzy matching (the default): ranks by how tightly the pattern fits.
    Fuzzy,
    /// The classic dmenu matcher: every token must appear; exact, prefix,
    /// then substring matches are ranked in that order.
    Dmenu,
    /// Only exact matches.
    Exact,
}

/// Slide settings as given on the command line (`instantmenu slide`), before
/// defaults are applied. `Config::slide` being `Some` is what puts the menu
/// in slide mode; `resolve` fills in the defaults and rejects
/// empty/inverted ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideSettings {
    /// Minimum value (`--min`, default 0).
    pub min: i32,
    /// Maximum value (`--max`, default 100).
    pub max: i32,
    /// Initial value (`--value`, default: the middle of the range).
    pub value: Option<i32>,
    /// Small step for left/right (`--step`, default 1).
    pub step: Option<i32>,
    /// Large step for up/down (`--big-step`, default max(range/10, 5)).
    pub big_step: Option<i32>,
    /// Command run on every value change (`--command`), the value appended
    /// as its last argument.
    pub command: Option<String>,
}

impl Default for SlideSettings {
    /// The command line defaults: a 0..=100 slider.
    fn default() -> Self {
        SlideSettings {
            min: 0,
            max: 100,
            value: None,
            step: None,
            big_step: None,
            command: None,
        }
    }
}

impl SlideSettings {
    /// The step after defaults: at least 1.
    pub fn resolved_step(&self) -> i32 {
        self.step.unwrap_or(1).max(1)
    }

    /// The large step after defaults: at least the small step, at least
    /// a tenth of the range (min 5).
    pub fn resolved_big_step(&self) -> i32 {
        self.big_step
            .unwrap_or((self.max - self.min) / 10)
            .max(5)
            .max(self.resolved_step())
    }

    /// The initial value after defaults: the middle of the range, clamped
    /// into it.
    pub fn resolved_value(&self) -> i32 {
        self.value
            .unwrap_or(self.min + (self.max - self.min) / 2)
            .clamp(self.min, self.max)
    }

    /// Validate the range and apply the defaults in place. `Err` carries a
    /// ready-to-print message.
    pub fn resolve(&mut self) -> Result<(), String> {
        if self.min >= self.max {
            return Err(format!(
                "slide: minimum ({}) must be less than maximum ({})",
                self.min, self.max
            ));
        }
        self.value = Some(self.resolved_value());
        self.step = Some(self.resolved_step());
        self.big_step = Some(self.resolved_big_step());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /* --position option; anchor corner/edge/center on screen */
    pub position: Position,
    /* --follow-cursor option; place menu at mouse position */
    pub follow_cursor: bool,
    /* minimum width when the menu is sized to its content
    (follow-cursor, embedded); not used for the monitor `center` popup */
    pub min_width: i32,

    /* --auto-confirm option; auto-confirm when exactly one item matches */
    pub auto_confirm: bool,
    /* --space-confirm option; confirm selection with the space key */
    pub space_confirm: bool,
    /* --match-mode option; item matching algorithm */
    pub match_mode: MatchMode,
    pub pre_match: bool,
    pub smart_case: bool,
    /* -i option; case-insensitive item matching */
    pub insensitive: bool,
    pub animated: bool,
    pub frame_count: i32,
    pub full_height: bool,
    /* --line-height option; minimum height of a menu line */
    pub line_height: LineHeight,

    /* --font option overrides fonts[0]; default font set */
    pub fonts: Vec<String>,

    /* -p option; prompt to the left of input field */
    pub prompt: Option<String>,
    /* --placeholder option; placeholder inside the input field */
    pub placeholder: Option<String>,
    /* --left-command option; command run on shift+left / left arrow cell */
    pub left_command: Option<String>,
    /* --right-command option; command run on shift+right */
    pub right_command: Option<String>,
    /* --frecency-cache option; file backing the selection frecency store */
    pub frecency_cache: Option<PathBuf>,
    /* fg / bg / detail per scheme */
    pub colors: [SchemeStrings; 9],

    /* -l option; if nonzero, vertical list with given number of lines */
    pub lines: i32,
    /* -g option; columns in grid if nonzero and lines is nonzero */
    pub columns: i32,

    /* Characters not considered part of a word while deleting words */
    pub word_delimiters: String,

    /* --preselect option; preselected item starting from 0 */
    pub preselected: i32,

    /* Size of the window border */
    pub border_width: i32,

    /* ---- runtime options (set from argv, globals in instantmenu.c) ---- */

    /* --toast option; toast mode that times out after a while (seconds) */
    pub toast: Option<f32>,
    /* --input-only option; input only */
    pub input_only: bool,
    /* --password option; display input as dots */
    pub password: bool,
    /* --no-grab option; don't grab the keyboard */
    pub no_grab: bool,
    /* --no-outside-close option; don't close on a click outside the menu */
    pub outside_close: bool,
    /* --alt-tab option; alt-tab behaviour */
    pub alt_tab: bool,
    /* --managed option; display as managed wm window */
    pub managed: bool,
    /* -r option; reject input that results in no match */
    pub reject_no_match: bool,
    /* --commented option; instantASSIST mode */
    pub commented: bool,
    /* -m option; monitor index or auto (focus, then first monitor) */
    pub monitor: MonitorChoice,
    /* -x option: horizontal nudge from the anchor */
    pub x_offset: i32,
    /* -y option: vertical nudge from the anchor */
    pub y_offset: i32,
    /* -w option: make instantmenu this wide */
    pub width: Width,
    /* --embed option; embedding window id */
    pub embed: Option<u32>,
    /* `slide` subcommand; Some(_) = slide mode with these settings */
    pub slide: Option<SlideSettings>,
}

impl Default for Config {
    fn default() -> Self {
        let scheme = |fg: &str, bg: &str, detail: &str| SchemeStrings {
            fg: fg.to_string(),
            bg: bg.to_string(),
            detail: detail.to_string(),
        };
        Config {
            position: Position::Top,
            follow_cursor: false,
            min_width: 500,
            auto_confirm: false,
            space_confirm: false,
            match_mode: MatchMode::Fuzzy,
            pre_match: false,
            smart_case: false,
            insensitive: false,
            animated: false,
            frame_count: 7,
            full_height: false,
            line_height: LineHeight::Default,
            fonts: vec![
                "Inter-Regular:size=12".to_string(),
                "Fira Code Nerd Font:size=14".to_string(),
                "JoyPixels:pixelsize=20:antialias=true:autohint=true".to_string(),
            ],
            prompt: None,
            placeholder: None,
            left_command: None,
            right_command: None,
            frecency_cache: None,
            colors: [
                scheme("#DFDFDF", "#121212", "#3E485B"), // Norm
                scheme("#575E70", "#121212", "#3E485B"), // Fade
                scheme("#DFDFDF", "#384252", "#272727"), // Highlight
                scheme("#DFDFDF", "#272727", "#2E2E2E"), // Hover
                scheme("#000000", "#8AB4F8", "#536DFE"), // Sel
                scheme("#000000", "#3579CA", "#3579CA"), // Out
                scheme("#000000", "#81c995", "#1e8e3e"), // Green
                scheme("#000000", "#fdd663", "#f9ab00"), // Yellow
                scheme("#000000", "#f28b82", "#d93025"), // Red
            ],
            lines: 0,
            columns: 1,
            word_delimiters: " ".to_string(),
            preselected: 0,
            border_width: 0,
            toast: None,
            input_only: false,
            password: false,
            no_grab: false,
            outside_close: true,
            alt_tab: false,
            managed: false,
            reject_no_match: false,
            commented: false,
            monitor: MonitorChoice::Auto,
            x_offset: 0,
            y_offset: 0,
            width: Width::Default,
            embed: None,
            slide: None,
        }
    }
}
