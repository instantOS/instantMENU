//! Command line: clap with long options (plus a few single-letter shorts
//! carried over from the C flags). Values keep C `atoi` semantics (parse
//! leading digits, default 0).

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
    #[arg(long)]
    pub reject_no_match: bool,

    /// Grab the keyboard before reading stdin.
    #[arg(long)]
    pub fast: bool,

    /// Toast mode that times out after a while (tenths of seconds).
    #[arg(long, value_name = "TENTHS")]
    pub toast: Option<String>,

    /// Activate instantASSIST mode (single-letter launcher).
    #[arg(long)]
    pub commented: bool,

    /// Center instantmenu on screen.
    #[arg(long)]
    pub centered: bool,

    /// Place the menu at the mouse position.
    #[arg(long)]
    pub follow_cursor: bool,

    /// Confirm using the space key (unimplemented in the original too).
    #[arg(long)]
    pub space_confirm: bool,

    /// Input only (no item list).
    #[arg(long)]
    pub input_only: bool,

    /// Enable smart case matching.
    #[arg(long)]
    pub smart_case: bool,

    /// Disable fuzzy matching.
    #[arg(long)]
    pub no_fuzzy: bool,

    /// Enable pre matching.
    #[arg(long)]
    pub pre_match: bool,

    /// Enable exact matching (implies --no-fuzzy).
    #[arg(long)]
    pub exact: bool,

    /// Make instantmenu take the full screen height.
    #[arg(long)]
    pub full_height: bool,

    /// Case-insensitive item matching.
    #[arg(long, short = 'i')]
    pub insensitive: bool,

    /// Instantly select the only match.
    #[arg(long)]
    pub instant: bool,

    /// Display input as dots.
    #[arg(long)]
    pub password: bool,

    /// Use a monospace font (Fira Code Nerd Font:pixelsize=15).
    #[arg(long)]
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
    pub right_cmd: Option<String>,

    /// Add a click target left of the input field running CMD.
    #[arg(long, value_name = "CMD")]
    pub left_cmd: Option<String>,

    /// Number of columns in grid mode (0 means 1; enables lines if unset).
    #[arg(long, value_name = "N")]
    pub columns: Option<String>,

    /// Number of lines in a vertical list.
    #[arg(long, short = 'l', value_name = "N")]
    pub lines: Option<String>,

    /// Window x offset.
    #[arg(long, short = 'x', value_name = "N")]
    pub x_offset: Option<String>,

    /// Window x offset counted from the right side of the screen.
    #[arg(long, value_name = "N")]
    pub right_x_offset: Option<String>,

    /// Window y offset (from bottom up with --bottom).
    #[arg(long, short = 'y', value_name = "N")]
    pub y_offset: Option<String>,

    /// Make instantmenu this wide.
    #[arg(long, short = 'w', value_name = "N")]
    pub width: Option<String>,

    /// Select monitor by index.
    #[arg(long, short = 'm', value_name = "N")]
    pub monitor: Option<String>,

    /// Prompt added to the left of the input field.
    #[arg(long, short = 'p', value_name = "TEXT")]
    pub prompt: Option<String>,

    /// Placeholder inside the input field.
    #[arg(long, value_name = "TEXT")]
    pub search_text: Option<String>,

    /// Font or font set (overrides the X resource and default).
    #[arg(long, value_name = "FONT")]
    pub font: Option<String>,

    /// Minimum height of one menu line.
    #[arg(long, value_name = "N")]
    pub line_height: Option<String>,

    /// Animation duration in frames.
    #[arg(long, value_name = "N")]
    pub animation: Option<String>,

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
    #[arg(long, value_name = "ID")]
    pub embed: Option<String>,

    /// Border width.
    #[arg(long, value_name = "N")]
    pub border_width: Option<String>,

    /// Preselected item index (a leading '-' is ignored).
    #[arg(long, value_name = "N")]
    pub preselect: Option<String>,

    /// Initial input text.
    #[arg(long, value_name = "TEXT")]
    pub initial_text: Option<String>,
}

pub fn parse() -> Args {
    Args::parse()
}

/// C `atoi`: leading whitespace, optional sign, leading digits; 0 otherwise.
pub fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let mut chars = t.chars().peekable();
    let mut sign = 1i64;
    if let Some(&c) = chars.peek() {
        if c == '-' || c == '+' {
            if c == '-' {
                sign = -1;
            }
            chars.next();
        }
    }
    let mut n: i64 = 0;
    for c in chars {
        match c.to_digit(10) {
            Some(d) => n = (n * 10 + d as i64).min(1 << 40),
            None => break,
        }
    }
    (sign * n).clamp(i32::MIN as i64, i32::MAX as i64) as i32
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
