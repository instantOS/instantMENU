//! Port of `main()` from instantmenu.c: argument layering (defaults → X
//! resources → command line), fontset creation, stdin/keyboard ordering and
//! the width fallback for negative `-w`.

use instantmenu::backend;
use instantmenu::cli;
use instantmenu::config::{Config, XRES_COLOR_TYPES};
use instantmenu::enums::{Scheme, COL_BG, COL_FG};
use instantmenu::menu::Menu;
use instantmenu::render::Renderer;

fn usage() -> ! {
    eprintln!(
        "usage: instantmenu [-bfirnPv] [-l lines] [-g columns] [-p prompt] [-m monitor]\n\
         \x20            [-x xoffset] [-xr right xoffset] [-y yoffset] [-w width]\n\
         \x20            [-h height] [-fn font]\n\
         \x20            [-nb color] [-nf color] [-sb color] [-sf color] [-W windowid]"
    );
    std::process::exit(1);
}

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args = cli::parse();
    if args.help {
        usage();
    }
    if args.version {
        println!("instantmenu-{}", instantmenu::config::VERSION);
        std::process::exit(0);
    }

    let mut cfg = Config::default();

    /* boolean flags, port of the argument loop in main() */
    if args.bottom {
        cfg.topbar = false;
    }
    if args.reject_no_match {
        cfg.rejectnomatch = true;
    }
    if args.commented {
        cfg.commented = true;
        cfg.prompt = Some("prompts".to_string());
    }
    if args.centered {
        cfg.centered = true;
    }
    if args.follow_cursor {
        cfg.followcursor = true;
    }
    if args.space_confirm {
        cfg.spaceconfirm = true;
    }
    if args.input_only {
        cfg.inputonly = true;
    }
    if args.smart_case {
        cfg.smartcase = true;
    }
    if args.no_fuzzy {
        cfg.fuzzy = false;
    }
    if args.pre_match {
        cfg.prematch = true;
    }
    if args.exact {
        cfg.exact = true;
        cfg.fuzzy = false;
    }
    if args.full_height {
        cfg.fullheight = true;
    }
    if args.insensitive {
        cfg.insensitive = true;
    }
    if args.instant {
        cfg.instant = true;
    }
    if args.password {
        cfg.passwd = true;
    }
    if args.no_grab {
        cfg.nograb = true;
    }
    if args.alt_tab {
        cfg.alttab = true;
    }
    if args.managed {
        cfg.managed = true;
    }
    cfg.fast = args.fast;

    /* value options (atoi semantics) */
    if let Some(v) = &args.toast {
        cfg.toast = cli::atoi(v);
    }
    if let Some(c) = &args.columns {
        cfg.columns = cli::atoi(c);
        if cfg.columns == 0 {
            cfg.columns = 1;
        }
        if args.lines.is_none() {
            cfg.lines = 1; /* C: -g sets lines=1 when unset (order-dependent) */
        }
    }
    if let Some(l) = &args.lines {
        cfg.lines = cli::atoi(l);
    }
    if let Some(x) = &args.x_offset {
        cfg.dmx = cli::atoi(x);
    }
    if let Some(x) = &args.right_x_offset {
        cfg.rightxoffset = true;
        cfg.dmx = cli::atoi(x);
    }
    if let Some(y) = &args.y_offset {
        cfg.dmy = cli::atoi(y);
    }
    if let Some(w) = &args.width {
        cfg.dmw = cli::atoi(w);
    }
    if let Some(m) = &args.monitor {
        cfg.mon = cli::atoi(m);
    }
    if let Some(p) = &args.prompt {
        cfg.prompt = Some(p.clone());
    }
    if let Some(q) = &args.search_text {
        cfg.searchtext = Some(q.clone());
    }
    if let Some(a) = &args.animation {
        cfg.framecount = cli::atoi(a);
    }
    if let Some(bw) = &args.border_width {
        cfg.border_width = cli::atoi(bw);
    }
    if let Some(ps) = &args.preselect {
        /* C strips one leading '-' from the value */
        let v = ps.strip_prefix('-').unwrap_or(ps);
        cfg.preselected = cli::atoi(v);
    }
    if let Some(h) = &args.line_height {
        /* C: only applied when !fullheight, then clamped to >= 8 */
        if !cfg.fullheight {
            cfg.lineheight = cli::atoi(h).max(8);
        } else {
            cfg.lineheight = cli::atoi(h);
        }
    }
    if let Some(w) = &args.embed {
        cfg.embed = Some(cli::strtol0(w));
    }
    cfg.leftcmd = args.left_cmd.clone();
    cfg.rightcmd = args.right_cmd.clone();

    /* temporary font/colors: applied AFTER X resources so the CLI wins */
    let mut tempfont: Option<String> = args.font.clone();
    if args.monospace {
        tempfont = Some("Fira Code Nerd Font:pixelsize=15".to_string());
    }
    let mut colortemp: Vec<(Scheme, usize, String)> = Vec::new();
    if let Some(c) = &args.normal_bg {
        colortemp.push((Scheme::Norm, COL_BG, c.clone()));
    }
    if let Some(c) = &args.normal_fg {
        colortemp.push((Scheme::Norm, COL_FG, c.clone()));
    }
    if let Some(c) = &args.selected_bg {
        colortemp.push((Scheme::Sel, COL_BG, c.clone()));
    }
    if let Some(c) = &args.selected_fg {
        colortemp.push((Scheme::Sel, COL_FG, c.clone()));
    }

    /* open backend (Wayland preferred when WAYLAND_DISPLAY is set) */
    let backend = backend::open(cfg.embed).unwrap_or_else(|e| {
        eprintln!("instantmenu: {e}");
        std::process::exit(1);
    });

    /* readxresources(): X resources are the base layer under the CLI */
    for (key, value) in backend.resource_pairs() {
        if key == "font" {
            cfg.fonts[0] = value;
            continue;
        }
        for scheme in Scheme::ALL {
            for (col, ctype) in XRES_COLOR_TYPES.iter().enumerate() {
                if key == format!("{}.{}", scheme.xres_name(), ctype) {
                    cfg.colors[scheme as usize][col] = value.clone();
                }
            }
        }
    }

    /* CLI font/colors override X resources */
    if let Some(f) = tempfont {
        cfg.fonts[0] = f;
    }
    for (scheme, col, value) in colortemp {
        cfg.colors[scheme as usize][col] = value;
    }

    /* drw_fontset_create + lrpad = drw->fonts->h */
    let renderer = Renderer::new(&cfg.fonts, &cfg.colors);

    if cfg.fullheight || cfg.lineheight == -1 {
        cfg.lineheight = (renderer.font_height as f32 * 2.5) as i32;
    }

    /* (C has a prompt/dmw adjustment here, guarded by mw which is still 0 —
     * dead code, intentionally not ported) */

    let mut menu = Menu::new(cfg, renderer, backend);

    /* -it: seed the input before reading stdin, with rejectnomatch off */
    if let Some(t) = args.initial_text.clone() {
        menu.initial_text(&t);
    }

    /* fast && !isatty(0): grab before reading stdin so the menu is snappy on
     * slow stdin producers */
    let grab = menu.cfg.toast == 0 && !menu.cfg.nograb;
    let fast = menu.cfg.fast && unsafe { libc::isatty(0) } == 0;
    if fast {
        if grab {
            menu.backend.grab_keyboard();
        }
        menu.readstdin();
    } else {
        menu.readstdin();
        if grab {
            menu.backend.grab_keyboard();
        }
    }

    /* negative -w: use the wider of |dmw| and the computed item width */
    if menu.cfg.dmw <= -1 {
        let prompt_text = menu.cfg.prompt.clone();
        let promptw = match &prompt_text {
            Some(p) => menu.textw(p),
            None => 0,
        };
        let maxw = (menu.max_textw() as f64 * 1.3 * menu.cfg.columns.max(1) as f64
            + promptw as f64) as i32;
        if -menu.cfg.dmw > maxw {
            menu.cfg.dmw = -menu.cfg.dmw;
        } else {
            menu.cfg.dmw = maxw;
        }
    }

    menu.setup();
    menu.run();
}
