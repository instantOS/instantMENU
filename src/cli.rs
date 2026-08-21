//! Command line: clap with long options as the canonical surface and
//! single-letter shorts for the common flags.
//!
//! The surface is split along two running modes. Menu mode is the default
//! (`instantmenu`); slide mode is a subcommand (`instantmenu slide`). Window,
//! geometry, font and color options are shared and live on the top level,
//! where they may only appear *before* a subcommand. Menu-only options also
//! live on the top level but are rejected at startup when a subcommand is
//! given, and the slide-specific options live on the subcommand itself.
//!
//! Unlike the C `atoi`, numeric options parse strictly: a malformed number
//! is an error, not silently 0. Options that take a negative value
//! (`--width -1`, `--line-height -1`, ...) accept hyphen-prefixed values.

use clap::{Parser, Subcommand};

use crate::backend::BackendChoice;
use crate::config::{MatchMode, Position};
use std::path::PathBuf;

/// Long-form description shown by `--help` and the generated man page.
const LONG_ABOUT: &str = concat!(
    "instantmenu reads a newline-separated list of items from stdin and ",
    "displays them in a menu. Selecting an item and pressing Return prints ",
    "it to stdout and exits; typing narrows the list to matching items.\n\n",
    "The menu opens immediately and items stream in: the keyboard is ",
    "grabbed before stdin is read, and the list grows as the input ",
    "produces it (a fast producer collapses into a single refresh). With ",
    "the `slide` subcommand it shows a value slider instead: Return ",
    "prints the value and exits, every change runs --command with the value ",
    "appended.",
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
    "SLIDE MODE (`slide`):\n",
    "  Left h       decrease by --step        Right l    increase by --step\n",
    "  Down j       decrease by --big-step    Up k       increase by --big-step\n",
    "  plus/minus   change by 1               1..9, 0    jump to a ninth of the range\n",
    "  Home         minimum value             End        maximum value\n",
    "  Return       print the value and exit  Escape q   exit without printing\n",
    "  click/drag   set the value at the pointer         wheel  step\n",
    "  middle click reset to the initial value\n",
);

/// The top-level command. Menu mode is the default; pass a subcommand to run
/// one of the other modes instead.
#[derive(Parser, Debug)]
#[command(
    name = "instantmenu",
    about = "A dynamic menu for X11 and Wayland (instantMENU, Rust port)",
    long_about = LONG_ABOUT,
    version = crate::config::VERSION,
    after_long_help = KEY_BINDINGS,
    disable_help_subcommand = true,
)]
pub struct Args {
    /// Window, geometry, font and color options shared by all modes.
    #[command(flatten)]
    pub window: WindowArgs,

    /// Menu mode options.
    #[command(flatten)]
    pub menu: MenuArgs,

    /// Running mode: menu (default) or slide.
    #[command(subcommand)]
    pub subcommand: Option<Cmd>,
}

impl Args {
    /// The first menu-only option passed alongside a subcommand, if any.
    ///
    /// Menu-only options cannot be used when a subcommand like `slide` is active.
    pub fn menu_only_option_in_subcommand(&self) -> Option<&'static str> {
        self.subcommand.as_ref()?;
        self.menu.active_flag()
    }
}

/// Menu-specific options: only applicable when running the default menu mode.
#[derive(clap::Args, Debug, Default, Clone)]
pub struct MenuArgs {
    /// Reject input if it results in no matches.
    #[arg(long, short = 'r')]
    pub reject_no_match: bool,

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

    /// Alt-tab behaviour.
    #[arg(long)]
    pub alt_tab: bool,

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
    #[arg(
        long,
        short = 'g',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub columns: Option<i32>,

    /// Number of lines in a vertical list.
    #[arg(
        long,
        short = 'l',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub lines: Option<i32>,

    /// Placeholder inside the input field.
    #[arg(long, value_name = "TEXT")]
    pub placeholder: Option<String>,

    /// Animation duration in frames.
    #[arg(
        long,
        short = 'a',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub animation: Option<i32>,

    /// Preselected item index.
    ///
    /// Starts from 0.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub preselect: Option<i32>,

    /// Initial input text.
    #[arg(long, value_name = "TEXT")]
    pub initial_text: Option<String>,

    /// Frecency cache: rank items by past selections and record new ones.
    ///
    /// The value is a cache ID resolved under the XDG cache directory
    /// ($XDG_CACHE_HOME/instantmenu/<ID>, or ~/.cache/instantmenu/<ID>);
    /// an absolute path is used as the cache file directly. Distinct IDs
    /// hold independent histories (e.g. one per launcher).
    ///
    /// On startup items are reordered best-frecency first (stable — ties
    /// keep stdin order). Every printed selection — a chosen item or
    /// free-typed text — is counted with a time decay and persisted.
    /// Not recorded: password input and slider values.
    #[arg(long, value_name = "ID")]
    pub frecency_cache: Option<PathBuf>,
}

impl MenuArgs {
    /// Return the flag name of the first present menu-only option, if any.
    pub fn active_flag(&self) -> Option<&'static str> {
        if self.reject_no_match {
            return Some("--reject-no-match");
        }
        if self.toast.is_some() {
            return Some("--toast");
        }
        if self.commented {
            return Some("--commented");
        }
        if self.input_only {
            return Some("--input-only");
        }
        if self.smart_case {
            return Some("--smart-case");
        }
        if self.match_mode.is_some() {
            return Some("--match-mode");
        }
        if self.pre_match {
            return Some("--pre-match");
        }
        if self.space_confirm {
            return Some("--space-confirm");
        }
        if self.full_height {
            return Some("--full-height");
        }
        if self.insensitive {
            return Some("--insensitive");
        }
        if self.instant {
            return Some("--instant");
        }
        if self.password {
            return Some("--password");
        }
        if self.alt_tab {
            return Some("--alt-tab");
        }
        if self.right_command.is_some() {
            return Some("--right-command");
        }
        if self.left_command.is_some() {
            return Some("--left-command");
        }
        if self.columns.is_some() {
            return Some("--columns");
        }
        if self.lines.is_some() {
            return Some("--lines");
        }
        if self.placeholder.is_some() {
            return Some("--placeholder");
        }
        if self.animation.is_some() {
            return Some("--animation");
        }
        if self.preselect.is_some() {
            return Some("--preselect");
        }
        if self.initial_text.is_some() {
            return Some("--initial-text");
        }
        if self.frecency_cache.is_some() {
            return Some("--frecency-cache");
        }
        None
    }
}

/// Window, geometry, font and color options shared by menu and slide modes.
#[derive(clap::Args, Debug, Clone)]
pub struct WindowArgs {
    /// Backend to use: auto, x11 or wayland.
    #[arg(
        long,
        value_enum,
        value_name = "BACKEND",
        default_value = "auto",
        global = true
    )]
    pub backend: BackendChoice,

    /// Anchor the menu to a corner, edge or the center of the screen.
    #[arg(
        long,
        value_enum,
        value_name = "POSITION",
        conflicts_with = "follow_cursor",
        global = true
    )]
    pub position: Option<Position>,

    /// Place the menu at the mouse position.
    #[arg(long, conflicts_with = "position", global = true)]
    pub follow_cursor: bool,

    /// Use a monospace font (Fira Code Nerd Font:pixelsize=15).
    #[arg(long, conflicts_with = "font", global = true)]
    pub monospace: bool,

    /// Don't grab the keyboard.
    #[arg(long, global = true)]
    pub no_grab: bool,

    /// Don't close the menu when clicking outside of it.
    ///
    /// By default a modal menu (keyboard grabbed) closes on a click outside
    /// of it, like a GTK context menu.
    #[arg(long, global = true)]
    pub no_outside_close: bool,

    /// Let instantmenu be managed by the window manager as a normal window.
    #[arg(long, global = true)]
    pub managed: bool,

    /// Horizontal offset from the anchor position.
    ///
    /// Positive moves right, negative moves left.
    #[arg(
        long,
        short = 'x',
        value_name = "N",
        allow_hyphen_values = true,
        global = true
    )]
    pub x_offset: Option<i32>,

    /// Vertical offset from the anchor position.
    ///
    /// Positive moves down, negative moves up.
    #[arg(
        long,
        short = 'y',
        value_name = "N",
        allow_hyphen_values = true,
        global = true
    )]
    pub y_offset: Option<i32>,

    /// Make instantmenu this wide.
    ///
    /// A negative value adjusts the width to the longest line read from
    /// stdin.
    #[arg(
        long,
        short = 'w',
        value_name = "N",
        allow_hyphen_values = true,
        global = true
    )]
    pub width: Option<i32>,

    /// Select monitor by index.
    ///
    /// Monitor numbers start from 0. Use -1 for automatic selection.
    #[arg(
        long,
        short = 'm',
        value_name = "N",
        allow_hyphen_values = true,
        value_parser = clap::value_parser!(i32).range(-1..),
        global = true
    )]
    pub monitor: Option<i32>,

    /// Prompt added to the left of the input field (menu mode) or the
    /// slider label (slide mode).
    #[arg(long, short = 'p', value_name = "TEXT", global = true)]
    pub prompt: Option<String>,

    /// Font or font set (overrides the X resource and default).
    #[arg(long, value_name = "FONT", conflicts_with = "monospace", global = true)]
    pub font: Option<String>,

    /// Minimum height of one menu line.
    ///
    /// At least 8 pixels.
    #[arg(long, value_name = "N", allow_hyphen_values = true, global = true)]
    pub line_height: Option<i32>,

    /// Normal background color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR", global = true)]
    pub normal_bg: Option<String>,

    /// Normal foreground color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR", global = true)]
    pub normal_fg: Option<String>,

    /// Selected background color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR", global = true)]
    pub selected_bg: Option<String>,

    /// Selected foreground color.
    ///
    /// Supports #RGB, #RRGGBB and X color names.
    #[arg(long, value_name = "COLOR", global = true)]
    pub selected_fg: Option<String>,

    /// Embedding window id (X11 only).
    #[arg(long, value_name = "ID", value_parser = parse_window_id, global = true)]
    pub embed: Option<u32>,

    /// Border width.
    ///
    /// Adds a border around the menu.
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..),
        global = true
    )]
    pub border_width: Option<i32>,
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

/// A non-default running mode, selected with a subcommand.
#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Slide mode: show a value slider instead of a menu.
    ///
    /// Return prints the current value to stdout and exits; Escape exits
    /// without printing. Every value change runs --command (if given) with
    /// the value appended as its last argument.
    Slide(SlideArgs),
}

/// Options for [`Cmd::Slide`] — the value slider.
#[derive(clap::Args, Debug, Clone)]
pub struct SlideArgs {
    /// Minimum slider value.
    #[arg(
        long,
        value_name = "N",
        allow_hyphen_values = true,
        default_value_t = 0
    )]
    pub min: i32,

    /// Maximum slider value.
    #[arg(
        long,
        value_name = "N",
        allow_hyphen_values = true,
        default_value_t = 100
    )]
    pub max: i32,

    /// Initial slider value.
    ///
    /// Defaults to the middle of the range; clamped into it.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub value: Option<i32>,

    /// Small step for left/right.
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(1..)
    )]
    pub step: Option<i32>,

    /// Large step for up/down.
    ///
    /// Defaults to a tenth of the range (at least 5).
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(1..)
    )]
    pub big_step: Option<i32>,

    /// Command run on every value change.
    ///
    /// Run through the shell with the current value appended as the last
    /// argument. Can be given as a positional argument or via --command.
    #[arg(value_name = "COMMAND")]
    pub command: Option<String>,

    /// Command run on every value change (flag form of `COMMAND`).
    #[arg(long = "command", value_name = "CMD", conflicts_with = "command")]
    pub command_flag: Option<String>,
}

impl SlideArgs {
    /// Return the command from either the positional argument or the `--command` flag.
    pub fn resolved_command(&self) -> Option<String> {
        self.command.clone().or_else(|| self.command_flag.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_choice_parses() {
        use crate::backend::BackendChoice;

        let a = Args::try_parse_from(["instantmenu"]).unwrap();
        assert_eq!(a.window.backend, BackendChoice::Auto);
        let a = Args::try_parse_from(["instantmenu", "--backend", "x11"]).unwrap();
        assert_eq!(a.window.backend, BackendChoice::X11);
        let a = Args::try_parse_from(["instantmenu", "--backend", "wayland"]).unwrap();
        assert_eq!(a.window.backend, BackendChoice::Wayland);
        let a = Args::try_parse_from(["instantmenu", "--backend", "auto"]).unwrap();
        assert_eq!(a.window.backend, BackendChoice::Auto);
        assert!(Args::try_parse_from(["instantmenu", "--backend", "xorg"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--backend"]).is_err());
    }

    #[test]
    fn smartrun_invocation_parses() {
        /* the argv instantmenu_smartrun passes in its default mode (see
         * instantmenu_smartrun) */
        let argv = [
            "instantmenu",
            "--frecency-cache",
            "smartrun",
            "--right-command",
            "instantmenu_smartrun terminal",
            "--left-command",
            "instantmenu_smartrun desktop",
            "-p",
            "",
            "-i",
            "--placeholder",
            "search apps",
            "-l",
            "10",
            "--position",
            "center",
            "--width",
            "900",
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
        assert_eq!(a.window.width, Some(-1));
        assert_eq!(a.menu.preselect, Some(-2));
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
        assert_eq!(a.window.position, Some(Position::Bottom));
        let a = Args::try_parse_from(["instantmenu", "--follow-cursor"]).unwrap();
        assert_eq!(a.window.position, None);
        assert!(a.window.follow_cursor);
    }

    #[test]
    fn no_outside_close_parses() {
        let a = Args::try_parse_from(["instantmenu"]).unwrap();
        assert!(!a.window.no_outside_close);
        let a = Args::try_parse_from(["instantmenu", "--no-outside-close"]).unwrap();
        assert!(a.window.no_outside_close);
        /* it is a window option, so it is also valid in slide mode */
        let a = Args::try_parse_from(["instantmenu", "slide", "--no-outside-close"]).unwrap();
        assert!(a.window.no_outside_close);
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
            assert_eq!(a.window.position, Some(expected), "{value}");
        }
    }

    #[test]
    fn match_mode_parses() {
        let a = Args::try_parse_from(["instantmenu", "--match-mode", "exact"]).unwrap();
        assert_eq!(a.menu.match_mode, Some(MatchMode::Exact));
        let a = Args::try_parse_from(["instantmenu", "--match-mode", "dmenu"]).unwrap();
        assert_eq!(a.menu.match_mode, Some(MatchMode::Dmenu));
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
        assert_eq!(a.window.monitor, Some(-1));
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
        let a = Args::try_parse_from(["instantmenu", "slide"]).unwrap();
        let Some(Cmd::Slide(s)) = a.subcommand.as_ref() else {
            panic!("expected the slide subcommand");
        };
        assert_eq!(s.min, 0);
        assert_eq!(s.max, 100);
        assert_eq!(s.value, None);

        /* positional command */
        let a = Args::try_parse_from([
            "instantmenu",
            "-p",
            "Brightness",
            "slide",
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
            "brightnessctl set",
        ])
        .unwrap();
        assert_eq!(a.window.prompt.as_deref(), Some("Brightness"));
        let Some(Cmd::Slide(s)) = a.subcommand.as_ref() else {
            panic!("expected the slide subcommand");
        };
        assert_eq!(s.min, -50);
        assert_eq!(s.max, 50);
        assert_eq!(s.value, Some(-10));
        assert_eq!(s.step, Some(2));
        assert_eq!(s.big_step, Some(10));
        assert_eq!(s.command.as_deref(), Some("brightnessctl set"));
        assert_eq!(s.resolved_command().as_deref(), Some("brightnessctl set"));

        /* --command flag compatibility */
        let a = Args::try_parse_from(["instantmenu", "slide", "--command", "brightnessctl set"])
            .unwrap();
        let Some(Cmd::Slide(s)) = a.subcommand.as_ref() else {
            panic!("expected the slide subcommand");
        };
        assert_eq!(s.command_flag.as_deref(), Some("brightnessctl set"));
        assert_eq!(s.resolved_command().as_deref(), Some("brightnessctl set"));

        /* positional and flag cannot both be passed */
        assert!(
            Args::try_parse_from(["instantmenu", "slide", "cmd1", "--command", "cmd2",]).is_err()
        );

        /* flags after the subcommand also work */
        let a = Args::try_parse_from([
            "instantmenu",
            "slide",
            "-p",
            "Volume",
            "--width",
            "400",
            "--min",
            "0",
            "--max",
            "100",
            "pamixer --set-volume",
        ])
        .unwrap();
        assert_eq!(a.window.prompt.as_deref(), Some("Volume"));
        assert_eq!(a.window.width, Some(400));
        let Some(Cmd::Slide(s)) = a.subcommand.as_ref() else {
            panic!("expected the slide subcommand");
        };
        assert_eq!(s.min, 0);
        assert_eq!(s.max, 100);
        assert_eq!(
            s.resolved_command().as_deref(),
            Some("pamixer --set-volume")
        );
    }

    #[test]
    fn slide_options_rejected_without_the_slide_subcommand() {
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
                "{bad:?} should require the `slide` subcommand"
            );
        }
    }

    #[test]
    fn slide_bad_steps_rejected() {
        for bad in [
            &["instantmenu", "slide", "--step", "0"][..],
            &["instantmenu", "slide", "--step", "-2"][..],
            &["instantmenu", "slide", "--big-step", "0"][..],
        ] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        /* step >= 1 is fine, and min/max may be negative */
        assert!(Args::try_parse_from(["instantmenu", "slide", "--step", "3"]).is_ok());
    }

    #[test]
    fn shared_flags_apply_before_and_after_slide() {
        /* shared flags before the subcommand */
        let a = Args::try_parse_from(["instantmenu", "--width", "600", "--prompt", "B", "slide"])
            .unwrap();
        assert_eq!(a.window.width, Some(600));
        assert_eq!(a.window.prompt.as_deref(), Some("B"));
        assert!(matches!(a.subcommand, Some(Cmd::Slide(_))));
        assert_eq!(a.menu_only_option_in_subcommand(), None);

        /* shared flags after the subcommand */
        let a = Args::try_parse_from(["instantmenu", "slide", "--width", "600", "--prompt", "B"])
            .unwrap();
        assert_eq!(a.window.width, Some(600));
        assert_eq!(a.window.prompt.as_deref(), Some("B"));
        assert!(matches!(a.subcommand, Some(Cmd::Slide(_))));
        assert_eq!(a.menu_only_option_in_subcommand(), None);
    }

    #[test]
    fn menu_only_flags_are_rejected_in_slide_mode() {
        for (argv, flag) in [
            (
                &["instantmenu", "--reject-no-match", "slide"][..],
                "--reject-no-match",
            ),
            (&["instantmenu", "--toast", "5", "slide"][..], "--toast"),
            (&["instantmenu", "--commented", "slide"][..], "--commented"),
            (
                &["instantmenu", "--input-only", "slide"][..],
                "--input-only",
            ),
            (
                &["instantmenu", "--smart-case", "slide"][..],
                "--smart-case",
            ),
            (
                &["instantmenu", "--match-mode", "fuzzy", "slide"][..],
                "--match-mode",
            ),
            (&["instantmenu", "--pre-match", "slide"][..], "--pre-match"),
            (
                &["instantmenu", "--space-confirm", "slide"][..],
                "--space-confirm",
            ),
            (
                &["instantmenu", "--full-height", "slide"][..],
                "--full-height",
            ),
            (
                &["instantmenu", "--insensitive", "slide"][..],
                "--insensitive",
            ),
            (&["instantmenu", "--instant", "slide"][..], "--instant"),
            (&["instantmenu", "--password", "slide"][..], "--password"),
            (&["instantmenu", "--alt-tab", "slide"][..], "--alt-tab"),
            (
                &["instantmenu", "--right-command", "true", "slide"][..],
                "--right-command",
            ),
            (
                &["instantmenu", "--left-command", "true", "slide"][..],
                "--left-command",
            ),
            (&["instantmenu", "--columns", "2", "slide"][..], "--columns"),
            (&["instantmenu", "--lines", "3", "slide"][..], "--lines"),
            (
                &["instantmenu", "--placeholder", "x", "slide"][..],
                "--placeholder",
            ),
            (
                &["instantmenu", "--animation", "5", "slide"][..],
                "--animation",
            ),
            (
                &["instantmenu", "--preselect", "1", "slide"][..],
                "--preselect",
            ),
            (
                &["instantmenu", "--initial-text", "x", "slide"][..],
                "--initial-text",
            ),
            (
                &["instantmenu", "--frecency-cache", "apps", "slide"][..],
                "--frecency-cache",
            ),
        ] {
            let a = Args::try_parse_from(argv).unwrap();
            assert_eq!(a.menu_only_option_in_subcommand(), Some(flag), "{argv:?}");
        }

        /* menu-only flags after slide are rejected by clap parser */
        assert!(Args::try_parse_from(["instantmenu", "slide", "--lines", "3"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "slide", "--password"]).is_err());

        /* menu-only flags are fine without a subcommand */
        let a = Args::try_parse_from(["instantmenu", "--insensitive"]).unwrap();
        assert_eq!(a.menu_only_option_in_subcommand(), None);
    }
}
