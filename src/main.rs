//! Port of `main()` from instantmenu.c: configuration, fontset creation,
//! stdin/keyboard ordering and driving the menu through setup and the event
//! loop.

use clap::Parser;
use instantmenu::appearance;
use instantmenu::backend;
use instantmenu::cli;
use instantmenu::config::{Config, LineHeight, MonitorChoice, SlideSettings};
use instantmenu::enums::ExitStatus;
use instantmenu::menu::{self, Menu};
use instantmenu::render::Renderer;
use std::io::{IsTerminal, Write};

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args = cli::Args::parse();

    if let Some(cli::Cmd::Icons(icons)) = &args.subcommand {
        print_icons(&icons.command);
        return;
    }

    let mut cfg = Config::default();
    apply_flags(&args, &mut cfg);
    apply_values(&args, &mut cfg);

    /* `slide` subcommand: validate the range and apply the slider defaults
     * before anything is opened */
    if let Some(slide) = cfg.slide.as_mut() {
        if let Err(e) = slide.resolve() {
            eprintln!("instantmenu: {e}");
            ExitStatus::Failure.exit();
        }
    }

    /* open backend (wayland by default when WAYLAND_DISPLAY is set;
     * --backend x11/wayland forces the choice) */
    let track_focused_monitor = cfg.monitor == MonitorChoice::Auto && !cfg.follow_cursor;
    let mut backend = backend::open(cfg.embed, args.window.backend, track_focused_monitor)
        .unwrap_or_else(|e| {
            eprintln!("instantmenu: {e}");
            ExitStatus::Failure.exit();
        });

    /* Startup ordering: see [`stdin_mode`]. Streaming is the default, so
     * the keyboard is grabbed before anything slow happens; the other modes
     * grab after their blocking load instead. */
    let mode = stdin_mode(&cfg);
    let streaming = matches!(mode, StdinMode::Stream);
    let can_acquire_early = !matches!(mode, StdinMode::Load);

    let grab = cfg.toast.is_none() && !cfg.no_grab;
    if can_acquire_early && grab {
        acquire_keyboard(&mut backend, &cfg);
    }

    /* Appearance is intentionally resolved after input acquisition. On
     * Wayland the backend has already mapped a transparent 1x1 exclusive
     * layer surface, so a cold or slow config disk cannot lose keystrokes.
     * CLI appearance values are applied last. */
    let config_path = if args.window.no_config {
        None
    } else {
        args.window.config.clone().or_else(|| {
            appearance::default_path(
                std::env::var_os("XDG_CONFIG_HOME"),
                std::env::var_os("HOME"),
            )
        })
    };
    let explicit_config = args.window.config.is_some();
    match appearance::load(config_path.as_deref(), explicit_config) {
        Ok(Some(value)) => value.apply(&mut cfg).unwrap_or_else(|error| {
            eprintln!("instantmenu: invalid appearance config: {error}");
            ExitStatus::Failure.exit();
        }),
        Ok(None) => {}
        Err(error) => {
            eprintln!("instantmenu: {error}");
            ExitStatus::Failure.exit();
        }
    }
    apply_appearance_values(&args, &mut cfg);

    /* Read candidates before constructing the font database so only fallback
     * fonts needed by the actual corpus have to be loaded. Streamed items
     * resolve their fonts lazily per batch instead (renderer.add_fallbacks).
     * When nothing streams, everything arrives here. */
    let preloaded = match mode {
        StdinMode::Stream => None,
        StdinMode::Skip => Some(Vec::new()),
        StdinMode::Load => Some(menu::read_stdin(&cfg)),
    };
    if !can_acquire_early && grab {
        acquire_keyboard(&mut backend, &cfg);
    }

    let mut required_chars = std::collections::HashSet::new();
    for item in preloaded.iter().flatten() {
        required_chars.extend(item.text.chars());
        // icon entries name their glyph; the resolved char is not in the text
        if let Some(icon) = item.entry.icon {
            required_chars.insert(icon);
        }
        if let Some(key) = item.entry.key {
            required_chars.insert(key);
        }
    }
    for text in [
        args.menu.initial_text.as_deref(),
        cfg.prompt.as_deref(),
        cfg.placeholder.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        required_chars.extend(text.chars());
    }

    /* drw_fontset_create + lrpad = drw->fonts->h */
    let renderer = Renderer::new(&cfg.fonts, cfg.palette, &required_chars);

    if cfg.full_height || cfg.line_height == LineHeight::FromFont {
        cfg.line_height = LineHeight::Pixels((renderer.font_height as f32 * 2.5) as i32);
    }

    /* (C has a prompt/dmw adjustment here, guarded by mw which is still 0 —
     * dead code, intentionally not ported) */

    let mut menu = Menu::new(cfg, renderer, backend);

    /* -it: seed the input before the items are loaded (the argv-loop order),
     * with rejectnomatch off */
    if let Some(t) = args.menu.initial_text.clone() {
        if let Some(status) = menu.initial_text(&t) {
            status.exit();
        }
    }
    if let Some(items) = preloaded {
        menu.add_items(items);
    }

    // Streaming used to do setup() with 0 items, which made --width auto
    // fall back to monitor.w (full width) and then shrink after the first
    // streamed batch — the wide flash seen in instantstartmenu (echo ... |
    // instantmenu -w auto -l 10 --position top-left). Preload whatever is
    // already buffered without blocking so the first layout measures the
    // final corpus when the producer is fast (echo). Geometry fallback
    // (content_width) bounds the flash for slow producers.
    let nonblock_stdin = if streaming {
        let nb = NonBlockingStdin::new(libc::STDIN_FILENO);
        menu.begin_stream(libc::STDIN_FILENO);
        menu.preload_available();
        Some(nb)
    } else {
        None
    };

    if let Some(status) = menu.setup() {
        drop(nonblock_stdin); // setup can finish before run(), so restore here too
        status.exit();
    }

    let status = menu.run();
    drop(nonblock_stdin); // restore the stdin flags before exiting
    status.exit();
}

/// Print the embedded icon catalog without initializing configuration, fonts,
/// stdin streaming, or a display backend. One record per line keeps the
/// output useful to shell tools while the literal glyph remains visible.
fn print_icons(command: &cli::IconsCmd) {
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    match command {
        cli::IconsCmd::List => {
            for icon in instantmenu::icons::catalog() {
                writeln!(
                    out,
                    "{}\t{}\tU+{:04X}",
                    icon.name, icon.glyph, icon.glyph as u32
                )
                .expect("write icon catalog");
            }
        }
        cli::IconsCmd::Search { query } => {
            for icon in instantmenu::icons::search(&query.join(" ")) {
                writeln!(
                    out,
                    "{}\t{}\tU+{:04X}",
                    icon.name, icon.glyph, icon.glyph as u32
                )
                .expect("write icon search results");
            }
        }
    }
}

/// O_NONBLOCK on the streaming stdin, restored on drop. The drain loop must
/// never block on its final read (the producer may stall between chunks),
/// and the flag lives on the open file description — so it is undone before
/// the process exits rather than left behind for whoever inherits stdin.
struct NonBlockingStdin {
    fd: i32,
    saved_flags: Option<i32>,
}

impl NonBlockingStdin {
    fn new(fd: i32) -> Self {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        let saved_flags = (flags >= 0).then_some(flags);
        if let Some(flags) = saved_flags {
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        }
        NonBlockingStdin { fd, saved_flags }
    }
}

impl Drop for NonBlockingStdin {
    fn drop(&mut self) {
        if let Some(flags) = self.saved_flags {
            unsafe { libc::fcntl(self.fd, libc::F_SETFL, flags) };
        }
    }
}

/// Grab the keyboard or die: the C version exited from inside
/// grabkeyboard() on failure, and without the grab the menu would leak
/// keystrokes to whatever had focus.
fn acquire_keyboard(backend: &mut Box<dyn backend::Backend>, cfg: &Config) {
    let monitor_rects: Vec<_> = backend
        .monitors()
        .iter()
        .map(|monitor| monitor.rect)
        .collect();
    /* Auto follows keyboard focus. When no monitor could be identified that
     * way (no activated window — e.g. an empty focused desktop — or one
     * spanning several outputs), None asks the compositor to place the menu
     * on its focused output rather than us guessing an index. */
    let output = match cfg.monitor {
        MonitorChoice::Index(index) if (index as usize) < monitor_rects.len() => {
            Some(index as usize)
        }
        MonitorChoice::Index(_) => Some(0),
        MonitorChoice::Auto if cfg.follow_cursor => backend
            .pointer_position()
            .and_then(|point| {
                monitor_rects
                    .iter()
                    .position(|rect| rect.contains_exclusive(point))
            }),
        MonitorChoice::Auto => backend
            .focused_monitor()
            .filter(|index| *index < monitor_rects.len()),
    };
    /* --embed is X11-only and Wayland intentionally ignores it; only an
     * xdg-shell managed window prevents layer-surface reuse. */
    let layer_menu = !cfg.managed;
    if let Err(e) = backend.acquire_keyboard(output, layer_menu) {
        eprintln!("instantmenu: {e}");
        ExitStatus::Failure.exit();
    }
}

/// How stdin feeds the menu.
enum StdinMode {
    /// Items arrive while the menu runs: stdin is switched to non-blocking
    /// and polled alongside the backend, batches coalescing into one
    /// rematch + redraw each.
    Stream,
    /// The whole corpus is read (blocking) before the menu opens. A tty
    /// stdin lands here too: the user types items interactively and ends
    /// with Ctrl-D, and grabbing the keyboard first would lock their
    /// terminal mid-typing.
    Load,
    /// Stdin is never read (password/input-only/slide).
    Skip,
}

/// The startup-ordering policy: streaming is the default (grab first, open
/// immediately, consume stdin while running); toast is a passive display
/// that still shows items passed on stdin; password/input-only/slide never
/// read stdin at all.
fn stdin_mode(cfg: &Config) -> StdinMode {
    if cfg.password || cfg.input_only || cfg.slide.is_some() {
        return StdinMode::Skip;
    }
    if cfg.toast.is_some() || std::io::stdin().is_terminal() {
        return StdinMode::Load;
    }
    StdinMode::Stream
}

/// Boolean flags: applied before the value options they gate.
fn apply_flags(args: &cli::Args, cfg: &mut Config) {
    /* boolean flags, port of the argument loop in main() */
    if let Some(p) = args.window.position {
        cfg.position = p;
    }
    if args.menu.reject_no_match {
        cfg.reject_no_match = true;
    }
    if args.menu.single_key {
        cfg.single_key = true;
    }
    if args.window.follow_cursor {
        cfg.follow_cursor = true;
    }
    if args.menu.input_only {
        cfg.input_only = true;
    }
    if args.menu.smart_case {
        cfg.smart_case = true;
    }
    if let Some(m) = args.menu.match_mode {
        cfg.match_mode = m;
    }
    if args.menu.pre_match {
        cfg.pre_match = true;
    }
    if args.menu.space_confirm {
        cfg.space_confirm = true;
    }
    if args.menu.full_height {
        cfg.full_height = true;
    }
    if args.menu.insensitive {
        cfg.insensitive = true;
    }
    if args.menu.auto_confirm {
        cfg.auto_confirm = true;
    }
    if args.menu.password {
        cfg.password = true;
    }
    if args.window.no_grab {
        cfg.no_grab = true;
    }
    if args.window.no_outside_close {
        cfg.outside_close = false;
    }
    if args.menu.alt_tab {
        cfg.alt_tab = true;
    }
    if args.window.managed {
        cfg.managed = true;
    }
}

/// Apply value options to the runtime configuration.
fn apply_values(args: &cli::Args, cfg: &mut Config) {
    if let Some(v) = args.menu.toast {
        /* 0 keeps the timeout disabled, like omitting the option */
        cfg.toast = (v > 0.0).then_some(v);
    }
    if let Some(c) = args.menu.columns {
        cfg.columns = c;
        if args.menu.lines.is_none() {
            cfg.lines = 1; /* -g sets lines=1 when unset (order-dependent) */
        }
    }
    if let Some(l) = args.menu.lines {
        cfg.lines = l;
    }
    if let Some(x) = args.window.x_offset {
        cfg.x_offset = x;
    }
    if let Some(y) = args.window.y_offset {
        cfg.y_offset = y;
    }
    if let Some(w) = args.window.width {
        cfg.width = w;
    }
    if let Some(m) = args.window.monitor {
        cfg.monitor = m;
    }
    if let Some(p) = &args.window.prompt {
        cfg.prompt = Some(p.clone());
    }
    if let Some(q) = &args.menu.placeholder {
        cfg.placeholder = Some(q.clone());
    }
    if let Some(f) = &args.menu.frecency_cache {
        /* IDs resolve under the XDG cache dir; absolute paths pass through */
        cfg.frecency_cache = Some(match menu::resolve_cache_path(f) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("instantmenu: {e}");
                ExitStatus::Failure.exit();
            }
        });
    }
    if let Some(a) = args.menu.animation_length {
        cfg.frame_count = a;
        cfg.animated = true;
    }
    if let Some(bw) = args.window.border_width {
        cfg.border_width = bw;
    }
    if let Some(ps) = args.menu.preselect {
        cfg.preselected = ps;
    }
    if let Some(h) = args.window.line_height {
        /* clamped to >= 8; full_height resolves the height anyway */
        cfg.line_height = match h {
            LineHeight::Pixels(n) if !cfg.full_height => LineHeight::Pixels(n.max(8)),
            other => other,
        };
    }
    if let Some(w) = args.window.embed {
        cfg.embed = Some(w);
    }
    cfg.left_command = args.menu.left_command.clone();
    cfg.right_command = args.menu.right_command.clone();
    if let Some(cli::Cmd::Slide(s)) = &args.subcommand {
        cfg.slide = Some(SlideSettings {
            min: s.min,
            max: s.max,
            value: s.value,
            step: s.step,
            big_step: s.big_step,
            command: s.resolved_command(),
        });
    }
}

/// Appearance precedence is built-in defaults, config file, selected CLI
/// theme, then individual CLI values.
fn apply_appearance_values(args: &cli::Args, cfg: &mut Config) {
    let mut font = args.window.font.clone();
    if args.window.monospace {
        font = Some("Fira Code Nerd Font:pixelsize=15".to_string());
    }
    if let Some(font) = font {
        cfg.fonts[0] = font;
    }

    if let Some(theme) = args.window.theme {
        cfg.palette = theme.palette();
    }
    if let Some(color) = args.window.normal_bg {
        cfg.palette.normal.bg = color;
    }
    if let Some(color) = args.window.normal_fg {
        cfg.palette.normal.fg = color;
    }
    if let Some(color) = args.window.selected_bg {
        cfg.palette.selected.bg = color;
    }
    if let Some(color) = args.window.selected_fg {
        cfg.palette.selected.fg = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_colors_override_the_selected_theme() {
        let args = cli::Args::try_parse_from([
            "instantmenu",
            "--theme",
            "gruvbox",
            "--normal-bg",
            "#010203",
        ])
        .unwrap();
        let mut cfg = Config::default();
        apply_appearance_values(&args, &mut cfg);

        assert_eq!(
            cfg.palette.normal.bg,
            instantmenu::render::Color::hex(0x010203)
        );
        assert_eq!(
            cfg.palette.normal.fg,
            instantmenu::render::Color::hex(0xEBDBB2)
        );
        assert_eq!(
            cfg.palette.selected.bg,
            instantmenu::render::Color::hex(0x83A598)
        );
    }
}
