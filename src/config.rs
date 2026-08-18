//! Port of `config.def.h` — default settings; can be overridden by command
//! line and X resources.

use crate::enums::Scheme;

pub const VERSION: &str = "4.9";

/// `xresname`
pub const XRES_NAME: &str = "instantmenu";

/// X resource color type names, port of `xrescolortype`.
pub const XRES_COLOR_TYPES: [&str; 3] = ["fg", "bg", "detail"];

#[derive(Debug, Clone)]
pub struct Config {
    /* -b option; if 0, instantmenu appears at bottom */
    pub topbar: bool,
    /* -c option; centers dmenu on screen */
    pub centered: bool,
    /* -C option; place menu at mouse position */
    pub followcursor: bool,
    /* minimum width when centered */
    pub min_width: i32,

    pub instant: bool,
    pub spaceconfirm: bool,
    pub fuzzy: bool,
    pub prematch: bool,
    pub smartcase: bool,
    pub exact: bool,
    pub sely: i32,
    pub animated: bool,
    pub framecount: i32,
    pub fullheight: bool,
    /* -h option; minimum height of a menu line */
    pub lineheight: i32,

    /* -fn option overrides fonts[0]; default font set */
    pub fonts: Vec<String>,

    /* -p option; prompt to the left of input field */
    pub prompt: Option<String>,
    /* -q option; placeholder inside the input field */
    pub searchtext: Option<String>,
    /* -lc option; command run on shift+left / left arrow cell */
    pub leftcmd: Option<String>,
    /* -rc option; command run on shift+right */
    pub rightcmd: Option<String>,
    /*        fg         bg     detail  */
    pub colors: [[String; 3]; 9],

    /* -l option; if nonzero, vertical list with given number of lines */
    pub lines: i32,
    /* -g option; columns in grid if nonzero and lines is nonzero */
    pub columns: i32,

    /* Characters not considered part of a word while deleting words */
    pub worddelimiters: String,

    /* -ps option; preselected item starting from 0 */
    pub preselected: i32,

    /* Size of the window border */
    pub border_width: i32,

    /* ---- runtime options (set from argv, globals in instantmenu.c) ---- */

    /* -T option; toast mode that times out after a while (tenth of seconds) */
    pub toast: i32,
    /* -I option; input only */
    pub inputonly: bool,
    /* -P option; display input as dots */
    pub passwd: bool,
    /* -G option; don't grab the keyboard */
    pub nograb: bool,
    /* -A option; alt-tab behaviour */
    pub alttab: bool,
    /* -wm option; display as managed wm window */
    pub managed: bool,
    /* -r option; reject input that results in no match */
    pub rejectnomatch: bool,
    /* -ct option; instantASSIST mode */
    pub commented: bool,
    /* -m option; monitor index, -1 = auto */
    pub mon: i32,
    /* -x option: window x offset */
    pub dmx: i32,
    /* -xr option: x offset counted from the right */
    pub rightxoffset: bool,
    /* -y option: window y offset */
    pub dmy: i32,
    /* -w option: make instantmenu this wide */
    pub dmw: i32,
    /* -W option; embedding window id */
    pub embed: Option<u32>,
    /* -f; grab keyboard before reading stdin */
    pub fast: bool,
}

impl Default for Config {
    fn default() -> Self {
        let scheme = |fg: &str, bg: &str, detail: &str| {
            [fg.to_string(), bg.to_string(), detail.to_string()]
        };
        Config {
            topbar: true,
            centered: false,
            followcursor: false,
            min_width: 500,
            instant: false,
            spaceconfirm: false,
            fuzzy: true,
            prematch: false,
            smartcase: false,
            exact: false,
            sely: 0,
            animated: false,
            framecount: 7,
            fullheight: false,
            lineheight: 0,
            fonts: vec![
                "Inter-Regular:size=12".to_string(),
                "Fira Code Nerd Font:size=14".to_string(),
                "JoyPixels:pixelsize=20:antialias=true:autohint=true".to_string(),
            ],
            prompt: None,
            searchtext: None,
            leftcmd: None,
            rightcmd: None,
            colors: [
                scheme("#DFDFDF", "#121212", "#3E485B"), // Norm
                scheme("#575E70", "#121212", "#3E485B"), // Fade
                scheme("#DFDFDF", "#384252", "#272727"), // Highlight
                scheme("#DFDFDF", "#272727", "#2E2E2E"), // Hover
                scheme("#000000", "#8AB4F8", "#536DFE"), // Sel
                scheme("#000000", "#3579CA", "#3579CA"), // Out
                scheme("#000000", "#81c995", "#1e8e3e"), // Green
                scheme("#000000", "#f28b82", "#d93025"), // Red
                scheme("#000000", "#fdd663", "#f9ab00"), // Yellow
            ],
            lines: 0,
            columns: 1,
            worddelimiters: " ".to_string(),
            preselected: 0,
            border_width: 0,
            toast: 0,
            inputonly: false,
            passwd: false,
            nograb: false,
            alttab: false,
            managed: false,
            rejectnomatch: false,
            commented: false,
            mon: -1,
            dmx: 0,
            rightxoffset: false,
            dmy: 0,
            dmw: 0,
            embed: None,
            fast: false,
        }
    }
}

impl Config {
    /// Apply a scheme color override (command line `-nb`/`-nf`/... or X
    /// resources), port of the `colortemp` handling.
    pub fn set_color(&mut self, scheme: Scheme, col: usize, value: &str) {
        self.colors[scheme as usize][col] = value.to_string();
    }

    pub fn scheme_color(&self, scheme: Scheme, col: usize) -> &str {
        &self.colors[scheme as usize][col]
    }
}
