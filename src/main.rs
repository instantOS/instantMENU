//! Port of `main()` from instantmenu.c: argument layering (defaults → X
//! resources → command line), fontset creation, stdin/keyboard ordering and
//! driving the menu through setup and the event loop.

use clap::Parser;
use instantmenu::backend;
use instantmenu::cli;
use instantmenu::config::{Config, SlideSettings};
use instantmenu::enums::{ColorRole, ExitStatus, Scheme};
use instantmenu::menu::{self, Menu};
use instantmenu::render::Renderer;
use std::io::IsTerminal;

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args = cli::Args::parse();

    /* menu-only options are meaningless in slide mode; reject them before
     * anything is opened (clap cannot express this per-subcommand) */
    if let Some(flag) = args.menu_only_option_in_subcommand() {
        eprintln!("instantmenu: {flag} cannot be used with `slide`");
        ExitStatus::Failure.exit();
    }

    let mut cfg = Config::default();
    apply_flags(&args, &mut cfg);
    let (temp_font, color_temp) = apply_values(&args, &mut cfg);

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
    let mut backend = backend::open(cfg.embed, args.window.backend).unwrap_or_else(|e| {
        eprintln!("instantmenu: {e}");
        ExitStatus::Failure.exit();
    });

    /* readxresources(): X resources are the base layer under the CLI */
    apply_resources(backend.as_ref(), &mut cfg);

    /* CLI font/colors override X resources */
    if let Some(f) = temp_font {
        cfg.fonts[0] = f;
    }
    for (scheme, role, value) in color_temp {
        *cfg.colors[scheme as usize].role_mut(role) = value;
    }

    /* Read candidates before constructing the font database so only fallback
     * fonts needed by the actual corpus have to be loaded. */
    let grab = cfg.toast == 0 && !cfg.no_grab;
    let fast = cfg.fast && !std::io::stdin().is_terminal();
    if fast && grab {
        backend.grab_keyboard();
    }
    let stdin = menu::read_stdin(&cfg);
    if !fast && grab {
        backend.grab_keyboard();
    }

    let mut required_chars = std::collections::HashSet::new();
    for item in &stdin.items {
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

    if cfg.full_height || cfg.line_height == -1 {
        cfg.line_height = (renderer.font_height as f32 * 2.5) as i32;
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
    menu.load_items(stdin);

    if let Some(status) = menu.setup() {
        status.exit();
    }
    menu.run().exit();
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
    if args.menu.alt_tab {
        cfg.alt_tab = true;
    }
    if args.window.managed {
        cfg.managed = true;
    }
    cfg.fast = args.menu.fast;
}

/// Value options, plus the temporary font/color overrides applied after X
/// resources.
fn apply_values(
    args: &cli::Args,
    cfg: &mut Config,
) -> (Option<String>, Vec<(Scheme, ColorRole, String)>) {
    if let Some(v) = args.menu.toast {
        cfg.toast = v;
    }
    if let Some(c) = args.menu.columns {
        cfg.columns = c;
        if cfg.columns == 0 {
            cfg.columns = 1;
        }
        if args.menu.lines.is_none() {
            cfg.lines = 1; /* C: -g sets lines=1 when unset (order-dependent) */
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
        /* C: only applied when !fullheight, then clamped to >= 8 */
        if !cfg.full_height {
            cfg.line_height = h.max(8);
        } else {
            cfg.line_height = h;
        }
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
    let mut temp_font: Option<String> = args.window.font.clone();
    if args.window.monospace {
        temp_font = Some("Fira Code Nerd Font:pixelsize=15".to_string());
    }
    let mut color_temp: Vec<(Scheme, ColorRole, String)> = Vec::new();
    if let Some(c) = &args.window.normal_bg {
        color_temp.push((Scheme::Normal, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.window.normal_fg {
        color_temp.push((Scheme::Normal, ColorRole::Foreground, c.clone()));
    }
    if let Some(c) = &args.window.selected_bg {
        color_temp.push((Scheme::Selected, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.window.selected_fg {
        color_temp.push((Scheme::Selected, ColorRole::Foreground, c.clone()));
    }

    (temp_font, color_temp)
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
