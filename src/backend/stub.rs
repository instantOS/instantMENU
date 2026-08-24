//! A configurable in-memory [`Backend`] shared by the menu-core
//! characterization tests and the geometry tests — one stub instead of two
//! hand-rolled copies of mostly no-op trait bodies. Run-loop tests reach the
//! event feed and recorded effects through a [`TestHandle`] while the backend
//! itself is boxed into a `Menu`; geometry tests only read the probe counters
//! afterwards.

use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Backend, BackendEvent, EventPoll, Modifiers, MonitorInfo};
use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color};

/// Everything the backend observed, shared between the backend inside the
/// menu and any number of handles.
#[derive(Default)]
pub(crate) struct TestState {
    pub(crate) presents: usize,
    pub(crate) focus_titles: Vec<String>,
    pub(crate) selection_requests: Vec<bool>,
    pub(crate) resizes: Vec<Rect>,
    /// `focused_monitor` probes.
    pub(crate) focus_calls: usize,
    /// `pointer_position` probes.
    pub(crate) pointer_calls: usize,
}

/// A stub backend: configurable static answers (monitors, focused monitor,
/// pointer position), an event queue to drain, and counters for everything
/// the menu asked it to do. The queue drains like a real connection: once it
/// is empty, an indefinite poll reports `Closed` (the C "connection died").
#[derive(Clone)]
pub(crate) struct TestBackend {
    pub(crate) monitors: Vec<MonitorInfo>,
    pub(crate) root: Size,
    pub(crate) focused: Option<usize>,
    pub(crate) pointer: Option<Point>,
    /// The live event queue; tests may also reach it through a handle.
    pub(crate) feed: Arc<Mutex<VecDeque<BackendEvent>>>,
    pub(crate) state: Arc<Mutex<TestState>>,
}

impl Default for TestBackend {
    fn default() -> Self {
        TestBackend {
            monitors: Vec::new(),
            root: Size::new(1920, 1080),
            focused: None,
            pointer: None,
            feed: Arc::new(Mutex::new(VecDeque::new())),
            state: Arc::new(Mutex::new(TestState::default())),
        }
    }
}

impl TestBackend {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A handle sharing this backend's feed and observation state.
    pub(crate) fn handle(&self) -> TestHandle {
        TestHandle {
            feed: self.feed.clone(),
            state: self.state.clone(),
        }
    }

    pub(crate) fn focus_calls(&self) -> usize {
        self.state.lock().unwrap().focus_calls
    }

    pub(crate) fn pointer_calls(&self) -> usize {
        self.state.lock().unwrap().pointer_calls
    }
}

impl Backend for TestBackend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    fn root_size(&self) -> Size {
        self.root
    }

    fn pointer_position(&mut self) -> Option<Point> {
        self.state.lock().unwrap().pointer_calls += 1;
        self.pointer
    }

    fn focused_monitor(&self) -> Option<usize> {
        self.state.lock().unwrap().focus_calls += 1;
        self.focused
    }

    fn create_window(
        &mut self,
        _rect: Rect,
        _border_width: i32,
        _managed: bool,
        _grab: bool,
        _outside_close: bool,
        _class_hint: &str,
        _bg: Color,
        _border_color: Color,
    ) -> Result<(), String> {
        Ok(())
    }

    fn grab_focus(&mut self, title: &str) -> Result<(), String> {
        self.state.lock().unwrap().focus_titles.push(title.into());
        Ok(())
    }

    fn set_title(&mut self, _title: &str) {}

    fn present(&mut self, _canvas: &Canvas) {
        self.state.lock().unwrap().presents += 1;
    }

    fn resize_window(&mut self, rect: Rect) {
        self.state.lock().unwrap().resizes.push(rect);
    }

    fn poll_event(&mut self, timeout: Option<Duration>, _extra: &[RawFd]) -> EventPoll {
        if let Some(ev) = self.feed.lock().unwrap().pop_front() {
            return EventPoll::Event(ev);
        }
        match timeout {
            Some(timeout) => {
                std::thread::sleep(timeout);
                EventPoll::Timeout
            }
            None => EventPoll::Closed,
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

/// Cloneable accessor for the parts of a boxed-in [`TestBackend`] a test
/// still wants to drive or inspect.
#[derive(Clone)]
pub(crate) struct TestHandle {
    /// The live event queue; push directly or through the helpers below.
    pub(crate) feed: Arc<Mutex<VecDeque<BackendEvent>>>,
    state: Arc<Mutex<TestState>>,
}

impl TestHandle {
    pub(crate) fn push(&self, ev: BackendEvent) {
        self.feed.lock().unwrap().push_back(ev);
    }

    pub(crate) fn key(&self, sym: u32, mods: Modifiers, text: &str) {
        self.push(BackendEvent::KeyPress {
            sym,
            mods,
            text: text.to_string(),
        });
    }

    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, TestState> {
        self.state.lock().unwrap()
    }
}
