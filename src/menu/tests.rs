//! Shell characterization tests: drive the [`Menu`] handlers against a stub
//! backend and assert on the returned [`Transition`]s — no window, and no
//! font-dependent pixels (fonts load, but nothing is rasterized).

use super::input::{read_stdin, StdinItems};
use super::layout::GridShape;
use super::matcher::Item;
use super::transition::Transition;
use super::Menu;
use crate::backend::{
    Backend, BackendEvent, EventPoll, MonitorInfo, MouseButton, CONTROL_MASK, MOD1_MASK, MOD4_MASK,
    SHIFT_MASK,
};
use crate::config::{Config, SlideSettings};
use crate::enums::{ExitStatus, Scheme};
use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color, Renderer};
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xkbcommon::xkb::keysyms as ks;

/* ── stub backend ──────────────────────────────────────────────────────── */

#[derive(Default)]
struct StubState {
    presents: usize,
    focus_titles: Vec<String>,
    selection_requests: Vec<bool>,
}

/// A backend with a feedable event queue; `next_event` pops from it and
/// returns None (connection died) once it is empty.
struct StubBackend {
    monitors: Vec<MonitorInfo>,
    feed: Arc<Mutex<VecDeque<BackendEvent>>>,
    state: Arc<Mutex<StubState>>,
}

impl Backend for StubBackend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }
    fn root_size(&self) -> Size {
        Size::new(1920, 1080)
    }
    fn create_window(
        &mut self,
        _rect: Rect,
        _border_width: i32,
        _managed: bool,
        _grab: bool,
        _class_hint: &str,
        _bg: Color,
        _border_color: Color,
    ) -> Result<(), String> {
        Ok(())
    }
    fn grab_focus(&mut self, title: &str) {
        self.state.lock().unwrap().focus_titles.push(title.into());
    }
    fn set_title(&mut self, _title: &str) {}
    fn present(&mut self, _canvas: &Canvas) {
        self.state.lock().unwrap().presents += 1;
    }
    fn poll_event(&mut self, timeout: Option<Duration>) -> EventPoll {
        if let Some(ev) = self.feed.lock().unwrap().pop_front() {
            return EventPoll::Event(ev);
        }
        if let Some(timeout) = timeout {
            std::thread::sleep(timeout);
            EventPoll::Timeout
        } else {
            EventPoll::Closed
        }
    }
    fn request_selection(&mut self, clipboard: bool) {
        self.state
            .lock()
            .unwrap()
            .selection_requests
            .push(clipboard);
    }
}

/// Test-side handle onto the stub backend living inside the menu.
#[derive(Clone)]
struct StubHandle {
    feed: Arc<Mutex<VecDeque<BackendEvent>>>,
    state: Arc<Mutex<StubState>>,
}

impl StubHandle {
    fn push(&self, ev: BackendEvent) {
        self.feed.lock().unwrap().push_back(ev);
    }
    fn key(&self, sym: u32, state: u32, text: &str) {
        self.push(BackendEvent::KeyPress {
            sym,
            state,
            text: text.to_string(),
        });
    }
    fn state(&self) -> std::sync::MutexGuard<'_, StubState> {
        self.state.lock().unwrap()
    }
}

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
    let renderer = Renderer::new(&cfg.fonts, &cfg.colors, &HashSet::new());
    let feed = Arc::new(Mutex::new(VecDeque::new()));
    let state = Arc::new(Mutex::new(StubState::default()));
    let backend = StubBackend {
        monitors: vec![MonitorInfo {
            rect: Rect::new(0, 0, 1920, 1080),
            name: "stub".into(),
        }],
        feed: feed.clone(),
        state: state.clone(),
    };
    let mut menu = Menu::new(cfg, renderer, Box::new(backend));
    menu.load_items(StdinItems {
        items: items.iter().map(|s| Item::new(*s)).collect(),
        grid: GridShape {
            lines: 0,
            columns: 1,
        },
    });
    // geometry normally computed by setup()
    menu.layout.menu_width = 600;
    menu.layout.menu_height = 240;
    menu.layout.bar_height = 30;
    menu.layout.input_width = 200;
    menu.layout.columns = 1;

    let out = SharedOutput::default();
    menu.out = Box::new(out.clone());

    let _ = menu.do_match();
    (menu, StubHandle { feed, state }, out)
}

/// Type raw text through key events (sym 0: plain characters are only
/// dispatched by their buffer).
fn type_text(menu: &mut Menu, text: &str) {
    for c in text.chars() {
        let _ = menu.key_press(0, 0, &c.to_string());
    }
}

fn key(menu: &mut Menu, sym: u32, state: u32) -> Transition {
    menu.key_press(sym, state, "")
}

/* ── confirm paths ─────────────────────────────────────────────────────── */

/// Return prints the selection and exits successfully.
#[test]
fn return_prints_selection_and_exits() {
    let (mut menu, _stub, out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Return, 0),
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
        key(&mut menu, ks::KEY_Return, CONTROL_MASK),
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
        key(&mut menu, ks::KEY_Return, SHIFT_MASK),
        Transition::PrintAndExit("al".into())
    );
}

/// Ctrl+1..9 select the n-th item and confirm it.
#[test]
fn ctrl_number_selects_and_confirms() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta", "gamma"]);
    assert_eq!(
        key(&mut menu, ks::KEY_2, CONTROL_MASK),
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
        key(&mut menu, ks::KEY_Return, 0),
        Transition::PrintAndExit("beta".into())
    );
}

/// Tab completes the input to the selected match.
#[test]
fn tab_completes_to_selection() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "al");
    assert_eq!(key(&mut menu, ks::KEY_Tab, 0), Transition::Redraw);
    assert_eq!(menu.editor.text, "alpha");
}

/// -r: an edit that empties the match list is reverted.
#[test]
fn reject_no_match_reverts_edit() {
    let cfg = Config {
        reject_no_match: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    type_text(&mut menu, "alph");
    type_text(&mut menu, "x");
    assert_eq!(menu.editor.text, "alph");
    assert_eq!(menu.matcher.matches, vec![0]);
}

/// Ctrl+u clears the input; Ctrl+w deletes the word left of the cursor.
#[test]
fn ctrl_u_clears_input() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alpha");
    assert_eq!(key(&mut menu, ks::KEY_u, CONTROL_MASK), Transition::Redraw);
    assert_eq!(menu.editor.text, "");
    assert_eq!(menu.matcher.matches, vec![0, 1]);
}

#[test]
fn ctrl_w_deletes_word() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["hello world"]);
    type_text(&mut menu, "hello world");
    assert_eq!(key(&mut menu, ks::KEY_w, CONTROL_MASK), Transition::Redraw);
    assert_eq!(menu.editor.text, "hello ");
}

/// Left moves the cursor; Ctrl+k truncates at it.
#[test]
fn ctrl_k_truncates_at_cursor() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alp");
    assert_eq!(key(&mut menu, ks::KEY_Left, 0), Transition::Redraw);
    assert_eq!(menu.editor.cursor, 2);
    assert_eq!(key(&mut menu, ks::KEY_k, CONTROL_MASK), Transition::Redraw);
    assert_eq!(menu.editor.text, "al");
}

/// Ctrl+s inserts the literal ".*" (the regex-any prefix trick).
#[test]
fn ctrl_s_inserts_regex_any() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(key(&mut menu, ks::KEY_s, CONTROL_MASK), Transition::Redraw);
    assert_eq!(menu.editor.text, ".*");
}

/// Ctrl+j/Ctrl+m are Return with the modifier consumed: a plain confirm.
#[test]
fn ctrl_j_confirms_like_return() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    assert_eq!(
        key(&mut menu, ks::KEY_j, CONTROL_MASK),
        Transition::PrintAndExit("alpha".into())
    );
    assert_eq!(
        key(&mut menu, ks::KEY_m, CONTROL_MASK),
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
    assert_eq!(key(&mut menu, ks::KEY_v, CONTROL_MASK), Transition::Redraw);
    assert_eq!(key(&mut menu, ks::KEY_y, CONTROL_MASK), Transition::Nop);
    assert!(!*stub.state().selection_requests.last().unwrap());
    // Shift+v holds the clipboard variant
    key(&mut menu, ks::KEY_v, CONTROL_MASK | SHIFT_MASK);
    assert!(*stub.state().selection_requests.last().unwrap());
}

/* ── instant and commented modes ───────────────────────────────────────── */

/// -n: typing down to a single fuzzy match prints it and exits mid-edit.
#[test]
fn instant_mode_picks_while_typing() {
    let cfg = Config {
        instant: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["abc", "bcd"]);
    assert_eq!(
        menu.key_press(0, 0, "a"),
        Transition::PrintAndExit("abc".into())
    );
}

/// -ct: the first typed byte picks the first item starting with it; a byte
/// no item starts with exits. (The first edit stays applied, so a second
/// keystroke still decides by the first byte — the menu would have exited
/// already in production.)
#[test]
fn commented_mode_picks_by_first_byte() {
    let cfg = Config {
        commented: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg.clone(), &["yes", "no"]);
    assert_eq!(
        menu.key_press(0, 0, "n"),
        Transition::PrintAndExit("no".into())
    );

    let (mut menu, _stub, _out) = menu_with(cfg.clone(), &["yes", "no"]);
    assert_eq!(
        menu.key_press(0, 0, "x"),
        Transition::Exit(ExitStatus::Success)
    );
}

/* ── exit paths ────────────────────────────────────────────────────────── */

#[test]
fn escape_exits_with_failure() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        key(&mut menu, ks::KEY_Escape, 0),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// Alt+F4 and Mod4+q quit; Ctrl+[ too.
#[test]
fn modifier_quit_keys_exit() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        key(&mut menu, ks::KEY_F4, MOD1_MASK),
        Transition::Exit(ExitStatus::Failure)
    );
    assert_eq!(
        key(&mut menu, ks::KEY_q, MOD4_MASK),
        Transition::Exit(ExitStatus::Failure)
    );
    assert_eq!(
        key(&mut menu, ks::KEY_bracketleft, CONTROL_MASK),
        Transition::Exit(ExitStatus::Failure)
    );
}

/// Right-click exits immediately.
#[test]
fn right_click_exits() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(
        menu.button_press(MouseButton::Right, 0, Point::new(0, 0)),
        Transition::Exit(ExitStatus::Failure)
    );
}

/* ── alt-tab ───────────────────────────────────────────────────────────── */

/// -A: Alt+Tab advances without confirming; the following Alt release
/// confirms the selection.
#[test]
fn alt_tab_release_confirms() {
    let cfg = Config {
        alt_tab: true,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha", "beta", "gamma"]);

    assert_eq!(key(&mut menu, ks::KEY_Tab, MOD1_MASK), Transition::Redraw);
    assert_eq!(menu.selection.selected, Some(1));

    // the Tab release only ends the tab cycle
    assert_eq!(menu.key_release(ks::KEY_Tab, MOD1_MASK), Transition::Nop);
    // the Alt release confirms
    assert_eq!(
        menu.key_release(ks::KEY_Alt_L, MOD1_MASK),
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
        key(&mut menu, ks::KEY_Left, SHIFT_MASK),
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
        menu.button_press(MouseButton::Left, 0, pos),
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
        menu.button_press(MouseButton::Left, 0, pos),
        Transition::PrintAndExit("beta".into())
    );
}

/// Left-click on the input field clears it.
#[test]
fn left_click_on_input_clears_it() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    type_text(&mut menu, "alp");
    // inside the input field: [0, input_width]
    let t = menu.button_press(MouseButton::Left, 0, Point::new(50, 10));
    assert!(matches!(t, Transition::Redraw));
    assert_eq!(menu.editor.text, "");
}

/// Scrolling pages through the list.
#[test]
fn scroll_turns_pages() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    // seed a page boundary (pixel-dependent with real fonts)
    menu.paging.next = Some(1);
    menu.paging.prev = 0;

    assert_eq!(
        menu.button_press(MouseButton::ScrollDown, 0, Point::new(0, 0)),
        Transition::Redraw
    );
    assert_eq!(menu.selection.selected, Some(1));
    assert_eq!(menu.selection.current, Some(1));

    // scrolling back up moves the page, the selection follows the page top
    assert_eq!(
        menu.button_press(MouseButton::ScrollUp, 0, Point::new(0, 0)),
        Transition::Redraw
    );
    assert_eq!(menu.selection.current, Some(0));
}

/* ── the event loop ────────────────────────────────────────────────────── */

#[test]
fn run_returns_failure_when_the_connection_dies() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &["alpha"]);
    assert_eq!(menu.run(), ExitStatus::Failure);
}

/// -T: toast mode ignores all events and times out successfully.
#[test]
fn run_toast_times_out_with_success() {
    let cfg = Config {
        toast: 1, // one tenth of a second
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.key(ks::KEY_Escape, 0, "");
    assert_eq!(menu.run(), ExitStatus::Success);
}

/// a negative toast (only reachable via a hand-built Config — the CLI
/// rejects it) must not overflow and behaves like the shortest toast
#[test]
fn run_toast_negative_is_clamped() {
    let cfg = Config {
        toast: -5,
        ..Config::default()
    };
    let (mut menu, _stub, _out) = menu_with(cfg, &["alpha"]);
    assert_eq!(menu.run(), ExitStatus::Success);
}

#[test]
fn run_toast_re_presents_on_expose() {
    let cfg = Config {
        toast: 1,
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
        toast: 10,
        ..Config::default()
    };
    let (mut menu, stub, _out) = menu_with(cfg, &["alpha"]);
    stub.feed.lock().unwrap().push_back(BackendEvent::Destroyed);
    assert_eq!(menu.run(), ExitStatus::Failure);
}

#[test]
fn run_prints_selection_and_exits_successfully() {
    let (mut menu, stub, out) = menu_with(Config::default(), &["alpha", "beta"]);
    stub.key(ks::KEY_Return, 0, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "alpha\n");
}

/// Expose only re-presents the canvas; it does not disturb the loop.
#[test]
fn run_expose_presents_without_side_effects() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::Expose);
    stub.key(ks::KEY_Escape, 0, "");
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
    stub.key(ks::KEY_Escape, 0, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(stub.state().focus_titles, vec!["menu".to_string()]);

    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha"]);
    stub.push(BackendEvent::FocusInOther);
    stub.key(ks::KEY_Escape, 0, "");
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

/// Motion is throttled to ~60fps: events closer than one frame are dropped.
#[test]
fn run_motion_is_throttled() {
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    let rects = menu.horizontal_item_rects(0);
    let item0 = Point::new(rects[0].1.x + 5, rects[0].1.y + 5);
    let item1 = Point::new(rects[1].1.x + 5, rects[1].1.y + 5);

    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: item1,
    });
    // 10ms later: within the frame budget, must be dropped
    stub.push(BackendEvent::Motion {
        time: 1010,
        pos: item0,
    });
    stub.key(ks::KEY_Escape, 0, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(menu.selection.selected, Some(1));

    // well past the budget, the same event applies
    let (mut menu, stub, _out) = menu_with(Config::default(), &["alpha", "beta"]);
    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: item1,
    });
    stub.push(BackendEvent::Motion {
        time: 2000,
        pos: item0,
    });
    stub.key(ks::KEY_Escape, 0, "");
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
    stub.key(ks::KEY_Return, 0, "");
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
    stub.key(ks::KEY_Escape, 0, "");
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
    });
    stub.key(ks::KEY_Escape, 0, "");
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
    stub.key(ks::KEY_Return, 0, "");
    assert_eq!(menu.run(), ExitStatus::Success);
    assert_eq!(out.contents(), "50\n"); // default value: middle of 0..=100
}

/// Escape and q cancel without printing.
#[test]
fn slide_escape_and_q_cancel() {
    let (mut menu, stub, out) = slide_with(SlideSettings::default());
    stub.key(ks::KEY_Escape, 0, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(out.contents(), "");

    let (mut menu, stub, out) = slide_with(SlideSettings::default());
    stub.key(ks::KEY_q, 0, "");
    assert_eq!(menu.run(), ExitStatus::Failure);
    assert_eq!(out.contents(), "");
}

/// hjkl and the arrows step by --step / --big-step, clamped at the ends.
#[test]
fn slide_keys_step_by_step_and_big_step() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_Right, 0), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_l, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 52);
    assert_eq!(menu.slide_key(ks::KEY_j, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 42);
    assert_eq!(menu.slide_key(ks::KEY_k, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 52);
    assert_eq!(menu.slide_key(ks::KEY_h, 0), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_Left, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 50);

    // already at the maximum: End then another increase is a no-op
    assert_eq!(menu.slide_key(ks::KEY_End, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 100);
    assert_eq!(menu.slide_key(ks::KEY_Up, 0), Transition::Nop);
    assert_eq!(slide_value(&menu), 100);
    // and at the minimum
    assert_eq!(menu.slide_key(ks::KEY_Home, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 0);
    assert_eq!(menu.slide_key(ks::KEY_Down, 0), Transition::Nop);
}

/// plus/minus change by exactly 1.
#[test]
fn slide_plus_minus_change_by_one() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_plus, 0), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_equal, 0), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_KP_Add, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 53);
    assert_eq!(menu.slide_key(ks::KEY_minus, 0), Transition::Redraw);
    assert_eq!(menu.slide_key(ks::KEY_KP_Subtract, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 51);
}

/// Digits jump to ninths of the range: 1 is the minimum, 0 the maximum.
#[test]
fn slide_digits_jump_to_ninths() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_1, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 0);
    assert_eq!(menu.slide_key(ks::KEY_5, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 44); // round(100 * 4/9)
    assert_eq!(menu.slide_key(ks::KEY_0, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 100);
}

/// Unbound keys do nothing; Ctrl+c cancels like Escape.
#[test]
fn slide_ignores_unbound_keys_and_ctrl_c_cancels() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_x, 0), Transition::Nop);
    assert_eq!(
        menu.slide_key(ks::KEY_Return, CONTROL_MASK),
        Transition::Nop
    );
    assert_eq!(slide_value(&menu), 50);
    assert_eq!(
        menu.slide_key(ks::KEY_c, CONTROL_MASK),
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
        menu.slide_key(ks::KEY_Right, 0),
        Transition::Spawn("true 51".into())
    );
    assert_eq!(
        menu.slide_key(ks::KEY_Left, 0),
        Transition::Spawn("true 50".into())
    );

    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    assert_eq!(menu.slide_key(ks::KEY_Right, 0), Transition::Redraw);
}

/// Clicking sets the value at the pointer; dragging follows the pointer
/// until the button is released.
#[test]
fn slide_click_and_drag_set_the_value() {
    let (mut menu, _stub, _out) = slide_with(SlideSettings::default());
    // menu_width is 600 in the stub geometry; clicking the exact current
    // value is a no-op
    assert_eq!(
        menu.slide_button(MouseButton::Left, 0, Point::new(300, 5)),
        Transition::Nop
    );
    assert_eq!(slide_value(&menu), 50);
    assert_eq!(
        menu.slide_button(MouseButton::Left, 0, Point::new(150, 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 25);
    assert_eq!(menu.slide_motion(Point::new(450, 5)), Transition::Redraw);
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
    let _ = menu.slide_key(ks::KEY_Right, 0);
    let _ = menu.slide_key(ks::KEY_Right, 0);
    assert_eq!(
        menu.slide_button(MouseButton::Middle, 0, Point::new(0, 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 50);

    assert_eq!(
        menu.slide_button(MouseButton::ScrollUp, 0, Point::new(0, 5)),
        Transition::Redraw
    );
    assert_eq!(
        menu.slide_button(MouseButton::ScrollDown, 0, Point::new(0, 5)),
        Transition::Redraw
    );
    assert_eq!(slide_value(&menu), 50);

    assert_eq!(
        menu.slide_button(MouseButton::Right, 0, Point::new(0, 5)),
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
    assert_eq!(menu.slide_key(ks::KEY_Right, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 5);
    assert_eq!(menu.slide_key(ks::KEY_Up, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 55);
    assert_eq!(menu.slide_key(ks::KEY_Down, 0), Transition::Redraw);
    assert_eq!(slide_value(&menu), 5);

    stub.key(ks::KEY_Return, 0, "");
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
        state: 0,
        pos: Point::new(450, 5),
    });
    stub.push(BackendEvent::Motion {
        time: 1000,
        pos: Point::new(600, 5),
    });
    stub.push(BackendEvent::ButtonRelease {
        button: MouseButton::Left,
        pos: Point::new(600, 5),
    });
    stub.key(ks::KEY_Return, 0, "");
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

/// The icon cell of a `:X ` item spans the full bar height: the gutter is
/// painted with the item's scheme down to the row's bottom edge and, for
/// the selected item, the detail strip sits at the bottom of the row like
/// every other cell. Regression: the icon cell was drawn at only
/// `line_height` (8px with `--line-height -1`), which shifted the glyph up
/// into the previous row and put the accent strip at the top of the row.
#[test]
fn icon_cell_spans_the_full_bar_height() {
    let (mut menu, _stub, _out) = menu_with(Config::default(), &[":b \u{f011}Shutdown"]);
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

/// Slide mode does not read items from stdin even when some are provided.
#[test]
fn slide_ignores_stdin_items() {
    let cfg = Config {
        slide: Some(SlideSettings::default()),
        ..Config::default()
    };
    let items = read_stdin(&cfg);
    assert!(items.items.is_empty());
}
