//! Port of `main()` from instantmenu.c: argument layering (defaults → X
//! resources → command line), fontset creation, stdin/keyboard ordering and
//! the width fallback for negative `-w`.

use instantmenu::backend;
use instantmenu::cli;
use instantmenu::config::Config;
use instantmenu::enums::{ColorRole, Scheme};
use instantmenu::menu::Menu;
use instantmenu::render::Renderer;
use clap::Parser;
use std::io::IsTerminal;

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args = cli::Args::parse();
    if args.version {
        println!("instantmenu-{}", instantmenu::config::VERSION);
        std::process::exit(0);
    }

    let mut cfg = Config::default();
    apply_flags(&args, &mut cfg);
    let (temp_font, color_temp) = apply_values(&args, &mut cfg);

    /* open backend (Wayland preferred when WAYLAND_DISPLAY is set) */
    let mut backend = backend::open(cfg.embed).unwrap_or_else(|e| {
        eprintln!("instantmenu: {e}");
        std::process::exit(1);
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
    let items = Menu::read_stdin(&mut cfg);
    if !fast && grab {
        backend.grab_keyboard();
    }

    let mut required_chars = std::collections::HashSet::new();
    for item in &items {
        required_chars.extend(item.text.chars());
    }
    for text in [
        args.initial_text.as_deref(),
        cfg.prompt.as_deref(),
        cfg.search_text.as_deref(),
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

    /* -it: seed the input before reading stdin, with rejectnomatch off */
    if let Some(t) = args.initial_text.clone() {
        menu.initial_text(&t);
    }
    menu.items = items;

    /* negative -w: use the wider of |width| and the computed item width */
    apply_negative_width(&mut menu);

    menu.setup();
    menu.run();
}

/// Boolean flags: applied before the value options they gate.
fn apply_flags(args: &cli::Args, cfg: &mut Config) {
    /* boolean flags, port of the argument loop in main() */
    if let Some(p) = args.position {
        cfg.position = p;
    }
    if args.reject_no_match {
        cfg.reject_no_match = true;
    }
    if args.commented {
        cfg.commented = true;
        cfg.prompt = Some("prompts".to_string());
    }
    if args.follow_cursor {
        cfg.follow_cursor = true;
    }
    if args.input_only {
        cfg.input_only = true;
    }
    if args.smart_case {
        cfg.smart_case = true;
    }
    if let Some(m) = args.match_mode {
        cfg.match_mode = m;
    }
    if args.pre_match {
        cfg.pre_match = true;
    }
    if args.full_height {
        cfg.full_height = true;
    }
    if args.insensitive {
        cfg.insensitive = true;
    }
    if args.instant {
        cfg.instant = true;
    }
    if args.password {
        cfg.password = true;
    }
    if args.no_grab {
        cfg.no_grab = true;
    }
    if args.alt_tab {
        cfg.alt_tab = true;
    }
    if args.managed {
        cfg.managed = true;
    }
    cfg.fast = args.fast;
}

/// Value options, plus the temporary font/color overrides applied after X
/// resources.
fn apply_values(args: &cli::Args, cfg: &mut Config) -> (Option<String>, Vec<(Scheme, ColorRole, String)>) {
    if let Some(v) = args.toast {
        cfg.toast = v;
    }
    if let Some(c) = args.columns {
        cfg.columns = c;
        if cfg.columns == 0 {
            cfg.columns = 1;
        }
        if args.lines.is_none() {
            cfg.lines = 1; /* C: -g sets lines=1 when unset (order-dependent) */
        }
    }
    if let Some(l) = args.lines {
        cfg.lines = l;
    }
    if let Some(x) = args.x_offset {
        cfg.x_offset = x;
    }
    if let Some(x) = args.right_x_offset {
        cfg.right_x_offset = true;
        cfg.x_offset = x;
    }
    if let Some(y) = args.y_offset {
        cfg.y_offset = y;
    }
    if let Some(w) = args.width {
        cfg.width = w;
    }
    if let Some(m) = args.monitor {
        cfg.monitor = m;
    }
    if let Some(p) = &args.prompt {
        cfg.prompt = Some(p.clone());
    }
    if let Some(q) = &args.search_text {
        cfg.search_text = Some(q.clone());
    }
    if let Some(a) = args.animation {
        cfg.frame_count = a;
        cfg.animated = true;
    }
    if let Some(bw) = args.border_width {
        cfg.border_width = bw;
    }
    if let Some(ps) = args.preselect {
        cfg.preselected = ps;
    }
    if let Some(h) = args.line_height {
        /* C: only applied when !fullheight, then clamped to >= 8 */
        if !cfg.full_height {
            cfg.line_height = h.max(8);
        } else {
            cfg.line_height = h;
        }
    }
    if let Some(w) = args.embed {
        cfg.embed = Some(w);
    }
    cfg.left_command = args.left_cmd.clone();
    cfg.right_command = args.right_cmd.clone();

    /* temporary font/colors: applied AFTER X resources so the CLI wins */
    let mut temp_font: Option<String> = args.font.clone();
    if args.monospace {
        temp_font = Some("Fira Code Nerd Font:pixelsize=15".to_string());
    }
    let mut color_temp: Vec<(Scheme, ColorRole, String)> = Vec::new();
    if let Some(c) = &args.normal_bg {
        color_temp.push((Scheme::Normal, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.normal_fg {
        color_temp.push((Scheme::Normal, ColorRole::Foreground, c.clone()));
    }
    if let Some(c) = &args.selected_bg {
        color_temp.push((Scheme::Selected, ColorRole::Background, c.clone()));
    }
    if let Some(c) = &args.selected_fg {
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

/// negative `-w`: use the wider of |width| and the computed item width.
fn apply_negative_width(menu: &mut Menu) {
    if menu.cfg.width <= -1 {
        const AUTO_WIDTH_WARNING_ITEMS: usize = 256;
        if menu.items.len() >= AUTO_WIDTH_WARNING_ITEMS {
            eprintln!(
                "instantmenu: warning: --width {} requires measuring all {} items; use a positive width for large lists",
                menu.cfg.width,
                menu.items.len()
            );
        }
        let prompt_text = menu.cfg.prompt.clone();
        let prompt_width = match &prompt_text {
            Some(p) => menu.text_width(p),
            None => 0,
        };
        let max_width = (menu.max_text_width() as f64 * 1.3 * menu.cfg.columns.max(1) as f64
            + prompt_width as f64) as i32;
        if -menu.cfg.width > max_width {
            menu.cfg.width = -menu.cfg.width;
        } else {
            menu.cfg.width = max_width;
        }
    }
}
