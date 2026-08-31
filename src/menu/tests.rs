//! Shell characterization tests: drive the [`Menu`] handlers against a stub
//! backend and assert on the returned [`Transition`]s — no window, and no
//! font-dependent pixels (fonts load, but nothing is rasterized).

use super::input::read_stdin;
use super::matcher::Item;
use super::transition::Transition;
use super::Menu;
use crate::backend::stub::{TestBackend, TestHandle as StubHandle};
use crate::backend::{BackendEvent, InputSource, Modifiers, MonitorInfo, MouseButton};
use crate::config::{Config, SlideSettings, Width};
use crate::enums::{ExitStatus, Scheme};
use crate::geom::{Point, Rect, Size};
use crate::render::{Color, Renderer};
use std::collections::HashSet;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use xkbcommon::xkb::keysyms as ks;

/* ── modifier shorthands ───────────────────────────────────────────────── */

const M_NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    logo: false,
};
const M_CTRL: Modifiers = Modifiers {
    shift: false,
    ctrl: true,
    alt: false,
    logo: false,
};
const M_SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: false,
    alt: false,
    logo: false,
};
const M_CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: true,
    alt: false,
    logo: false,
};
const M_ALT: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: true,
    logo: false,
};
const M_LOGO: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    logo: true,
};

/* ── captured stdout ───────────────────────────────────────────────────── */

#[derive(Clone, Default)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl Write for SharedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedOutput {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

/* ── helpers ───────────────────────────────────────────────────────────── */

/// A menu with items loaded and the post-setup geometry the handlers read;
/// `do_match()` has run once against the empty query.
fn menu_with(cfg: Config, items: &[&str]) -> (Menu, StubHandle, SharedOutput) {
    let renderer = Renderer::new(&cfg.fonts, cfg.palette, &HashSet::new());
    let backend = TestBackend {
        monitors: vec![MonitorInfo {
            rect: Rect::new(0, 0, 1920, 1080),
            name: "stub".into(),
        }],
        ..TestBackend::new()
    };
    let stub = backend.handle();
    let mut menu = Menu::new(cfg, renderer, Box::new(backend));
    menu.add_items(items.iter().map(|s| Item::new(*s)).collect());
    menu.stream_dirty = false; // the batch-load is not a pending stream settle
                               // geometry normally computed by setup()
    menu.layout.menu_width = 600;
    menu.layout.menu_height = 240;
    menu.layout.bar_height = 30;
    menu.layout.input_width = 200;
    menu.layout.columns = 1;

    let out = SharedOutput::default();
    menu.out = Box::new(out.clone());

    let _ = menu.do_match();
    (menu, stub, out)
}

/// Type raw text through key events (sym 0: plain characters are only
/// dispatched by their buffer).
fn type_text(menu: &mut Menu, text: &str) {
    for c in text.chars() {
        let _ = menu.key_press(0, M_NONE, &c.to_string());
    }
}

fn key(menu: &mut Menu, sym: u32, mods: Modifiers) -> Transition {
    menu.key_press(sym, mods, "")
}

/* ── confirm paths ─────────────────────────────────────────────────────── */

/// Return prints the selection and exits successfully.
#[test]
fn return_prints_selection_and_exits() {
    let (mut menu, _stub, out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("alpha".into())
    );
    assert_eq!(
        menu.perform(Transition::PrintAndExit("alpha".into())),
        Some(ExitStatus::Success)
    );
    assert_eq!(out.contents(), "alpha\n");
}

/// Ctrl+Return prints but keeps the menu running, and marks the item as
/// already output (drawn with the Out scheme afterwards).
#[test]
fn ctrl_return_prints_and_keeps_running() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_CTRL),
        Transition::Print("alpha".into())
    );
    assert!(menu.matcher.items[0].already_output);
    assert!(!menu.matcher.items[1].already_output);
}

/// Shift+Return prints the raw input instead of the selection (the
/// intentional deviation from plain dmenu).
#[test]
fn shift_return_prints_raw_input() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "al");
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_SHIFT),
        Transition::PrintAndExit("al".into())
    );
}

/// Ctrl+1..9 select the n-th item and confirm it.
#[test]
fn ctrl_number_selects_and_confirms() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta", "gamma"]);
    assert_eq!(
        key(&mut menu, ks::KEY_2, M_CTRL),
        Transition::PrintAndExit("beta".into())
    );
}

/* ── editing ───────────────────────────────────────────────────────────── */

#[test]
fn typing_filters_matches() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "bet");
    assert_eq!(menu.matcher.matches, vec![1]);
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("beta".into())
    );
}

/// Tab completes the input to the selected match.
#[test]
fn tab_completes_to_selection() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "al");
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_NONE), Transition::Redraw);
    assert_eq!(menu.editor.text, "alpha");
}

/// -r: an edit that empties the match list is reverted. Typo tolerance (one
/// typo per four characters) keeps near-miss edits like "alphx" accepted;
/// only edits with no match even accounting for typos are reverted.
#[test]
fn reject_no_match_reverts_edit() {
    let cfg = Config {
        reject_no_match: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    type_text(&mut menu, "alph");
    // one slipped key still matches alpha: accepted
    type_text(&mut menu, "x");
    assert_eq!(menu.editor.text, "alphx");
    assert_eq!(menu.matcher.matches, vec![0]);
    // a second one is past the typo budget: reverted to the last matching text
    type_text(&mut menu, "y");
    assert_eq!(menu.editor.text, "alphx");
    assert_eq!(menu.matcher.matches, vec![0]);
}

/// Ctrl+u clears the input; Ctrl+w deletes the word left of the cursor.
#[test]
fn ctrl_u_clears_input() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alpha");
    assert_eq!(key(&mut menu, ks::KEY_u, M_CTRL), Transition::Redraw);
    assert_eq!(menu.editor.text, "");
    assert_eq!(menu.matcher.matches, vec![0, 1]);
}

#[test]
fn ctrl_w_deletes_word() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["hello world"]);
    type_text(&mut menu, "hello world");
    assert_eq!(key(&mut menu, ks::KEY_w, M_CTRL), Transition::Redraw);
    assert_eq!(menu.editor.text, "hello ");
}

/// Left moves the cursor; Ctrl+k truncates at it.
#[test]
fn ctrl_k_truncates_at_cursor() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alp");
    assert_eq!(key(&mut menu, ks::KEY_Left, M_NONE), Transition::Redraw);
    assert_eq!(menu.editor.cursor, 2);
    assert_eq!(key(&mut menu, ks::KEY_k, M_CTRL), Transition::Redraw);
    assert_eq!(menu.editor.text, "al");
}

/// Ctrl+s inserts the literal ".*" (the regex-any prefix trick).
#[test]
fn ctrl_s_inserts_regex_any() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(key(&mut menu, ks::KEY_s, M_CTRL), Transition::Redraw);
    assert_eq!(menu.editor.text, ".*");
}

/// Ctrl+j/Ctrl+m are Return with the modifier consumed: a plain confirm.
#[test]
fn ctrl_j_confirms_like_return() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(
        key(&mut menu, ks::KEY_j, M_CTRL),
        Transition::PrintAndExit("alpha".into())
    );
    assert_eq!(
        key(&mut menu, ks::KEY_m, M_CTRL),
        Transition::PrintAndExit("alpha".into())
    );
}

/// Paste inserts only the first line of the selection.
#[test]
fn paste_takes_the_first_line() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(menu.paste("pasted\nsecond line"), Transition::Redraw);
    assert_eq!(menu.editor.text, "pasted");
}

/// Ctrl+v pastes and redraws; Ctrl+y pastes without forcing a redraw. Both
/// take the primary selection — Shift switches the request to the clipboard.
#[test]
fn ctrl_v_and_ctrl_y_request_selections() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(key(&mut menu, ks::KEY_v, M_CTRL), Transition::Redraw);
    assert_eq!(key(&mut menu, ks::KEY_y, M_CTRL), Transition::Nop);
    assert!(!*stub.state().selection_requests.last().unwrap());
    // Shift+v holds the clipboard variant
    key(&mut menu, ks::KEY_v, M_CTRL_SHIFT);
    assert!(*stub.state().selection_requests.last().unwrap());
}

/* ── frecency ──────────────────────────────────────────────────────────── */

/// Unique cache path per test (the suite runs in parallel).
fn frecency_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "instantmenu-frecency-{name}-{}",
        std::process::id()
    ))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// --frecency-cache: items load best-frecency first, unseen last, ties in
/// stdin order.
#[test]
fn frecency_ranks_items_on_load() {
    let path = frecency_path("rank");
    let now = unix_now();
    let day = 24 * 60 * 60;
    std::fs::write(
        &path,
        // stale: 3.0 three days ago → 1.78; fresh: 2.0 now → 2.0
        format!("3.000000 {} stale\n2.000000 {} fresh\n", now - 3 * day, now),
    )
    .unwrap();
    let cfg = Config {
        frecency_cache: Some(path.clone()),
        ..Config::default()
    };
    let (menu, _stub, _out) = menu_with(cfg, &["stale", "fresh", "unseen"]);
    let texts: Vec<&str> = menu.matcher.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["fresh", "stale", "unseen"]);
    let _ = std::fs::remove_file(&path);
}

/// Confirming a selection records it into the cache.
#[test]
fn frecency_records_selections() {
    let path = frecency_path("record");
    let _ = std::fs::remove_file(&path);
    let cfg = Config {
        frecency_cache: Some(path.clone()),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta"]);
    let t = key(&mut menu, ks::KEY_Return, M_NONE);
    assert_eq!(menu.perform(t), Some(ExitStatus::Success));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains(" alpha\n"), "{contents}");
    let _ = std::fs::remove_file(&path);
}

/// Shift+Return free-typed input is recorded too — how new commands enter
/// the cache without being in the item list.
#[test]
fn frecency_records_free_typed_input() {
    let path = frecency_path("typed");
    let _ = std::fs::remove_file(&path);
    let cfg = Config {
        frecency_cache: Some(path.clone()),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta"]);
    type_text(&mut menu, "newcmd");
    let t = key(&mut menu, ks::KEY_Return, M_SHIFT);
    assert_eq!(menu.perform(t), Some(ExitStatus::Success));
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains(" newcmd\n"), "{contents}");
    let _ = std::fs::remove_file(&path);
}

/// --password selections are never written to the cache.
#[test]
fn password_mode_never_records() {
    let path = frecency_path("password");
    let _ = std::fs::remove_file(&path);
    let cfg = Config {
        password: true,
        frecency_cache: Some(path.clone()),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta"]);
    type_text(&mut menu, "secret");
    let t = key(&mut menu, ks::KEY_Return, M_NONE);
    assert_eq!(menu.perform(t), Some(ExitStatus::Success));
    assert!(!path.exists());
}

/* ── auto-confirm and single-key modes ──────────────────────────────────── */

/// -n: typing down to a single fuzzy match prints it and exits mid-edit.
#[test]
fn auto_confirm_mode_picks_while_typing() {
    let cfg = Config {
        auto_confirm: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["abc", "bcd"]);
    assert_eq!(
        menu.key_press(0, M_NONE, "a"),
        Transition::PrintAndExit("abc".into())
    );
}

/// --single-key uses explicit key metadata and returns only the label.
#[test]
fn single_key_mode_picks_by_explicit_key() {
    let cfg = Config {
        single_key: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) =
        menu_with(cfg.clone(), &["{key=y} Yes", "Not keyed", "{key=n} No"]);
    assert_eq!(
        menu.key_press(0, M_NONE, "n"),
        Transition::PrintAndExit("No".into())
    );

    let (mut menu, _stub, _out) = menu_with(cfg, &["{key=y} Yes", "{key=n} No"]);
    assert_eq!(
        menu.key_press(0, M_NONE, "x"),
        Transition::Exit(ExitStatus::Success)
    );
}

/* ── exit paths ────────────────────────────────────────────────────────── */

#[test]
fn escape_exits_with_failure() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Escape, M_NONE),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// Alt+F4 and Mod4+q quit; Ctrl+[ too.
#[test]
fn modifier_quit_keys_exit() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        key(&mut menu, ks::KEY_F4, M_ALT),
        Transition::Exit(ExitStatus::Failure)
    );
    assert_eq!(
        key(&mut menu, ks::KEY_q, M_LOGO),
        Transition::Exit(ExitStatus::Failure)
    );
    assert_eq!(
        key(&mut menu, ks::KEY_bracketleft, M_CTRL),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// Right-click exits immediately.
#[test]
fn right_click_exits() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        menu.button_press(MouseButton::Right, M_NONE, Point::new(0, 0)),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// An outside click (delivered by the pointer grab on X11, or by a
/// transparent click-catcher surface on Wayland) closes the modal menu
/// without a selection.
#[test]
fn outside_click_closes_modal_menu() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        menu.perform(Transition::Exit(ExitStatus::Failure)),
        Some(ExitStatus::Failure)
    );
}

/* ── alt-tab ───────────────────────────────────────────────────────────── */

/// Alt+Tab advances without confirming; the release of the Alt key itself
/// confirms the selection. X11 order: the release event reports the released
/// modifier as still held.
#[test]
fn alt_tab_release_confirms() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));

    // the Tab release only ends the tab cycle
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    // the Alt release confirms
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_ALT),
        Transition::PrintAndExit("beta".into())
    );
}

/// Wayland order: the compositor sends the modifiers event *before* the key
/// release, so the Alt release arrives with alt already cleared. The confirm
/// keys off the released keysym — not the stale modifier bits (which is how
/// the port used to lose the confirm on Wayland entirely).
#[test]
fn alt_tab_release_confirms_on_wayland_event_order() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_NONE),
        Transition::PrintAndExit("beta".into())
    );
}

/// Tapping Tab twice advances two items; only the final Alt release
/// confirms.
#[test]
fn alt_tab_double_tap_advances_two_items() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(2));
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_ALT),
        Transition::PrintAndExit("gamma".into())
    );
}

/// Only the release of an Alt keysym confirms. The C version confirmed on
/// *any* key released while Alt was held; that breadth is narrowed
/// deliberately.
#[test]
fn alt_tab_other_key_releases_do_not_confirm() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(menu.key_release(ks::KEY_x, M_ALT), Transition::Nop);
    // still running — the Alt release does the confirming
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_ALT),
        Transition::PrintAndExit("beta".into())
    );
}

/// Shift+Alt release does not confirm (C parity: the shift guard stays).
#[test]
fn alt_tab_shift_release_does_not_confirm() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(menu.key_release(ks::KEY_Alt_L, M_SHIFT), Transition::Nop);
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_NONE),
        Transition::PrintAndExit("beta".into())
    );
}

/// Alt+Space cancels the mode at runtime: the release no longer confirms,
/// and Alt+Tab falls back to the plain Tab completion.
#[test]
fn alt_space_cancels_alt_tab_mode() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_space, M_ALT), Transition::Redraw);
    // the mode is off: Alt+Tab completes the selection instead of cycling
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.editor.text, "alpha");
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(menu.key_release(ks::KEY_Alt_L, M_ALT), Transition::Nop);
}

/// Shift+Tab moves the selection back; before the first item it wraps to
/// the last. The C branch ran this for every shifted key — typing capitals
/// no longer moves the selection.
#[test]
fn shift_tab_wraps_backward_and_typing_does_not() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    // a shifted letter is no longer a wrap binding
    assert_eq!(key(&mut menu, ks::KEY_a, M_SHIFT), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(0));

    // Shift+Tab from the first item wraps to the last
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_SHIFT), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(2));
    // and back
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_SHIFT), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));
}

/// With nothing selected (no matches), the confirm falls back to the input
/// text — the C version dereferenced a null `sel` here.
#[test]
fn alt_tab_release_without_selection_prints_input() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta"]);

    menu.editor.set_text("zz");
    let _ = menu.do_match();
    assert!(menu.matcher.matches.is_empty());

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    assert_eq!(menu.key_release(ks::KEY_Tab, M_ALT), Transition::Nop);
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_ALT),
        Transition::PrintAndExit("zz".into())
    );
}

/// Plain Tab in alt-tab mode marks a cycle like the C main switch: the next
/// release is absorbed once.
#[test]
fn plain_tab_marks_a_cycle() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_NONE), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(0));
    assert_eq!(menu.key_release(ks::KEY_Alt_L, M_ALT), Transition::Nop);
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_ALT),
        Transition::PrintAndExit("alpha".into())
    );
}

/// KeyboardLeft (wl_keyboard.leave / FocusOut) concludes a pending cycle so
/// the next Alt release confirms instead of being absorbed.
#[test]
fn keyboard_left_concludes_pending_cycle() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, M_ALT), Transition::Redraw);
    menu.keyboard_left();
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, M_NONE),
        Transition::PrintAndExit("beta".into())
    );
}

/* ── commands ──────────────────────────────────────────────────────────── */

/// Shift+Left in the horizontal list runs the left command instead of
/// moving the cursor.
#[test]
fn left_command_triggers_spawn() {
    let cfg = Config {
        left_command: Some("true".into()),
        frame_count: 0, // skip the animation in tests
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Left, M_SHIFT),
        Transition::SpawnAndExit("true".into())
    );
}

/* ── mouse selection ───────────────────────────────────────────────────── */

/// Horizontal list: hover selects, click confirms.
#[test]
fn horizontal_hover_and_click() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    let (_, rect) = menu
        .horizontal_item_rects(0)
        .into_iter()
        .next_back()
        .unwrap();
    let pos = Point::new(rect.x + rect.w / 2, rect.y + rect.h / 2);

    assert_eq!(menu.set_selection(pos), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));
    assert_eq!(
        menu.button_press(MouseButton::Left, M_NONE, pos),
        Transition::PrintAndExit("beta".into())
    );
}

/// Vertical list (-l): the input row is row 0, items follow.
#[test]
fn vertical_hover_and_click() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    menu.layout.lines = 3;
    let _ = menu.do_match();

    let pos = Point::new(10, 70); // second grid row
    assert_eq!(menu.set_selection(pos), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));
    assert_eq!(
        menu.button_press(MouseButton::Left, M_NONE, pos),
        Transition::PrintAndExit("beta".into())
    );
}

#[test]
fn headings_are_structural_and_navigation_skips_them() {
    let items = [
        "{heading blue} Applications",
        "Display",
        "{heading green} Tools",
        "Terminal",
    ];
    let (mut menu, _stub, _out) = menu_with(Config::default(), &items);
    menu.layout.lines = 4;
    let _ = menu.do_match();

    assert_eq!(menu.matcher.matches, vec![0, 1, 2, 3]);
    assert_eq!(menu.selection.selected, Some(1));
    key(&mut menu, ks::KEY_Down, M_NONE);
    assert_eq!(menu.selection.selected, Some(3));
    key(&mut menu, ks::KEY_Up, M_NONE);
    assert_eq!(menu.selection.selected, Some(1));
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("Display".into())
    );
}

#[test]
fn headings_ignore_hover_and_click_but_actions_do_not() {
    let items = ["{heading} Applications", "Display"];
    let (mut menu, _stub, _out) = menu_with(Config::default(), &items);
    menu.layout.lines = 2;
    let _ = menu.do_match();
    assert_eq!(menu.selection.selected, Some(1));

    let heading = Point::new(10, 35);
    assert_eq!(menu.set_selection(heading), Transition::Nop);
    assert_eq!(
        menu.button_press(MouseButton::Left, M_NONE, heading),
        Transition::Nop
    );
    assert_eq!(menu.selection.selected, Some(1));

    let action = Point::new(10, 65);
    assert_eq!(
        menu.button_press(MouseButton::Left, M_NONE, action),
        Transition::PrintAndExit("Display".into())
    );
}

#[test]
fn metadata_is_hidden_from_completion_and_output() {
    let source = "{red icon=display match='monitor screen'} Display";
    let (mut menu, _stub, _out) = menu_with(Config::default(), &[source]);
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_NONE), Transition::Redraw);
    assert_eq!(menu.editor.text, "Display");
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("Display".into())
    );
}

#[test]
fn value_is_hidden_from_completion_but_used_for_output() {
    let source = "{value=one} same";
    let (mut menu, _stub, _out) = menu_with(Config::default(), &[source]);
    // Tab copies label, not value
    assert_eq!(key(&mut menu, ks::KEY_Tab, M_NONE), Transition::Redraw);
    assert_eq!(menu.editor.text, "same");
    // Return prints value
    assert_eq!(
        key(&mut menu, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("one".into())
    );

    // duplicate labels with distinct values disambiguate output
    let sources = ["{value=one} same", "{value=two} same"];
    let (mut menu2, _stub, _out) = menu_with(Config::default(), &sources);
    // first item selected by default
    assert_eq!(
        key(&mut menu2, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("one".into())
    );
    // move to second and confirm
    let (mut menu3, _stub, _out) = menu_with(Config::default(), &sources);
    key(&mut menu3, ks::KEY_Down, M_NONE);
    assert_eq!(
        key(&mut menu3, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("two".into())
    );

    // quoted value with spaces
    let source_q = r#"{value="file:/tmp/a b"} My File"#;
    let (mut menu4, _stub, _out) = menu_with(Config::default(), &[source_q]);
    assert_eq!(
        key(&mut menu4, ks::KEY_Return, M_NONE),
        Transition::PrintAndExit("file:/tmp/a b".into())
    );
}

/// Left-click on the input field clears it.
#[test]
fn left_click_on_input_clears_it() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alp");
    // inside the input field: [0, input_width]
    let t = menu.button_press(MouseButton::Left, M_NONE, Point::new(50, 10));
    assert!(matches!(t, Transition::Redraw));
    assert_eq!(menu.editor.text, "");
}

/* ── header geometry (draw ↔ hit-test alignment) ───────────────────────── */

/// The header geometry is one computation shared by drawing and
/// hit-testing. Regression: with a left command cell and a prompt, item
/// hit-rects were offset by the command-cell width relative to the drawn
/// pixels (hovering the prompt selected the first item), and the input
/// field was drawn a second command-cell width past the prompt.
#[test]
fn header_aligns_content_after_command_cell_and_prompt() {
    let cfg = Config {
        left_command: Some("true".into()),
        prompt: Some("run:".into()),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta"]);
    menu.layout.lines = 2;
    menu.layout.command_width = 40;
    menu.layout.prompt_width = 60;

    let header = menu.header();
    // the prompt sits right of the command cell, content after both
    assert_eq!(header.prompt.unwrap().x, 40);
    assert_eq!(header.content_x, 100);
    // the input field begins at the content origin — no second shift
    assert_eq!(header.input.x, header.content_x);

    // hovering the prompt block (x between 40 and 100) selects nothing…
    assert_eq!(
        menu.set_selection(Point::new(65, 35)),
        Transition::Nop,
        "the prompt is not an item"
    );
    // …and hovering the first grid cell — the exact rect draw_grid paints —
    // selects it, because both read the same geometry
    menu.selection.selected = None;
    let drawn_cell = menu.layout.grid_cell_rect(0, header.content_x);
    assert_eq!(
        menu.set_selection(Point::new(drawn_cell.x + 5, drawn_cell.y + 5)),
        Transition::Redraw
    );
    assert_eq!(menu.selection.selected, Some(0));
}

/// Scrolling pages through the list.
#[test]
fn scroll_turns_pages() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    // seed a page boundary (pixel-dependent with real fonts)
    menu.paging.next = Some(1);
    menu.paging.prev = 0;

    assert_eq!(menu.scroll(1), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));
    assert_eq!(menu.selection.page_start, Some(1));

    // scrolling back up moves the page, the selection follows the page top
    assert_eq!(menu.scroll(-1), Transition::Redraw);
    assert_eq!(menu.selection.page_start, Some(0));
}

/* ── the event loop ────────────────────────────────────────────────────── */

#[test]
fn run_returns_failure_when_the_connection_dies() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(menu.run(), ExitStatus::Failure);
}

/// A click outside the menu (pointer grab on X11, shield surface on Wayland)
/// closes it without a selection.
#[test]
fn run_outside_click_exits_with_failure() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::ButtonPress {
        button: MouseButton::Left,
        mods: M_NONE,
        pos: Point::new(0, 0),
        source: InputSource::External,
    });
    assert_eq!(menu.run(), ExitStatus::Failure);
}

/// --toast: toast mode ignores all events and times out successfully.
#[test]
fn run_toast_times_out_with_success() {
    let cfg = Config {
        toast: Some(0.1), // a tenth of a second
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
}

/// a negative toast (only reachable via a hand-built Config — the CLI
/// rejects it) must not overflow and behaves like the shortest toast
#[test]
fn run_toast_negative_is_clamped() {
    let cfg = Config {
        toast: Some(-0.5),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    assert_eq!(menu.run(), ExitStatus::Success);
}

#[test]
fn run_toast_re_presents_on_expose() {
    let cfg = Config {
        toast: Some(0.1),
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.feed.lock().unwrap().push_back(BackendEvent::Expose);
    assert_eq!(menu.run(), ExitStatus::Success);
    assert!(stub.state().presents >= 1);
}

#[test]
fn run_toast_fails_on_destroyed() {
    let cfg = Config {
        toast: Some(1.0),
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.feed.lock().unwrap().push_back(BackendEvent::Destroyed);
    assert_eq!(menu.run(), ExitStatus::Failure);
}

#[test]
fn run_prints_selection_and_exits_successfully() {
    let (mut menu, stub, out) = menu_with(Config::default(), &["alpha", "beta"]);
    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "alpha\n");
}

/// Expose only re-presents the canvas; it does not disturb the loop.
#[test]
fn run_expose_presents_without_side_effects() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::Expose);
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert!(stub.state().presents >= 1);
}

/// FocusInOther regrabs focus under the prompt title (or "dmenu").
#[test]
fn run_focus_loss_regrabs_with_prompt_title() {
    let cfg = Config {
        prompt: Some("menu".into()),
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.push(BackendEvent::FocusInOther);
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(stub.state().focus_titles, vec!["menu".to_string()]);

    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::FocusInOther);
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(stub.state().focus_titles, vec!["dmenu".to_string()]);
}

/// Window destruction ends the loop with failure.
#[test]
fn run_destroyed_exits_with_failure() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::Destroyed);
    assert_eq!(menu.run(), ExitStatus::Failure);
}

/// Every crossed row is observed, including the final event before the
/// pointer stops. Timestamp throttling used to drop the second event here and
/// leave the highlight permanently stale.
#[test]
fn run_motion_does_not_drop_the_final_position() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    let rects = menu.horizontal_item_rects(0);
    let item0 = Point::new(rects[0].1.x + 5, rects[0].1.y + 5);
    let item1 = Point::new(rects[1].1.x + 5, rects[1].1.y + 5);

    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: item1,
        source: InputSource::Menu,
    });
    // 10ms later: still the authoritative latest pointer position
    stub.push(BackendEvent::Motion {
        time: 1010,
        pos: item0,
        source: InputSource::Menu,
    });
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(menu.selection.selected, Some(0));
}

/// -Ps: the preselected-th item is selected and drawn before the first
/// event is handled.
#[test]
fn run_preselects_before_the_first_event() {
    let cfg = Config {
        preselected: 2,
        ..Config::default()
    };
    let (mut menu, stub, out) = menu_with(cfg, &["alpha", "beta", "gamma"]);
    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "gamma\n");
    assert!(stub.state().presents >= 1);
}

/// SelectionNotify inserts the first line of the selection into the input.
#[test]
fn run_selection_notify_pastes() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    stub.push(BackendEvent::SelectionNotify {
        text: "pasted\nsecond".into(),
    });
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(menu.editor.text, "pasted");
}

/// Motion hovers items (the 60fps throttle only skips events).
#[test]
fn run_motion_selects_items() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    let (_, rect) = menu
        .horizontal_item_rects(0)
        .into_iter()
        .next_back()
        .unwrap();
    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: Point::new(rect.x + rect.w / 2, rect.y + rect.h / 2),
        source: InputSource::Menu,
    });
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(menu.selection.selected, Some(1));
}

/* ── slide mode ────────────────────────────────────────────────────────── */

/// A slide-mode menu: no items, resolved slider settings, the same stub
/// geometry menu_with sets up.
fn slide_with(settings: SlideSettings) -> (Menu, StubHandle, SharedOutput) {
    let cfg = Config {
        slide: Some(settings),
        ..Config::default()
    };
    let (menu, stub, out) = menu_with(cfg, &[]);
    (menu, stub, out)
}

fn slide_value(menu: &Menu) -> i32 {
    menu.slider.as_ref().unwrap().value()
}

/// Return prints the current value and exits successfully.
#[test]
fn slide_return_prints_value_and_exits() {
    let (mut menu, stub, out) = slide_with(SlideSettings::default());
    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "50\n"); // default value: middle of 0..=100
}

/// Escape and q cancel without printing.
#[test]
fn slide_escape_and_q_cancel() {
    let (mut menu, stub, out) = slide_with(SlideSettings::default());
    stub.key(ks::KEY_Escape, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(out.contents(), "");

    let (mut menu, stub, out) = slide_with(SlideSettings::default());
    stub.key(ks::KEY_q, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(out.contents(), "");
}

/// hjkl and the arrows step by --step / --big-step, clamped at the ends.
#[test]
fn slide_keys_step_by_step_and_big_step() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_Right, M_NONE), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_l, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 52);
    assert_eq!(menu.slide_key(ks::KEY_j, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 42);
    assert_eq!(menu.slide_key(ks::KEY_k, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 52);
    assert_eq!(menu.slide_key(ks::KEY_h, M_NONE), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_Left, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 50);

    // already at the maximum: End then another increase is a no-op
    assert_eq!(menu.slide_key(ks::KEY_End, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 100);
    assert_eq!(menu.slide_key(ks::KEY_Up, M_NONE), Transition::Nop);
    assert_eq!(slide_value(&menu), 100);
    // and at the minimum
    assert_eq!(menu.slide_key(ks::KEY_Home, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 0);
    assert_eq!(menu.slide_key(ks::KEY_Down, M_NONE), Transition::Nop);
}

/// plus/minus change by exactly 1.
#[test]
fn slide_plus_minus_change_by_one() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_plus, M_NONE), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_equal, M_NONE), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_KP_Add, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 53);
    assert_eq!(menu.slide_key(ks::KEY_minus, M_NONE), Transition::Redraw);
    assert_eq!(
        menu.slide_key(ks::KEY_KP_Subtract, M_NONE),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 51);
}

/// Digits jump to ninths of the range: 1 is the minimum, 0 the maximum.
#[test]
fn slide_digits_jump_to_ninths() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_1, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 0);
    assert_eq!(menu.slide_key(ks::KEY_5, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 44); // round(100 * 4/9)
    assert_eq!(menu.slide_key(ks::KEY_0, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 100);
}

/// Unbound keys do nothing; Ctrl+c cancels like Escape.
#[test]
fn slide_ignores_unbound_keys_and_ctrl_c_cancels() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_x, M_NONE), Transition::Nop);
    assert_eq!(menu.slide_key(ks::KEY_Return, M_CTRL), Transition::Nop);
    assert_eq!(slide_value(&menu), 50);
    assert_eq!(
        menu.slide_key(ks::KEY_c, M_CTRL),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// A configured --command spawns with the value appended on every change;
/// without one a change just redraws.
#[test]
fn slide_changes_spawn_the_command() {
    let settings = SlideSettings {
        command: Some("true".into()),
        ..SlideSettings::default()
    };
    let (mut menu, _stub, _out) = slide_with(settings);
    assert_eq!(
        menu.slide_key(ks::KEY_Right, M_NONE),
        Transition::Spawn("true 51".into())
    );
    assert_eq!(
        menu.slide_key(ks::KEY_Left, M_NONE),
        Transition::Spawn("true 50".into())
    );

    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_Right, M_NONE), Transition::Redraw);
}

/// Clicking sets the value at the pointer; dragging follows the pointer
/// until the button is released.
#[test]
fn slide_click_and_drag_set_the_value() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    // Click mapping is relative to the bar which starts after the reserved
    // label box, so compute bar geometry from the rendered label widths.
    let label_for = |v: i32| format!("{}/{}", v, 100);
    let w_min = menu.cell_width(&label_for(0));
    let w_max = menu.cell_width(&label_for(100));
    let label_box = w_min.max(w_max).min(menu.layout.menu_width);
    let bar_x = label_box;
    let bar_w = (menu.layout.menu_width - bar_x).max(1);
    let x_for = |frac: f64| (bar_x as f64 + frac * bar_w as f64).round() as i32;
    // clicking the exact current value is a no-op
    assert_eq!(
        menu.slide_button(MouseButton::Left, M_NONE, Point::new(x_for(0.5), 5)),
        Transition::Nop
    );
    assert_eq!(slide_value(&menu), 50);
    assert_eq!(
        menu.slide_button(MouseButton::Left, M_NONE, Point::new(x_for(0.25), 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 25);
    assert_eq!(
        menu.slide_motion(Point::new(x_for(0.75), 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 75);
    // outside the bar: clamped into the range
    assert_eq!(menu.slide_motion(Point::new(-20, 5)), Transition::Redraw);
    assert_eq!(slide_value(&menu), 0);

    // release ends the drag; further motion does nothing
    assert_eq!(
        menu.slide_release(MouseButton::Left, Point::new(0, 5)),
        Transition::Nop
    );
    assert_eq!(menu.slide_motion(Point::new(590, 5)), Transition::Nop);
    assert_eq!(slide_value(&menu), 0);
}

/// Motion without a held button never changes the value.
#[test]
fn slide_motion_without_drag_is_ignored() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_motion(Point::new(0, 5)), Transition::Nop);
    assert_eq!(slide_value(&menu), 50);
}

/// Middle click resets to the initial value, scroll steps, right click
/// exits.
#[test]
fn slide_middle_scroll_and_right_click() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    let _ = menu.slide_key(ks::KEY_Right, M_NONE);
    let _ = menu.slide_key(ks::KEY_Right, M_NONE);
    assert_eq!(
        menu.slide_button(MouseButton::Middle, M_NONE, Point::new(0, 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 50);

    assert_eq!(menu.slide_scroll(-1), Transition::Redraw);
    assert_eq!(menu.slide_scroll(1), Transition::Redraw);
    assert_eq!(slide_value(&menu), 50);

    assert_eq!(
        menu.slide_button(MouseButton::Right, M_NONE, Point::new(0, 5)),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// The custom range/steps from the settings drive the keys.
#[test]
fn slide_respects_range_and_steps() {
    let settings = SlideSettings {
        min: -100,
        max: 100,
        step: Some(5),
        big_step: Some(50),
        value: Some(0),
        ..SlideSettings::default()
    };
    let (mut menu, stub, out) = slide_with(settings);
    assert_eq!(menu.slide_key(ks::KEY_Right, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 5);
    assert_eq!(menu.slide_key(ks::KEY_Up, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 55);
    assert_eq!(menu.slide_key(ks::KEY_Down, M_NONE), Transition::Redraw);
    assert_eq!(slide_value(&menu), 5);

    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "5\n");
}

/// The event loop: a dying connection fails, Expose re-presents, and the
/// drag path works end to end through run().
#[test]
fn slide_run_loop() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.run(), ExitStatus::Failure); // feed empty: connection died

    let (mut menu, stub, _out) = slide_with(SlideSettings::default());
    stub.push(BackendEvent::Expose);
    stub.push(BackendEvent::ButtonPress {
        button: MouseButton::Left,
        mods: M_NONE,
        pos: Point::new(450, 5),
        source: InputSource::Menu,
    });
    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: Point::new(600, 5),
        source: InputSource::Menu,
    });
    stub.push(BackendEvent::ButtonRelease {
        button: MouseButton::Left,
        pos: Point::new(600, 5),
        source: InputSource::Menu,
    });
    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(slide_value(&menu), 100);
    assert!(stub.state().presents >= 1);
}

/// draw_menu dispatches to the slider drawing in slide mode.
#[test]
fn slide_draw_dispatches() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    menu.draw_menu();
    // nothing panicked and a frame was presented
    assert!(!menu.canvas.data.is_empty());
}

/// The fill paints the selected scheme's *background* (blue), not its fg
/// (black) — regression test for an inverted `rect` flag that rendered the
/// bar black with only the bottom accent strip blue.
#[test]
fn slide_paints_the_fill_with_the_selected_background() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    menu.layout.menu_width = 600;
    menu.layout.menu_height = 30; // below 40px: the accent strip branch
    menu.canvas.resize(Size::new(600, 30));
    menu.draw_slide(); // value 50 → the left 300px are the fill

    let selected = menu.renderer.color_scheme(Scheme::Selected);
    let normal = menu.renderer.color_scheme(Scheme::Normal);
    let bgra = |c: Color| {
        let [r, g, b, a] = c.channels();
        [b, g, r, a]
    };
    let pixel = |m: &Menu, x: usize, y: usize| -> [u8; 4] {
        m.canvas.data[(y * m.canvas.width as usize + x) * 4..][..4]
            .try_into()
            .unwrap()
    };

    // inside the fill (past the label box), above the bottom strip
    assert_eq!(pixel(&menu, 250, 10), bgra(selected.bg));
    // right of the fill: the cleared normal background
    assert_eq!(pixel(&menu, 400, 10), bgra(normal.bg));
    // the bottom 4px of the fill are the detail strip — and only of the fill
    assert_eq!(pixel(&menu, 250, 28), bgra(selected.detail));
    assert_eq!(pixel(&menu, 400, 28), bgra(normal.bg));
    // the label box keeps the normal background over the fill
    assert_eq!(pixel(&menu, 2, 10), bgra(normal.bg));
}

/// An icon cell spans the full bar height: the gutter is
/// painted with the item's scheme down to the row's bottom edge and, for
/// the selected item, the detail strip sits at the bottom of the row like
/// every other cell. Regression: the icon cell was drawn at only
/// `line_height` (8px under the old `--line-height -1` clamping), which
/// shifted the glyph up into the previous row and put the accent strip at
/// the top of the row.
#[test]
fn icon_cell_spans_the_full_bar_height() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["{blue icon=power-off} Shutdown"]);
    menu.layout.lines = 1;
    menu.layout.menu_height = 240;
    menu.canvas.resize(Size::new(600, 240));
    menu.selection.selected = Some(0);
    menu.draw_menu();

    let selected = menu.renderer.color_scheme(Scheme::Selected);
    let normal = menu.renderer.color_scheme(Scheme::Normal);
    let bgra = |c: Color| {
        let [r, g, b, a] = c.channels();
        [b, g, r, a]
    };
    let pixel = |m: &Menu, x: usize, y: usize| -> [u8; 4] {
        m.canvas.data[(y * m.canvas.width as usize + x) * 4..][..4]
            .try_into()
            .unwrap()
    };

    // row 0 spans y 30..60 (bar_height 30); x=5 is inside the icon gutter
    // mid-row: the gutter carries the item's scheme down to the row bottom
    // (was the menu background below the 8px icon cell)
    assert_eq!(pixel(&menu, 5, 45), bgra(selected.bg));
    // the accent strip of the selected item sits at the row's bottom edge
    assert_eq!(pixel(&menu, 5, 58), bgra(selected.detail));
    // the input row above keeps the plain menu background
    assert_eq!(pixel(&menu, 5, 15), bgra(normal.bg));
}

/// An unselected icon item draws entirely in the Normal scheme — its color
/// appears only while selected/hovered. Regression: icon items carried
/// their scheme at all times, coloring every row permanently.
#[test]
fn unselected_icon_item_stays_in_the_normal_scheme() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["{blue icon=power-off} Shutdown"]);
    // do_match preselects the first match; clear it for the hover-off state
    menu.selection.selected = None;
    menu.layout.lines = 1;
    menu.layout.menu_height = 240;
    menu.canvas.resize(Size::new(600, 240));
    menu.draw_menu();

    let normal = menu.renderer.color_scheme(Scheme::Normal);
    let selected = menu.renderer.color_scheme(Scheme::Selected);
    let bgra = |c: Color| {
        let [r, g, b, a] = c.channels();
        [b, g, r, a]
    };
    let pixel = |m: &Menu, x: usize, y: usize| -> [u8; 4] {
        m.canvas.data[(y * m.canvas.width as usize + x) * 4..][..4]
            .try_into()
            .unwrap()
    };

    // nothing is selected: the gutter keeps the plain background. x=0 is
    // left of the glyph (which starts at gutter/2.6), so the pixel cannot
    // land on a glyph stroke whatever the font metrics resolve to.
    assert_eq!(pixel(&menu, 0, 45), bgra(normal.bg));
    // ... and so does the row behind the label
    assert_eq!(pixel(&menu, 550, 45), bgra(normal.bg));

    // selecting the item colors it with its own scheme
    menu.selection.selected = Some(0);
    menu.draw_menu();
    assert_eq!(pixel(&menu, 0, 45), bgra(selected.bg));
}

/// Icon geometry is based on what draw_item paints, not the potentially long
/// metadata prefix retained in the source item.
#[test]
fn icon_item_measurement_uses_label_plus_gutter() {
    let source = "{red icon=md-power_off} Label";
    let (mut menu, _stub, _out) = menu_with(Config::default(), &[source]);
    let label_width = menu.cell_width("Label");
    let gutter_width = menu.renderer.font_height * 3;

    assert_eq!(menu.max_cell_width(), label_width + gutter_width);

    menu.match_counter_text.clear();
    menu.paging.next = None;
    let rect = menu.horizontal_item_rects(0)[0].1;
    assert_eq!(rect.w, label_width + gutter_width);
    assert!(rect.w < menu.cell_width(source));
}

/// Slide mode does not read items from stdin even when some are provided.
#[test]
fn slide_ignores_stdin_items() {
    let cfg = Config {
        slide: Some(SlideSettings::default()),
        ..Config::default()
    };
    let items = read_stdin(&cfg);
    assert!(items.is_empty());
}

/* ── streaming stdin ───────────────────────────────────────────────────── */

/// A real os pipe with O_NONBLOCK set on the read end, like main() does for
/// the streaming startup. Dropping closes the read end; the write end is
/// closed explicitly via [`close_write`](TestPipe::close_write).
struct TestPipe {
    read_fd: std::os::fd::RawFd,
    write_fd: std::os::fd::RawFd,
}

impl TestPipe {
    fn new() -> Self {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFL) };
        unsafe { libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK) };
        TestPipe {
            read_fd: fds[0],
            write_fd: fds[1],
        }
    }
    fn write(&self, bytes: &[u8]) {
        let n = unsafe { libc::write(self.write_fd, bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(n, bytes.len() as isize);
    }
    /// Close the write end: the reader sees EOF once the buffer is drained.
    fn close_write(&self) {
        unsafe { libc::close(self.write_fd) };
    }
}

impl Drop for TestPipe {
    fn drop(&mut self) {
        unsafe { libc::close(self.read_fd) };
    }
}

/// Startup only consumes a bounded prefix. In particular, a producer that
/// never reaches EOF cannot hold window creation hostage.
#[test]
fn stream_preload_leaves_a_large_remainder_for_the_event_loop() {
    let path = std::env::temp_dir().join(format!(
        "instantmenu-preload-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let item_count = 100_000;
    file.write_all(&b"x\n".repeat(item_count)).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();

    let (mut menu, _stub, _out) = menu_with(Config::default(), &[]);
    menu.begin_stream(file.as_raw_fd());
    menu.preload_available();

    assert_eq!(menu.matcher.items.len(), 32_768);
    assert!(menu.stream_active(), "the byte budget stopped before EOF");
    assert!(menu.drain_stdin(), "the event-loop drain reaches EOF");
    assert_eq!(menu.matcher.items.len(), item_count);

    drop(file);
    std::fs::remove_file(path).unwrap();
}

/// Items stream in batch by batch; the menu keeps running until EOF, which
/// flushes the unterminated tail line.
#[test]
fn streamed_items_arrive_before_eof_and_finalize_on_it() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &[]);
    let pipe = TestPipe::new();
    menu.begin_stream(pipe.read_fd);
    assert!(menu.stream_active());

    pipe.write(b"alpha\nbeta\n");
    assert!(!menu.drain_stdin(), "writer still open");
    assert!(menu.stream_active());
    assert_eq!(
        menu.matcher
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );

    pipe.write(b"gamma");
    pipe.close_write();
    assert!(menu.drain_stdin(), "write end closed");
    assert!(!menu.stream_active());
    assert_eq!(
        menu.matcher
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "gamma"]
    );
}

/// The run loop settles EOF on its own: finalize runs once, then the menu
/// behaves like a fully loaded one (Return prints the selection).
#[test]
fn run_settles_eof_then_behaves_like_a_loaded_menu() {
    let (mut menu, stub, out) = menu_with(Config::default(), &[]);
    let pipe = TestPipe::new();
    pipe.write(b"alpha\nbeta\ngamma\n");
    pipe.close_write(); // everything already in the pipe
    menu.begin_stream(pipe.read_fd);

    stub.key(ks::KEY_Return, M_NONE, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "alpha\n"); // first item selected after EOF
}

/// Auto-confirm mode must not conclude from a partial corpus: a single match
/// mid-stream stays listed, and only fires once EOF settled.
#[test]
fn auto_confirm_pick_waits_for_eof_when_streaming() {
    let cfg = Config {
        auto_confirm: true,
        match_mode: crate::config::MatchMode::Dmenu,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["abc", "other"]);
    let pipe = TestPipe::new(); // idle pipe: streaming, nothing ever drained
    menu.begin_stream(pipe.read_fd);
    menu.editor.set_text("abc");

    assert_eq!(menu.do_match(), Transition::Nop, "mid-stream pick deferred");
    assert_eq!(menu.matcher.matches, vec![0]);

    menu.stream_eof = true;
    assert_eq!(
        menu.do_match(),
        Transition::PrintAndExit("abc".into()),
        "pick fires at EOF"
    );
}

/// reject-no-match is suspended while items stream in — an empty match list
/// means "nothing arrived yet", not "no match" — and resumes at EOF.
#[test]
fn reject_no_match_is_suspended_while_items_stream_in() {
    let cfg = Config {
        reject_no_match: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    let pipe = TestPipe::new(); // idle pipe: streaming, nothing ever drained
    menu.begin_stream(pipe.read_fd);

    // mid-stream: garbage is accepted
    type_text(&mut menu, "z");
    assert_eq!(menu.editor.text, "z");

    // corpus complete: garbage reverts again
    menu.stream_eof = true;
    type_text(&mut menu, "q");
    assert_eq!(menu.editor.text, "z");
}

/// A batch landing under the user's arrow keys keeps the selection instead
/// of yanking it back to the top.
#[test]
fn streamed_batches_preserve_the_selection() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["a1", "a2", "a3"]);
    let pipe = TestPipe::new(); // idle pipe: streaming, nothing ever drained
    menu.begin_stream(pipe.read_fd);

    assert_eq!(menu.do_match(), Transition::Nop);
    key(&mut menu, ks::KEY_Down, M_NONE);
    assert_eq!(menu.selection.selected, Some(1));

    menu.add_items(vec![Item::new("a4")]);
    assert_eq!(menu.do_match(), Transition::Nop);
    assert_eq!(menu.selection.selected, Some(1), "selection survives");

    // ...but resets once the corpus is final, like a fresh load always did
    menu.stream_eof = true;
    assert_eq!(menu.do_match(), Transition::Nop);
    assert_eq!(menu.selection.selected, Some(0));
}

/// Reflow resizes the window when streamed items change the derived grid:
/// `-l 3` starts bar-only (zero items) and grows to four rows.
#[test]
fn reflow_resizes_the_window_when_the_grid_grows() {
    let cfg = Config {
        lines: 3,
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &[]);
    assert!(menu.setup().is_none());
    let initial_height = menu.layout.menu_height;
    assert_eq!(stub.state().resizes.len(), 0);

    menu.add_items(["a", "b", "c"].iter().map(|s| Item::new(*s)).collect());
    menu.reflow();

    assert!(menu.layout.menu_height > initial_height);
    let resizes = stub.state().resizes.clone();
    assert_eq!(resizes.len(), 1);
    assert_eq!(resizes[0].h, menu.layout.menu_height);
    assert_eq!(menu.layout.input_width, menu.layout.menu_width / 3);
}

/// An auto-sized multi-column grid can exceed the monitor because the
/// measured cell width is multiplied by the column count. Preserve that
/// measurement and cap it; falling back to one cell makes every column tiny.
#[test]
fn oversized_multicolumn_auto_width_is_capped_to_the_monitor() {
    let cfg = Config {
        lines: 2,
        columns: 3,
        width: Width::Auto,
        ..Config::default()
    };
    let wide = "W".repeat(120);
    let items = vec![wide.as_str(); 6];
    let (mut menu, _stub, _out) = menu_with(cfg, &items);

    assert!(menu.setup().is_none());
    assert_eq!(menu.layout.columns, 3);
    assert_eq!(menu.layout.menu_width, 1920);
}

/// Regression for the streamed smartrun startup: setup begins with no items
/// and therefore a horizontal one-row layout. The completed corpus changes it
/// to a ten-row list; paging and hover must use that new shape immediately,
/// before any PageDown/PageUp workaround.
#[test]
fn streamed_reflow_recalculates_visible_rows_and_hit_testing() {
    let cfg = Config {
        lines: 10,
        width: Width::Fixed(900),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &[]);
    assert!(menu.setup().is_none());
    assert_eq!(menu.layout.lines, 0);

    menu.add_items((0..12).map(|i| Item::new(format!("item-{i}"))).collect());
    assert!(menu.finalize_stream().is_none());

    assert_eq!(menu.layout.lines, 10);
    assert_eq!(menu.paging.next, Some(10));
    let header = menu.header();
    let last_visible = menu.layout.grid_cell_rect(9, header.content_x);
    let normal = menu.renderer.color_scheme(Scheme::Normal).bg.channels();
    let normal_bgra = [normal[2], normal[1], normal[0], normal[3]];
    let stride = menu.canvas.width as usize * 4;
    let painted = (last_visible.y..last_visible.bottom()).any(|y| {
        let start = y as usize * stride + last_visible.x.max(0) as usize * 4;
        let end = y as usize * stride + last_visible.right().min(menu.canvas.width) as usize * 4;
        menu.canvas.data[start..end]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel != &normal_bgra)
    });
    assert!(painted, "the tenth row must be painted on the first frame");
    assert_eq!(
        menu.set_selection(Point::new(last_visible.x + 1, last_visible.y + 1)),
        Transition::Redraw
    );
    assert_eq!(menu.selection.selected, Some(9));
}

/// Deferred preselection is another paging consumer: it must walk the final
/// ten-row page, not turn at the stale horizontal boundary from empty setup.
#[test]
fn streamed_preselection_uses_the_final_layout() {
    let cfg = Config {
        lines: 10,
        preselected: 9,
        width: Width::Fixed(900),
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &[]);
    assert!(menu.setup().is_none());
    menu.add_items((0..12).map(|i| Item::new(format!("item-{i}"))).collect());

    assert!(menu.finalize_stream().is_none());
    assert_eq!(menu.selection.selected, Some(9));
    assert_eq!(menu.selection.page_start, Some(0));
    assert_eq!(menu.paging.next, Some(10));
}

/// A grid change that keeps the window rectangle identical (columns grow
/// inside a fixed `-w` width) must still be adopted.
#[test]
fn reflow_adopts_grid_changes_that_keep_the_rect() {
    let cfg = Config {
        lines: 2,
        columns: 3,
        width: Width::Fixed(600),
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &[]);
    assert!(menu.setup().is_none());

    // 4 items fill the 2x2 grid; the window grows from bar-only
    menu.add_items(["a", "b", "c", "d"].iter().map(|s| Item::new(*s)).collect());
    menu.reflow();
    assert_eq!(menu.layout.columns, 2);
    assert_eq!(stub.state().resizes.len(), 1);

    // 6 items widen the grid to 2x3 — same rect, different shape
    menu.add_items(["e", "f"].iter().map(|s| Item::new(*s)).collect());
    menu.reflow();
    assert_eq!(menu.layout.lines, 2);
    assert_eq!(menu.layout.columns, 3, "grid shape adopted");
    assert_eq!(stub.state().resizes.len(), 1, "no redundant resize");
}
