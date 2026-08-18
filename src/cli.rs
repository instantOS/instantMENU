//! Command line: clap with long options as the canonical surface and
//! single-letter shorts for the common flags.
//!
//! Unlike the C `atoi`, numeric options parse strictly: a malformed number
//! is an error, not silently 0. Options that take a negative value
//! (`--width -1`, `--line-height -1`, ...) accept hyphen-prefixed values.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "instantmenu",
    about = "A dynamic menu for X11 and Wayland (instantMENU, Rust port)",
    disable_version_flag = true,
)]
pub struct Args {
    /// Print version information and exit.
    #[arg(long, short = 'v')]
    pub version: bool,

    /// Appears at the bottom of the screen.
    #[arg(long, short = 'b')]
    pub bottom: bool,

    /// Reject input if it results in no matches.
    #[arg(long, short = 'r')]
    pub reject_no_match: bool,

    /// Grab the keyboard before reading stdin.
    #[arg(long, short = 'f')]
    pub fast: bool,

    /// Toast mode that times out after a while (tenths of seconds).
    #[arg(long, short = 'T', value_name = "TENTHS", allow_hyphen_values = true)]
    pub toast: Option<i32>,

    /// Activate instantASSIST mode (single-letter launcher).
    #[arg(long)]
    pub commented: bool,

    /// Center instantmenu on screen.
    #[arg(long, short = 'c')]
    pub centered: bool,

    /// Place the menu at the mouse position.
    #[arg(long, short = 'C')]
    pub follow_cursor: bool,

    /// Input only (no item list).
    #[arg(long, short = 'I')]
    pub input_only: bool,

    /// Enable smart case matching.
    #[arg(long, short = 's')]
    pub smart_case: bool,

    /// Disable fuzzy matching.
    #[arg(long, short = 'F')]
    pub no_fuzzy: bool,

    /// Enable pre matching.
    #[arg(long)]
    pub pre_match: bool,

    /// Enable exact matching (implies --no-fuzzy).
    #[arg(long, short = 'E')]
    pub exact: bool,

    /// Make instantmenu take the full screen height.
    #[arg(long, short = 'H')]
    pub full_height: bool,

    /// Case-insensitive item matching.
    #[arg(long, short = 'i')]
    pub insensitive: bool,

    /// Instantly select the only match.
    #[arg(long, short = 'n')]
    pub instant: bool,

    /// Display input as dots.
    #[arg(long, short = 'P')]
    pub password: bool,

    /// Use a monospace font (Fira Code Nerd Font:pixelsize=15).
    #[arg(long, short = 'M')]
    pub monospace: bool,

    /// Don't grab the keyboard.
    #[arg(long, short = 'G')]
    pub no_grab: bool,

    /// Alt-tab behaviour.
    #[arg(long, short = 'A')]
    pub alt_tab: bool,

    /// Display as a managed wm window.
    #[arg(long)]
    pub managed: bool,

    /// Execute this command on shift + right arrow.
    #[arg(long, value_name = "CMD")]
    pub right_cmd: Option<String>,

    /// Add a click target left of the input field running CMD.
    #[arg(long, value_name = "CMD")]
    pub left_cmd: Option<String>,

    /// Number of columns in grid mode (0 means 1; enables lines if unset).
    #[arg(long, short = 'g', value_name = "N")]
    pub columns: Option<i32>,

    /// Number of lines in a vertical list.
    #[arg(long, short = 'l', value_name = "N")]
    pub lines: Option<i32>,

    /// Window x offset.
    #[arg(long, short = 'x', value_name = "N", allow_hyphen_values = true)]
    pub x_offset: Option<i32>,

    /// Window x offset counted from the right side of the screen.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub right_x_offset: Option<i32>,

    /// Window y offset (from bottom up with --bottom).
    #[arg(long, short = 'y', value_name = "N", allow_hyphen_values = true)]
    pub y_offset: Option<i32>,

    /// Make instantmenu this wide (negative: auto width).
    #[arg(long, short = 'w', value_name = "N", allow_hyphen_values = true)]
    pub width: Option<i32>,

    /// Select monitor by index.
    #[arg(long, short = 'm', value_name = "N", allow_hyphen_values = true)]
    pub monitor: Option<i32>,

    /// Prompt added to the left of the input field.
    #[arg(long, short = 'p', value_name = "TEXT")]
    pub prompt: Option<String>,

    /// Placeholder inside the input field.
    #[arg(long, short = 'q', value_name = "TEXT")]
    pub search_text: Option<String>,

    /// Font or font set (overrides the X resource and default).
    #[arg(long, value_name = "FONT")]
    pub font: Option<String>,

    /// Minimum height of one menu line.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub line_height: Option<i32>,

    /// Animation duration in frames.
    #[arg(long, short = 'a', value_name = "N")]
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

    /// Embedding window id.
    #[arg(long, short = 'W', value_name = "ID")]
    pub embed: Option<String>,

    /// Border width.
    #[arg(long, value_name = "N")]
    pub border_width: Option<i32>,

    /// Preselected item index.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    pub preselect: Option<i32>,

    /// Initial input text.
    #[arg(long, value_name = "TEXT")]
    pub initial_text: Option<String>,
}

pub fn parse() -> Args {
    Args::parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smartrun_invocation_parses() {
        /* the argv instantmenu_smartrun passes (see instantmenu_smartrun) */
        let argv = [
            "instantmenu",
            "--right-cmd", "instantmenu_smartrun terminal",
            "--left-cmd", "instantmenu_smartrun desktop",
            "-p", "desktop", "-i", "--fast", "--search-text", "search apps",
            "-l", "10", "--centered", "--width", "-1",
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
    fn strtol0_c_semantics() {
        assert_eq!(strtol0("0x2a"), 42);
        assert_eq!(strtol0("0Xff"), 255);
        assert_eq!(strtol0("42"), 42);
        assert_eq!(strtol0("-1"), u32::MAX); /* wrapped, like C */
        assert_eq!(strtol0("abc"), 0);
    }
}

/// C `strtol(s, NULL, 0)`: 0x-prefixed hex, else decimal.
pub fn strtol0(s: &str) -> u32 {
    let t = s.trim_start();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let v = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(&hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect::<String>(), 16)
            .unwrap_or(0)
    } else {
        t.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    };
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}