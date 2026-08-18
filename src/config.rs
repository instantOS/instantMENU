//! Port of `config.def.h` — default settings; can be overridden by command
//! line and X resources.

use clap::ValueEnum;
use crate::render::SchemeStrings;

pub const VERSION: &str = "4.9";

/// `xresname`
pub const XRES_NAME: &str = "instantmenu";

/// Where the menu appears on screen (`--position`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Position {
    Top,
    Bottom,
    Centered,
}

/// Item matching algorithm (`--match-mode`).
///
/// `Dmenu` is the classic dmenu matcher: every whitespace-separated word of
/// the input must appear contiguously in the item; exact, prefix and
/// substring matches are ranked in that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MatchMode {
    Fuzzy,
    Dmenu,
    Exact,
}

#[derive(Debug, Clone)]
pub struct Config {
    /* --position option; top (default), bottom, or centered on screen */
    pub position: Position,
    /* -C option; place menu at mouse position */
    pub follow_cursor: bool,
    /* minimum width when centered */
    pub min_width: i32,

    pub instant: bool,
    /* --match-mode option; item matching algorithm */
    pub match_mode: MatchMode,
    pub pre_match: bool,
    pub smart_case: bool,
    /* -i option; case-insensitive item matching */
    pub insensitive: bool,
    pub animated: bool,
    pub frame_count: i32,
    pub full_height: bool,
    /* -h option; minimum height of a menu line */
    pub line_height: i32,

    /* -fn option overrides fonts[0]; default font set */
    pub fonts: Vec<String>,

    /* -p option; prompt to the left of input field */
    pub prompt: Option<String>,
    /* -q option; placeholder inside the input field */
    pub search_text: Option<String>,
    /* -lc option; command run on shift+left / left arrow cell */
    pub left_command: Option<String>,
    /* -rc option; command run on shift+right */
    pub right_command: Option<String>,
    /* fg / bg / detail per scheme */
    pub colors: [SchemeStrings; 9],

    /* -l option; if nonzero, vertical list with given number of lines */
    pub lines: i32,
    /* -g option; columns in grid if nonzero and lines is nonzero */
    pub columns: i32,

    /* Characters not considered part of a word while deleting words */
    pub word_delimiters: String,

    /* -ps option; preselected item starting from 0 */
    pub preselected: i32,

    /* Size of the window border */
    pub border_width: i32,

    /* ---- runtime options (set from argv, globals in instantmenu.c) ---- */

    /* -T option; toast mode that times out after a while (tenth of seconds) */
    pub toast: i32,
    /* -I option; input only */
    pub input_only: bool,
    /* -P option; display input as dots */
    pub password: bool,
    /* -G option; don't grab the keyboard */
    pub no_grab: bool,
    /* -A option; alt-tab behaviour */
    pub alt_tab: bool,
    /* -wm option; display as managed wm window */
    pub managed: bool,
    /* -r option; reject input that results in no match */
    pub reject_no_match: bool,
    /* -ct option; instantASSIST mode */
    pub commented: bool,
    /* -m option; monitor index, -1 = auto */
    pub monitor: i32,
    /* -x option: window x offset */
    pub x_offset: i32,
    /* -xr option: x offset counted from the right */
    pub right_x_offset: bool,
    /* -y option: window y offset */
    pub y_offset: i32,
    /* -w option: make instantmenu this wide */
    pub width: i32,
    /* -W option; embedding window id */
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
            search_text: None,
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
            right_x_offset: false,
            y_offset: 0,
            width: 0,
            embed: None,
            fast: false,
        }
    }
}
