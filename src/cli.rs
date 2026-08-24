//! Command line: clap with long options as the canonical surface and
//! single-letter shorts for the common flags.
//!
//! The surface is split along two running modes. Menu mode is the default
//! (`instantmenu`); slide mode is a subcommand (`instantmenu slide`). Options
//! always follow the mode word: `instantmenu slide --width 600`, never
//! `instantmenu --width 600 slide` (`args_conflicts_with_subcommands` makes
//! clap reject any option that precedes a subcommand). Window, geometry,
//! font and color options are shared and marked global, so they work in both
//! modes; menu-only options live on the top level and slide-specific options
//! on the subcommand.
//!
//! Unlike the C `atoi`, numeric options parse strictly: a malformed number
//! is an error, not silently 0. Options that take a negative value
//! (`--x-offset -5`, `--y-offset -3`, ...) accept hyphen-prefixed values;
//! sizing options take `auto` instead of a negative sentinel.

use clap::{Parser, Subcommand};

use crate::backend::BackendChoice;
use crate::config::{LineHeight, MatchMode, MonitorChoice, Position, Theme, Width};
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
    args_conflicts_with_subcommands = true,
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

/// Menu-specific options: only applicable when running the default menu mode.
#[derive(clap::Args, Debug, Default, Clone)]
#[command(next_help_heading = "Menu mode options")]
pub struct MenuArgs {
    /// Reject input if it results in no matches.
    #[arg(long, short = 'r')]
    pub reject_no_match: bool,

    /// Toast mode that times out after SECONDS seconds.
    ///
    /// The menu draws itself, waits the given time, then exits without a
    /// selection. 0 keeps the timeout disabled.
    #[arg(
        long,
        value_name = "SECONDS",
        value_parser = parse_seconds
    )]
    pub toast: Option<f32>,

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

    /// Auto-confirm when exactly one item matches.
    #[arg(long, short = 'n')]
    pub auto_confirm: bool,

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

    /// Number of columns in grid mode.
    ///
    /// Implies one line per row unless --lines is given.
    #[arg(
        long,
        short = 'g',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(1..)
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

    /// Animation length in frames.
    #[arg(
        long = "animation-length",
        short = 'a',
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub animation_length: Option<i32>,

    /// Preselected item index.
    ///
    /// Starts from 0.
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
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

/// Window, geometry, font and color options shared by menu and slide modes.
#[derive(clap::Args, Debug, Clone)]
#[command(next_help_heading = "Window options")]
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
    /// `auto` sizes the menu to its content (the widest item plus the
    /// prompt).
    #[arg(
        long,
        short = 'w',
        value_name = "N|auto",
        value_parser = parse_width,
        global = true
    )]
    pub width: Option<Width>,

    /// Select monitor by index.
    ///
    /// Monitor numbers start from 0. `auto` follows keyboard focus, then
    /// uses the first monitor (the default). `--follow-cursor` explicitly
    /// selects the monitor containing the pointer.
    #[arg(
        long,
        short = 'm',
        value_name = "N|auto",
        value_parser = parse_monitor,
        global = true
    )]
    pub monitor: Option<MonitorChoice>,

    /// Prompt added to the left of the input field (menu mode) or the
    /// slider label (slide mode).
    #[arg(long, short = 'p', value_name = "TEXT", global = true)]
    pub prompt: Option<String>,

    /// Font or font set (overrides the X resource and default).
    #[arg(long, value_name = "FONT", conflicts_with = "monospace", global = true)]
    pub font: Option<String>,

    /// Minimum height of one menu line.
    ///
    /// At least 8 pixels; `auto` derives it from the font.
    #[arg(
        long,
        value_name = "N|auto",
        value_parser = parse_line_height,
        global = true
    )]
    pub line_height: Option<LineHeight>,

    /// Built-in color theme.
    ///
    /// `default` is an alias for `catppuccin`. Individual color options
    /// override the selected theme.
    #[arg(long, value_enum, value_name = "THEME", global = true)]
    pub theme: Option<Theme>,

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

/// Parse `--width`: a positive pixel count, or `auto` to fit the content.
fn parse_width(s: &str) -> Result<Width, String> {
    if s.eq_ignore_ascii_case("auto") {
        return Ok(Width::Auto);
    }
    let n = s
        .parse::<i32>()
        .map_err(|_| format!("invalid width: `{s}` (expected a positive number or `auto`)"))?;
    if n > 0 {
        Ok(Width::Fixed(n))
    } else {
        Err(format!(
            "width must be a positive number or `auto`, got `{s}`"
        ))
    }
}

/// Parse `--line-height`: a positive pixel count, or `auto` to derive it
/// from the font.
fn parse_line_height(s: &str) -> Result<LineHeight, String> {
    if s.eq_ignore_ascii_case("auto") {
        return Ok(LineHeight::FromFont);
    }
    let n = s.parse::<i32>().map_err(|_| {
        format!("invalid line height: `{s}` (expected a positive number or `auto`)")
    })?;
    if n > 0 {
        Ok(LineHeight::Pixels(n))
    } else {
        Err(format!(
            "line height must be a positive number or `auto`, got `{s}`"
        ))
    }
}

/// Parse `--monitor`: a 0-based index, or `auto`.
fn parse_monitor(s: &str) -> Result<MonitorChoice, String> {
    if s.eq_ignore_ascii_case("auto") {
        return Ok(MonitorChoice::Auto);
    }
    s.parse::<u32>()
        .map(MonitorChoice::Index)
        .map_err(|_| format!("invalid monitor: `{s}` (expected a number or `auto`)"))
}

/// Parse `--toast`: a number of seconds, fractions allowed.
fn parse_seconds(s: &str) -> Result<f32, String> {
    let v = s
        .parse::<f32>()
        .map_err(|_| format!("invalid number of seconds: `{s}`"))?;
    if v.is_finite() && v >= 0.0 {
        Ok(v)
    } else {
        Err(format!("seconds must be a non-negative number, got `{s}`"))
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
    fn built_in_themes_parse() {
        for (value, expected) in [
            ("default", Theme::Catppuccin),
            ("catppuccin", Theme::Catppuccin),
            ("classic", Theme::Classic),
            ("gruvbox", Theme::Gruvbox),
        ] {
            let a = Args::try_parse_from(["instantmenu", "--theme", value]).unwrap();
            assert_eq!(a.window.theme, Some(expected), "{value}");
        }
        assert!(Args::try_parse_from(["instantmenu", "--theme", "unknown"]).is_err());
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
        /* offsets are genuinely signed values */
        let a = Args::try_parse_from(["instantmenu", "-x", "-5", "-y", "-3"]).unwrap();
        assert_eq!(a.window.x_offset, Some(-5));
        assert_eq!(a.window.y_offset, Some(-3));
    }

    #[test]
    fn negative_sentinels_rejected() {
        /* sizing options take `auto` instead of the old negative sentinels,
         * and counts start at their natural minimum */
        for bad in [
            &["instantmenu", "--width", "-1"][..],
            &["instantmenu", "--width", "0"][..],
            &["instantmenu", "--line-height", "-1"][..],
            &["instantmenu", "--line-height", "0"][..],
            &["instantmenu", "--monitor", "-1"][..],
            &["instantmenu", "--preselect", "-2"][..],
            &["instantmenu", "--columns", "0"][..],
        ] {
            assert!(
                Args::try_parse_from(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn auto_sizing_values_parse() {
        let a = Args::try_parse_from(["instantmenu", "--width", "auto", "--line-height", "auto"])
            .unwrap();
        assert_eq!(a.window.width, Some(Width::Auto));
        assert_eq!(a.window.line_height, Some(LineHeight::FromFont));

        let a = Args::try_parse_from(["instantmenu", "-w", "900", "--line-height", "20"]).unwrap();
        assert_eq!(a.window.width, Some(Width::Fixed(900)));
        assert_eq!(a.window.line_height, Some(LineHeight::Pixels(20)));
    }

    #[test]
    fn monitor_choice_parses() {
        let a = Args::try_parse_from(["instantmenu", "-m", "auto"]).unwrap();
        assert_eq!(a.window.monitor, Some(MonitorChoice::Auto));
        let a = Args::try_parse_from(["instantmenu", "-m", "1"]).unwrap();
        assert_eq!(a.window.monitor, Some(MonitorChoice::Index(1)));
        assert!(Args::try_parse_from(["instantmenu", "-m", "banana"]).is_err());
    }

    #[test]
    fn toast_seconds_parse() {
        let a = Args::try_parse_from(["instantmenu", "--toast", "1.5"]).unwrap();
        assert_eq!(a.menu.toast, Some(1.5));
        /* 0 is accepted and means disabled (resolved in main) */
        let a = Args::try_parse_from(["instantmenu", "--toast", "0"]).unwrap();
        assert_eq!(a.menu.toast, Some(0.0));
        assert!(Args::try_parse_from(["instantmenu", "--toast", "banana"]).is_err());
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
    fn renamed_longs_parse() {
        /* --instant is now --auto-confirm and --animation is now
         * --animation-length; the old spellings must be rejected */
        let a = Args::try_parse_from(["instantmenu", "--auto-confirm"]).unwrap();
        assert!(a.menu.auto_confirm);
        let a = Args::try_parse_from(["instantmenu", "--animation-length", "5"]).unwrap();
        assert_eq!(a.menu.animation_length, Some(5));
        for bad in [
            &["instantmenu", "--instant"][..],
            &["instantmenu", "--animation", "5"][..],
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
        assert!(Args::try_parse_from(["instantmenu", "--animation-length", "-5"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--border-width", "-3"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--monitor", "-2"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "--lines", "-1"]).is_err());
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
            "slide",
            "-p",
            "Brightness",
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
        assert_eq!(a.window.width, Some(Width::Fixed(400)));
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
    fn shared_flags_apply_after_slide() {
        /* window options are global: valid after the mode word */
        let a = Args::try_parse_from(["instantmenu", "slide", "--width", "600", "--prompt", "B"])
            .unwrap();
        assert_eq!(a.window.width, Some(Width::Fixed(600)));
        assert_eq!(a.window.prompt.as_deref(), Some("B"));
        assert!(matches!(a.subcommand, Some(Cmd::Slide(_))));

        /* mode word first: any option before the subcommand is rejected */
        assert!(
            Args::try_parse_from(["instantmenu", "--width", "600", "--prompt", "B", "slide"])
                .is_err()
        );
    }

    #[test]
    fn menu_only_flags_are_rejected_in_slide_mode() {
        /* args_conflicts_with_subcommands: any option before the mode word
         * is a parse error, menu-only ones included */
        for argv in [
            &["instantmenu", "--reject-no-match", "slide"][..],
            &["instantmenu", "--toast", "5", "slide"][..],
            &["instantmenu", "--frecency-cache", "apps", "slide"][..],
        ] {
            assert!(Args::try_parse_from(argv).is_err(), "{argv:?}");
        }

        /* menu-only flags after slide are unknown arguments */
        assert!(Args::try_parse_from(["instantmenu", "slide", "--lines", "3"]).is_err());
        assert!(Args::try_parse_from(["instantmenu", "slide", "--password"]).is_err());

        /* menu-only flags are fine without a subcommand */
        let a = Args::try_parse_from(["instantmenu", "--insensitive"]).unwrap();
        assert!(a.menu.insensitive);
    }

    #[test]
    fn menu_only_options_are_not_global() {
        /* the rejection above rests on menu-only options not being global:
         * if one were ever marked global it would silently become valid
         * alongside `slide` again */
        use clap::CommandFactory;
        let mut cmd = Args::command();
        cmd.build();
        let members = cmd
            .get_groups()
            .find(|g| g.get_id().as_str() == "MenuArgs")
            .expect("MenuArgs arg group")
            .get_args()
            .collect::<Vec<_>>();
        assert!(!members.is_empty(), "MenuArgs group must have members");
        for id in members {
            let arg = cmd
                .get_arguments()
                .find(|a| a.get_id() == id)
                .expect("group member");
            assert!(
                !arg.is_global_set(),
                "--{} must not be global",
                arg.get_long().unwrap_or_default()
            );
        }
    }
}
