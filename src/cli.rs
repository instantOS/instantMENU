//! Command line: clap with long options as the canonical surface and
//! single-letter shorts for the common flags.
//!
//! Unlike the C `atoi`, numeric options parse strictly: a malformed number
//! is an error, not silently 0. Options that take a negative value
//! (`--width -1`, `--line-height -1`, ...) accept hyphen-prefixed values.

use clap::Parser;

use crate::config::{MatchMode, Position};

#[derive(Parser, Debug)]
#[command(
    name = "instantmenu",
    about = "A dynamic menu for X11 and Wayland (instantMENU, Rust port)",
    version = crate::config::VERSION,
)]
pub struct Args {
    /// Where the menu appears on screen: top, bottom or centered.
    #[arg(long, value_enum, value_name = "POSITION", conflicts_with = "follow_cursor")]
    pub position: Option<Position>,

    /// Reject input if it results in no matches.
    #[arg(long, short = 'r')]
    pub reject_no_match: bool,

    /// Grab the keyboard before reading stdin.
    #[arg(long, short = 'f')]
    pub fast: bool,

    /// Toast mode that times out after a while (tenths of seconds).
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

    /// Place the menu at the mouse position.
    #[arg(long, conflicts_with = "position")]
    pub follow_cursor: bool,

    /// Input only (no item list).
    #[arg(long)]
    pub input_only: bool,

    /// Enable smart case matching.
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

    /// Display as a managed wm window.
    #[arg(long)]
    pub managed: bool,

    /// Execute this command on shift + right arrow.
    #[arg(long, value_name = "CMD")]
    pub right_command: Option<String>,

    /// Add a click target left of the input field running CMD.
    #[arg(long, value_name = "CMD")]
    pub left_command: Option<String>,

    /// Number of columns in grid mode (0 means 1; enables lines if unset).
    #[arg(long, short = 'g', value_name = "N", value_parser = clap::value_parser!(i32).range(0..))]
    pub columns: Option<i32>,

    /// Number of lines in a vertical list.
    #[arg(long, short = 'l', value_name = "N", value_parser = clap::value_parser!(i32).range(0..))]
    pub lines: Option<i32>,

    /// Window x offset.
    #[arg(
        long,
        short = 'x',
        value_name = "N",
        allow_hyphen_values = true,
        conflicts_with = "right_x_offset"
    )]
    pub x_offset: Option<i32>,

    /// Window x offset counted from the right side of the screen.
    #[arg(
        long,
        value_name = "N",
        allow_hyphen_values = true,
        conflicts_with = "x_offset"
    )]
    pub right_x_offset: Option<i32>,

    /// Window y offset (measured from the bottom with --position bottom).
    #[arg(long, short = 'y', value_name = "N", allow_hyphen_values = true)]
    pub y_offset: Option<i32>,

    /// Make instantmenu this wide (negative: auto width).
    #[arg(long, short = 'w', value_name = "N", allow_hyphen_values = true)]
    pub width: Option<i32>,

    /// Select monitor by index (-1: automatic).
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
    #[arg(long, value_name = "COLOR")]
    pub normal_bg: Option<String>,

    /// Normal foreground color.
    #[arg(long, value_name = "COLOR")]
    pub normal_fg: Option<String>,

    /// Selected background color.
    #[arg(long, value_name = "COLOR")]
    pub selected_bg: Option<String>,

    /// Selected foreground color.
    #[arg(long, value_name = "COLOR")]
    pub selected_fg: Option<String>,

    /// Embedding window id (X11 only).
    #[arg(long, value_name = "ID", value_parser = parse_window_id)]
    pub embed: Option<u32>,

    /// Border width.
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(i32).range(0..)
    )]
    pub border_width: Option<i32>,

    /// Preselected item index.
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
    fn smartrun_invocation_parses() {
        /* the argv instantmenu_smartrun passes (see instantmenu_smartrun) */
        let argv = [
            "instantmenu",
            "--right-command", "instantmenu_smartrun terminal",
            "--left-command", "instantmenu_smartrun desktop",
            "-p", "desktop", "-i", "--fast", "--placeholder", "search apps",
            "-l", "10", "--position", "centered", "--width", "-1",
            "--line-height", "-1", "--border-width", "4",
        ];
        assert!(Args::try_parse_from(argv).is_ok());
    }

    #[test]
    fn legacy_multichar_spellings_rejected() {
        /* the old single-dash multi-char spellings are gone; they must not
         * silently parse (e.g. -rc as -r -c with a stray positional) */
        for bad in [["-rc", "x"], ["-bw", "4"], ["-wm", "x"], ["-fn", "font"]] {
            assert!(Args::try_parse_from(bad).is_err(), "{bad:?} should be rejected");
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
            assert!(Args::try_parse_from(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn position_is_exclusive_with_follow_cursor() {
        assert!(Args::try_parse_from(["instantmenu", "--position", "bottom", "--follow-cursor"]).is_err());
        let a = Args::try_parse_from(["instantmenu", "--position", "bottom"]).unwrap();
        assert_eq!(a.position, Some(Position::Bottom));
        let a = Args::try_parse_from(["instantmenu", "--follow-cursor"]).unwrap();
        assert_eq!(a.position, None);
        assert!(a.follow_cursor);
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
        assert!(Args::try_parse_from(["instantmenu", "--x-offset", "5", "--right-x-offset", "10"]).is_err());
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
}

/// Parse a window id for `--embed`: decimal, or 0x-prefixed hex like the C
/// `strtol`, but strict — garbage is an error instead of silently becoming 0.
fn parse_window_id(s: &str) -> Result<u32, String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|_| format!("invalid window id: `{s}`"))
    } else {
        t.parse::<u32>().map_err(|_| format!("invalid window id: `{s}`"))
    }
}
