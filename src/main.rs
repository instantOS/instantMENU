//! Port of `main()` from instantmenu.c: argument layering (defaults → X
//! resources → command line), fontset creation, stdin/keyboard ordering and
//! driving the menu through setup and the event loop.

use clap::Parser;
use instantmenu::backend;
use instantmenu::cli;
use instantmenu::config::{Config, LineHeight, MonitorChoice, SlideSettings};
use instantmenu::enums::{ColorRole, ExitStatus, Scheme};
use instantmenu::menu::{self, Menu};
use instantmenu::render::Renderer;
use std::io::IsTerminal;

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args = cli::Args::parse();

    let mut cfg = Config::default();
    apply_flags(&args, &mut cfg);
    let overrides = apply_values(&args, &mut cfg);

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

    /* readxresources(): X resources are the base layer under the CLI */
    apply_resources(backend.as_ref(), &mut cfg);

    /* CLI font/colors override X resources */
    overrides.apply_to(&mut cfg);

    /* Startup ordering: see [`stdin_mode`]. Streaming is the default, so
     * the keyboard is grabbed before anything slow happens; the other modes
     * grab after their blocking load instead. */
    let mode = stdin_mode(&cfg);
    let streaming = matches!(mode, StdinMode::Stream);

    let grab = cfg.toast.is_none() && !cfg.no_grab;
    if streaming && grab {
        grab_keyboard(&mut backend);
    }

    /* Read candidates before constructing the font database so only fallback
     * fonts needed by the actual corpus have to be loaded. Streamed items
     * resolve their fonts lazily per batch instead (renderer.add_fallbacks).
     * When nothing streams, everything arrives here. */
    let preloaded = match mode {
        StdinMode::Stream => None,
        StdinMode::Skip => Some(Vec::new()),
        StdinMode::Load => Some(menu::read_stdin(&cfg)),
    };
    if !streaming && grab {
        grab_keyboard(&mut backend);
    }

    let mut required_chars = std::collections::HashSet::new();
    for item in preloaded.iter().flatten() {
        required_chars.extend(item.text.chars());
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
    let renderer = Renderer::new(&cfg.fonts, &cfg.colors, &required_chars);

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

    if let Some(status) = menu.setup() {
        status.exit();
    }

    let _nonblock_stdin = streaming.then(|| {
        menu.begin_stream(libc::STDIN_FILENO);
        NonBlockingStdin::new(libc::STDIN_FILENO)
    });

    let status = menu.run();
    drop(_nonblock_stdin); // restore the stdin flags before exiting
    status.exit();
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
fn grab_keyboard(backend: &mut Box<dyn backend::Backend>) {
    if let Err(e) = backend.grab_keyboard() {
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
    if args.menu.commented {
        cfg.commented = true;
        cfg.prompt = Some("prompts".to_string());
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
    if args.menu.instant {
        cfg.instant = true;
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

/// CLI font/color overrides, applied after X resources so the command line
/// wins (the C version's argument-layering order).
struct CliOverrides {
    font: Option<String>,
    colors: Vec<(Scheme, ColorRole, String)>,
}

impl CliOverrides {
    fn apply_to(self, cfg: &mut Config) {
        if let Some(f) = self.font {
            cfg.fonts[0] = f;
        }
        for (scheme, role, value) in self.colors {
            *cfg.colors[scheme as usize].role_mut(role) = value;
        }
    }
}

/// Value options, plus the temporary font/color overrides applied after X
/// resources.
fn apply_values(args: &cli::Args, cfg: &mut Config) -> CliOverrides {
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
    if let Some(a) = args.menu.animation {
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

    /* temporary font/colors: applied AFTER X resources so the CLI wins */
    let mut font: Option<String> = args.window.font.clone();
    if args.window.monospace {
        font = Some("Fira Code Nerd Font:pixelsize=15".to_string());
    }
    let mut colors = Vec::new();
    if let Some(c) = &args.window.normal_bg {
        colors.push((Scheme::Normal, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.window.normal_fg {
        colors.push((Scheme::Normal, ColorRole::Foreground, c.clone()));
    }
    if let Some(c) = &args.window.selected_bg {
        colors.push((Scheme::Selected, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.window.selected_fg {
        colors.push((Scheme::Selected, ColorRole::Foreground, c.clone()));
    }

    CliOverrides { font, colors }
}

/// Apply X resource "key -> value" pairs to the config.
fn apply_resources(backend: &dyn backend::Backend, cfg: &mut Config) {
    for (key, value) in backend.resource_pairs() {
        if key == "font" {
            cfg.fonts[0] = value;
            continue;
        }
        for scheme in Scheme::ALL {
            for role in ColorRole::ALL {
                if key == format!("{}.{}", scheme.x_resource_name(), role.x_res_name()) {
                    *cfg.colors[scheme as usize].role_mut(role) = value.clone();
                }
            }
        }
    }
}
