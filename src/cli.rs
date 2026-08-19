//! Command line: clap with long options as the canonical surface and
//! single-letter shorts for the common flags.
//!
//! Unlike the C `atoi`, numeric options parse strictly: a malformed number
//! is an error, not silently 0. Options that take a negative value
//! (`--width -1`, `--line-height -1`, ...) accept hyphen-prefixed values.

use clap::Parser;

use crate::backend::BackendChoice;
use crate::config::{MatchMode, Position};

/// Long-form description shown by `--help` and the generated man page.
const LONG_ABOUT: &str = concat!(
    "instantmenu reads a newline-separated list of items from stdin and ",
    "displays them in a menu. Selecting an item and pressing Return prints ",
    "it to stdout and exits; typing narrows the list to matching items.\n\n",
    "With --slide it shows a value slider instead: Return prints the value ",
    "and exits, every change runs --command with the value appended.",
);

/// Keyboard bindings shown by `--help` and the generated man page.
const KEY_BINDINGS: &str = concat!(
    "KEYBOARD CONTROL:\n",
    "  Tab          copy the selected item to the input field\n",
    "  Return       confirm the selection and exit\n",
    "  Ctrl-Return  confirm the selection and keep running\n",
    "  Shift-Return confirm the input text and exit\n",
    "  Escape       exit without selecting an item\n",
    "  Ctrl-Left    move the cursor to the start of the current word\n",
    "  Ctrl-Right   move the cursor to the end of the current word\n",
    "  C-a Home  C-b Left  C-c Escape  C-d Delete  C-e End  C-f Right\n",
    "  C-g Escape  C-h Backspace  C-i Tab  C-j Return  C-k delete right\n",
    "  C-m Return  C-n Down  C-p Up  C-u delete left  C-w delete word\n",
    "  C-y paste primary  C-Y paste clipboard\n",
    "  M-b word start  M-f word end  M-g Home  M-G End  M-h Up\n",
    "  M-j page down  M-k page up  M-l Down  M-F4 quit\n",
    "SLIDE MODE (--slide):\n",
    "  Left h       decrease by --step        Right l    increase by --step\n",
    "  Down j       decrease by --big-step    Up k       increase by --big-step\n",
    "  plus/minus   change by 1               1..9, 0    jump to a ninth of the range\n",
    "  Home         minimum value             End        maximum value\n",
    "  Return       print the value and exit  Escape q   exit without printing\n",
    "  click/drag   set the value at the pointer         wheel  step\n",
    "  middle click reset to the initial value\n",
);

#[derive(Parser, Debug)]
#[command(
    name = "instantmenu",
    about = "A dynamic menu for X11 and Wayland (instantMENU, Rust port)",
    long_about = LONG_ABOUT,
    version = crate::config::VERSION,
    after_long_help = KEY_BINDINGS,
)]
pub struct Args {
    /// Backend to use: auto, x11 or wayland.
    #[arg(long, value_enum, value_name = "BACKEND", default_value = "auto")]
    pub backend: BackendChoice,

    /// Anchor the menu to a corner, edge or the center of the screen.
    #[arg(
        long,
        value_enum,
        value_name = "POSITION",
        conflicts_with = "follow_cursor"
    )]
    pub position: Option<Position>,

    /// Reject input if it results in no matches.
    #[arg(long, short = 'r')]
    pub reject_no_match: bool,

    /// Grab the keyboard before reading stdin.
    ///
    /// Only done when stdin is not a tty. Faster, but locks up X until
    /// stdin reaches end-of-file.
    #[arg(long, short = 'f')]
    pub fast: bool,

    /// Toast mode that times out after a while (tenths of seconds).
    ///
    /// The menu draws itself, waits the given time, then exits without a
    /// selection.
    #[arg(
        long,
        value_name = "TENTHS",
        allow_hyphen_values = true,
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub toast: Option<i32>,

    /// Activate instantASSIST mode (single-letter launcher).
    #[arg(long)]
    pub commented: bool,

    /// Slide mode: show a value slider instead of a menu.
    ///
    /// Return prints the current value to stdout and exits; Escape exits
    /// without printing. Every value change runs --command (if given) with
    /// the value appended as its last argument.
    #[arg(
        long,
        conflicts_with_all = [
            "toast",
            "commented",
            "input_only",
            "password",
            "preselect"
        ]
    )]
    pub slide: bool,

    /// Minimum slider value (slide mode).
    #[arg(
        long,
        requires = "slide",
        value_name = "N",
        allow_hyphen_values = true,
        default_value_t = 0
    )]
    pub min: i32,

    /// Maximum slider value (slide mode).
    #[arg(
        long,
        requires = "slide",
        value_name = "N",
        allow_hyphen_values = true,
        default_value_t = 100
    )]
    pub max: i32,

    /// Initial slider value (slide mode).
    ///
    /// Defaults to the middle of the range; clamped into it.
    #[arg(long, requires = "slide", value_name = "N", allow_hyphen_values = true)]
    pub value: Option<i32>,

    /// Small step for left/right (slide mode).
    #[arg(
        long,
        requires = "slide",
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(1..)
    )]
    pub step: Option<i32>,

    /// Large step for up/down (slide mode).
    ///
    /// Defaults to a tenth of the range (at least 5).
    #[arg(
        long,
        requires = "slide",
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(1..)
    )]
    pub big_step: Option<i32>,

    /// Command run on every value change (slide mode).
    ///
    /// Run through the shell with the current value appended as the last
    /// argument.
    #[arg(long, requires = "slide", value_name = "CMD")]
    pub command: Option<String>,

    /// Place the menu at the mouse position.
    #[arg(long, conflicts_with = "position")]
    pub follow_cursor: bool,

    /// Only display the input field, without the item list.
    #[arg(long)]
    pub input_only: bool,

    /// Enable smart case matching.
    ///
    /// The pattern is matched case-insensitively unless it contains an
    /// uppercase letter.
    #[arg(long, short = 's')]
    pub smart_case: bool,

    /// Item matching algorithm: fuzzy, dmenu or exact.
    #[arg(long, value_enum, value_name = "MODE")]
    pub match_mode: Option<MatchMode>,

    /// Enable pre matching.
    #[arg(long)]
    pub pre_match: bool,

    /// Confirm the selection with the space key.
    #[arg(long)]
    pub space_confirm: bool,

    /// Make instantmenu take the full screen height.
    #[arg(long)]
    pub full_height: bool,

    /// Case-insensitive item matching.
    #[arg(long, short = 'i')]
    pub insensitive: bool,

    /// Instantly select the only match.
    #[arg(long, short = 'n')]
    pub instant: bool,

    /// Display input as dots.
    #[arg(long)]
    pub password: bool,

    /// Use a monospace font (Fira Code Nerd Font:pixelsize=15).
    #[arg(long, conflicts_with = "font")]
    pub monospace: bool,

    /// Don't grab the keyboard.
    #[arg(long)]
    pub no_grab: bool,

    /// Alt-tab behaviour.
    #[arg(long)]
    pub alt_tab: bool,

    /// Let instantmenu be managed by the window manager as a normal window.
    #[arg(long)]
    pub managed: bool,

    /// Execute this command on shift + right arrow.
    #[arg(long, value_name = "CMD")]
    pub right_command: Option<String>,

    /// Run this command on shift + left and add a click target left of the
    /// input field.
    #[arg(long, value_name = "CMD")]
    pub left_command: Option<String>,

    /// Number of columns in grid mode (0 means 1).
    ///
    /// Implies one line per row unless --lines is given.
    #[arg(long, short = 'g', value_name = "N", value_parser = clap::value_parser!(i32).range(0..))]
    pub columns: Option<i32>,

    /// Number of lines in a vertical list.
    #[arg(long, short = 'l', value_name = "N", value_parser = clap::value_parser!(i32).range(0..))]
    pub lines: Option<i32>,

    /// Horizontal offset from the anchor position.
    ///
    /// Positive moves right, negative moves left.
    #[arg(long, short = 'x', value_name = "N", allow_hyphen_values = true)]
    pub x_offset: Option<i32>,

    /// Vertical offset from the anchor position.
    ///
    /// Positive moves down, negative moves up.
    #[arg(long, short = 'y', value_name = "N", allow_hyphen_values = true)]
    pub y_offset: Option<i32>,

    /// Make instantmenu this wide.
    ///
    /// A negative value adjusts the width to the longest line read from
    /// stdin.
    #[arg(long, short = 'w', value_name = "N", allow_hyphen_values = true)]
    pub width: Option<i32>,

    /// Select monitor by index.
    ///
    /// Monitor numbers start from 0. Use -1 for automatic selection.
    #[arg(
        long,
        short = 'm',
        value_name = "N",
        allow_hyphen_values = true,
        value_parser = clap::value_parser!(i32).range(-1..)
    )]
    pub monitor: Option<i32>,

    /// Prompt added to the left of the input field.
    #[arg(long, short = 'p', value_name = "TEXT")]
    pub prompt: Option<String>,

    /// Placeholder inside the input field.
    #[arg(long, value_name = "TEXT")]
    pub placeholder: Option<String>,

    /// Font or font set (overrides the X resource and default).
    #[arg(long, value_name = "FONT", conflicts_with = "monospace")]
    pub font: Option<String>,

    /// Minimum height of one menu line.
    ///
    /// At least 8 pixels.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub line_height: Option<i32>,

    /// Animation duration in frames.
    #[arg(
        long,
        short = 'a',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub animation: Option<i32>,

    /// Normal background color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR")]
    pub normal_bg: Option<String>,

    /// Normal foreground color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR")]
    pub normal_fg: Option<String>,

    /// Selected background color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR")]
    pub selected_bg: Option<String>,

    /// Selected foreground color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR")]
    pub selected_fg: Option<String>,

    /// Embedding window id (X11 only).
    #[arg(long, value_name = "ID", value_parser = parse_window_id)]
    pub embed: Option<u32>,

    /// Border width.
    ///
    /// Adds a border around the menu.
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub border_width: Option<i32>,

    /// Preselected item index.
    ///
    /// Starts from 0.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub preselect: Option<i32>,

    /// Initial input text.
    #[arg(long, value_name = "TEXT")]
    pub initial_text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_choice_parses() {
        use crate::backend::BackendChoice;

        let a = Args::try_parse_from(["instantmenu"]).unwrap();
        assert_eq!(a.backend, BackendChoice::Auto);
        let a = Args::try_parse_from(["instantmenu", "--backend", "x11"]).unwrap();
        assert_eq!(a.backend, BackendChoice::X11);
        let a = Args::try_parse_from(["instantmenu", "--backend", "wayland"]).unwrap();
        assert_eq!(a.backend, BackendChoice::Wayland);
        let a = Args::try_parse_from(["instantmenu", "--backend", "auto"]).unwrap();
        assert_eq!(a.backend, BackendChoice::Auto);
        assert!(Args::try_parse_from(["instantmenu", "--backend", "xorg"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--backend"]).is_err());
    }

    #[test]
    fn smartrun_invocation_parses() {
        /* the argv instantmenu_smartrun passes (see instantmenu_smartrun) */
        let argv = [
            "instantmenu",
            "--right-command",
            "instantmenu_smartrun terminal",
            "--left-command",
            "instantmenu_smartrun desktop",
            "-p",
            "desktop",
            "-i",
            "--fast",
            "--placeholder",
            "search apps",
            "-l",
            "10",
            "--position",
            "center",
            "--width",
            "-1",
            "--line-height",
            "-1",
            "--border-width",
            "4",
        ];
        assert!(Args::try_parse_from(argv).is_ok());
    }

    #[test]
    fn legacy_multichar_spellings_rejected() {
        /* the old single-dash multi-char spellings are gone; they must not
         * silently parse (e.g. -rc as -r -c with a stray positional) */
        for bad in [["-rc", "x"], ["-bw", "4"], ["-wm", "x"], ["-fn", "font"]] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn negative_numbers_accepted() {
        let a = Args::try_parse_from(["instantmenu", "-w", "-1", "--preselect", "-2"]).unwrap();
        assert_eq!(a.width, Some(-1));
        assert_eq!(a.preselect, Some(-2));
    }

    #[test]
    fn garbage_numbers_rejected() {
        assert!(Args::try_parse_from(["instantmenu", "--lines", "banana"]).is_err());
    }

    #[test]
    fn demoted_shorts_rejected() {
        /* the old uppercase and non-mnemonic shorts are now long-only */
        for bad in [
            &["instantmenu", "-P"][..],
            &["instantmenu", "-M"][..],
            &["instantmenu", "-W", "0x2a"][..],
            &["instantmenu", "-G"][..],
            &["instantmenu", "-T", "3"][..],
            &["instantmenu", "-C"][..],
            &["instantmenu", "-I"][..],
            &["instantmenu", "-H"][..],
            &["instantmenu", "-A"][..],
            &["instantmenu", "-q", "x"][..],
            &["instantmenu", "-v"][..],
        ] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn position_is_exclusive_with_follow_cursor() {
        assert!(
            Args::try_parse_from(["instantmenu", "--position", "bottom", "--follow-cursor"])
                .is_err()
        );
        let a = Args::try_parse_from(["instantmenu", "--position", "bottom"]).unwrap();
        assert_eq!(a.position, Some(Position::Bottom));
        let a = Args::try_parse_from(["instantmenu", "--follow-cursor"]).unwrap();
        assert_eq!(a.position, None);
        assert!(a.follow_cursor);
    }

    #[test]
    fn position_anchors_parse() {
        for (value, expected) in [
            ("top-left", Position::TopLeft),
            ("top", Position::Top),
            ("top-right", Position::TopRight),
            ("left", Position::Left),
            ("center", Position::Center),
            ("right", Position::Right),
            ("bottom-left", Position::BottomLeft),
            ("bottom", Position::Bottom),
            ("bottom-right", Position::BottomRight),
        ] {
            let a = Args::try_parse_from(["instantmenu", "--position", value]).unwrap();
            assert_eq!(a.position, Some(expected), "{value}");
        }
    }

    #[test]
    fn match_mode_parses() {
        let a = Args::try_parse_from(["instantmenu", "--match-mode", "exact"]).unwrap();
        assert_eq!(a.match_mode, Some(MatchMode::Exact));
        let a = Args::try_parse_from(["instantmenu", "--match-mode", "dmenu"]).unwrap();
        assert_eq!(a.match_mode, Some(MatchMode::Dmenu));
        assert!(Args::try_parse_from(["instantmenu", "--match-mode", "fuzzy"]).is_ok());
        /* the old spellings are gone */
        assert!(Args::try_parse_from(["instantmenu", "-F"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "-E"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--no-fuzzy"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--exact"]).is_err());
    }

    #[test]
    fn formerly_silent_overrides_now_rejected() {
        /* both flags used to parse with one silently winning */
        assert!(Args::try_parse_from(["instantmenu", "--font", "x", "--monospace"]).is_err());
    }

    #[test]
    fn nonsense_ranges_rejected() {
        /* negative values used to be accepted and misbehave at runtime */
        assert!(Args::try_parse_from(["instantmenu", "--toast", "-1"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--animation", "-5"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--border-width", "-3"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--monitor", "-2"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--lines", "-1"]).is_err());
    }

    #[test]
    fn monitor_automatic_is_accepted() {
        let a = Args::try_parse_from(["instantmenu", "-m", "-1"]).unwrap();
        assert_eq!(a.monitor, Some(-1));
    }

    #[test]
    fn embed_ids_parse_strictly() {
        assert_eq!(parse_window_id("0x2a"), Ok(42));
        assert_eq!(parse_window_id("42"), Ok(42));
        assert!(parse_window_id("banana").is_err());
        assert!(parse_window_id("-1").is_err());
        assert!(Args::try_parse_from(["instantmenu", "--embed", "0x2a"]).is_ok());
        assert!(Args::try_parse_from(["instantmenu", "--embed", "banana"]).is_err());
    }

    #[test]
    fn slide_args_parse() {
        let a = Args::try_parse_from(["instantmenu", "--slide"]).unwrap();
        assert!(a.slide);
        assert_eq!(a.min, 0);
        assert_eq!(a.max, 100);
        assert_eq!(a.value, None);

        let a = Args::try_parse_from([
            "instantmenu",
            "--slide",
            "--min",
            "-50",
            "--max",
            "50",
            "--value",
            "-10",
            "--step",
            "2",
            "--big-step",
            "10",
            "--command",
            "brightnessctl set",
            "-p",
            "Brightness",
        ])
        .unwrap();
        assert_eq!(a.min, -50);
        assert_eq!(a.max, 50);
        assert_eq!(a.value, Some(-10));
        assert_eq!(a.step, Some(2));
        assert_eq!(a.big_step, Some(10));
        assert_eq!(a.command.as_deref(), Some("brightnessctl set"));
    }

    #[test]
    fn slide_options_require_slide() {
        for bad in [
            &["instantmenu", "--max"][..],
            &["instantmenu", "--min", "0"][..],
            &["instantmenu", "--value", "50"][..],
            &["instantmenu", "--step", "1"][..],
            &["instantmenu", "--big-step", "5"][..],
            &["instantmenu", "--command", "true"][..],
        ] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should require --slide"
            );
        }
    }

    #[test]
    fn slide_rejects_contradictory_modes_and_steps() {
        for bad in [
            &["instantmenu", "--slide", "--toast", "5"][..],
            &["instantmenu", "--slide", "--commented"][..],
            &["instantmenu", "--slide", "--input-only"][..],
            &["instantmenu", "--slide", "--password"][..],
            &["instantmenu", "--slide", "--preselect", "1"][..],
            &["instantmenu", "--slide", "--step", "0"][..],
            &["instantmenu", "--slide", "--step", "-2"][..],
            &["instantmenu", "--slide", "--big-step", "0"][..],
        ] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        /* step >= 1 is fine, and min/max may be negative */
        assert!(Args::try_parse_from(["instantmenu", "--slide", "--step", "3"]).is_ok());
    }
}

/// Parse a window id for `--embed`: decimal, or 0x-prefixed hex like the C
/// `strtol`, but strict — garbage is an error instead of silently becoming 0.
fn parse_window_id(s: &str) -> Result<u32, String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|_| format!("invalid window id: `{s}`"))
    } else {
        t.parse::<u32>()
            .map_err(|_| format!("invalid window id: `{s}`"))
    }
}
