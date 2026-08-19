//! Port of `config.def.h` — default settings; can be overridden by command
//! line and X resources.

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

#[derive(Debug, Clone)]
pub struct Config {
    /* --position option; anchor corner/edge/center on screen */
    pub position: Position,
    /* --follow-cursor option; place menu at mouse position */
    pub follow_cursor: bool,
    /* minimum width when the menu is sized to its content
    (follow-cursor, embedded); not used for the monitor `center` popup */
    pub min_width: i32,

    pub instant: bool,
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
    pub line_height: i32,

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

    /* --toast option; toast mode that times out after a while (tenths of seconds) */
    pub toast: i32,
    /* --input-only option; input only */
    pub input_only: bool,
    /* --password option; display input as dots */
    pub password: bool,
    /* --no-grab option; don't grab the keyboard */
    pub no_grab: bool,
    /* --alt-tab option; alt-tab behaviour */
    pub alt_tab: bool,
    /* --managed option; display as managed wm window */
    pub managed: bool,
    /* -r option; reject input that results in no match */
    pub reject_no_match: bool,
    /* --commented option; instantASSIST mode */
    pub commented: bool,
    /* -m option; monitor index, -1 = auto */
    pub monitor: i32,
    /* -x option: horizontal nudge from the anchor */
    pub x_offset: i32,
    /* -y option: vertical nudge from the anchor */
    pub y_offset: i32,
    /* -w option: make instantmenu this wide */
    pub width: i32,
    /* --embed option; embedding window id */
    pub embed: Option<u32>,
    /* -f; grab keyboard before reading stdin */
    pub fast: bool,
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
            instant: false,
            space_confirm: false,
            match_mode: MatchMode::Fuzzy,
            pre_match: false,
            smart_case: false,
            insensitive: false,
            animated: false,
            frame_count: 7,
            full_height: false,
            line_height: 0,
            fonts: vec![
                "Inter-Regular:size=12".to_string(),
                "Fira Code Nerd Font:size=14".to_string(),
                "JoyPixels:pixelsize=20:antialias=true:autohint=true".to_string(),
            ],
            prompt: None,
            placeholder: None,
            left_command: None,
            right_command: None,
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
            toast: 0,
            input_only: false,
            password: false,
            no_grab: false,
            alt_tab: false,
            managed: false,
            reject_no_match: false,
            commented: false,
            monitor: -1,
            x_offset: 0,
            y_offset: 0,
            width: 0,
            embed: None,
            fast: false,
        }
    }
}
