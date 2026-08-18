//! Command line: clap with the legacy instantmenu flags.
//!
//! The original argv loop (`-fn`, `-nb`, `-wm`, `-ct`, ...) predates GNU-style
//! option parsing; a shim rewrites the legacy tokens to long options before
//! clap sees them, so scripts keep working unchanged. Values keep C `atoi`
//! semantics (parse leading digits, default 0).

use clap::Parser;

/// Legacy flag (no argument) → long option.
const FLAGS: &[(&str, &str)] = &[
    ("-b", "--bottom"),
    ("-r", "--reject-no-match"),
    ("-f", "--fast"),
    ("-ct", "--commented"),
    ("-c", "--centered"),
    ("-C", "--follow-cursor"),
    ("-S", "--space-confirm"),
    ("-I", "--input-only"),
    ("-s", "--smart-case"),
    ("-F", "--no-fuzzy"),
    ("-pm", "--pre-match"),
    ("-E", "--exact"),
    ("-H", "--full-height"),
    ("-i", "--insensitive"),
    ("-n", "--instant"),
    ("-P", "--password"),
    ("-M", "--monospace"),
    ("-G", "--no-grab"),
    ("-A", "--alt-tab"),
    ("-wm", "--managed"),
];

/// Legacy flag with one argument → long option taking a value.
const OPTS: &[(&str, &str)] = &[
    ("-T", "--toast"),
    ("-rc", "--right-cmd"),
    ("-lc", "--left-cmd"),
    ("-g", "--columns"),
    ("-l", "--lines"),
    ("-x", "--x-offset"),
    ("-xr", "--right-x-offset"),
    ("-y", "--y-offset"),
    ("-w", "--width"),
    ("-m", "--monitor"),
    ("-p", "--prompt"),
    ("-q", "--search-text"),
    ("-fn", "--font"),
    ("-h", "--line-height"),
    ("-a", "--animation"),
    ("-nb", "--normal-bg"),
    ("-nf", "--normal-fg"),
    ("-sb", "--selected-bg"),
    ("-sf", "--selected-fg"),
    ("-W", "--embed"),
    ("-bw", "--border-width"),
    ("-ps", "--preselect"),
    ("-it", "--initial-text"),
];

#[derive(Parser, Debug)]
#[command(
    name = "instantmenu",
    about = "A dynamic menu for X11 and Wayland (instantMENU, Rust port)",
    disable_help_flag = true,
    disable_version_flag = true,
)]
pub struct Args {
    /// Print version information and exit.
    #[arg(long, short = 'v')]
    pub version: bool,

    /// Show this help and exit.
    #[arg(long)]
    pub help: bool,

    /// Appears at the bottom of the screen.
    #[arg(long)]
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
    #[arg(long)]
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
    #[arg(long, value_name = "N")]
    pub lines: Option<String>,

    /// Window x offset.
    #[arg(long, value_name = "N")]
    pub x_offset: Option<String>,

    /// Window x offset counted from the right side of the screen.
    #[arg(long, value_name = "N")]
    pub right_x_offset: Option<String>,

    /// Window y offset (from bottom up with --bottom).
    #[arg(long, value_name = "N")]
    pub y_offset: Option<String>,

    /// Make instantmenu this wide.
    #[arg(long, value_name = "N")]
    pub width: Option<String>,

    /// Select monitor by index.
    #[arg(long, value_name = "N")]
    pub monitor: Option<String>,

    /// Prompt added to the left of the input field.
    #[arg(long, value_name = "TEXT")]
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

/// Rewrite the legacy single-dash flags to long options for clap.
fn map_legacy(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some((_, long)) = FLAGS.iter().find(|(s, _)| s == a) {
            out.push(long.to_string());
        } else if let Some((_, long)) = OPTS.iter().find(|(s, _)| s == a) {
            out.push(long.to_string());
            /* keep the value with the option (clap allows --opt value) */
            if let Some(v) = args.get(i + 1) {
                out.push(v.clone());
                i += 1;
            }
        } else {
            out.push(a.clone());
        }
        i += 1;
    }
    out
}

pub fn parse() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    /* -v is handled before clap so the output matches the C version */
    if raw.iter().any(|a| a == "-v") {
        println!("instantmenu-{}", crate::config::VERSION);
        std::process::exit(0);
    }
    /* try_parse_from expects argv[0] (the program name) as the first item */
    let mut argv = vec!["instantmenu".to_string()];
    argv.extend(map_legacy(&raw));
    match Args::try_parse_from(argv) {
        Ok(args) => args,
        Err(err) => err.exit(),
    }
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
