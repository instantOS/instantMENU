//! Decode keys once into semantic actions. Scheduling and execution use the
//! same action, so aliases and mode-dependent keys cannot disagree about
//! whether they edit input, cancel, or consume search results.

use super::{AltTab, Menu};
use crate::backend::Modifiers;
use crate::enums::{Direction, Side};
use xkbcommon::xkb::keysyms as ks;

pub(super) enum KeyAction<'a> {
    Ignore,
    Redraw,
    Cancel,
    Insert(&'a str),
    Backspace,
    Delete,
    TruncateRight,
    DeleteLeft,
    DeleteWord,
    Word(Direction),
    Paste {
        mods: Modifiers,
        redraw: bool,
    },
    Cursor(Direction),
    CursorHome,
    Home,
    End,
    Column(Direction),
    Previous,
    Next,
    Page(Direction),
    Command(Side),
    Confirm(Modifiers),
    Complete,
    Cycle {
        direction: Option<Direction>,
        absorb_release: bool,
    },
    Number {
        offset: usize,
        mods: Modifiers,
    },
    Binding(String),
    Entry(usize),
}

impl KeyAction<'_> {
    pub fn needs_results(&self) -> bool {
        matches!(
            self,
            Self::Home
                | Self::End
                | Self::Column(_)
                | Self::Previous
                | Self::Next
                | Self::Page(_)
                | Self::Confirm(_)
                | Self::Complete
                | Self::Cycle { .. }
                | Self::Number { .. }
                | Self::Binding(_)
                | Self::Entry(_)
        )
    }
}

pub(super) struct DecodedKey<'a> {
    pub action: KeyAction<'a>,
    pub wait_for_shortcut: bool,
    pub disable_alt_tab: bool,
}

impl<'a> DecodedKey<'a> {
    fn new(action: KeyAction<'a>) -> Self {
        Self {
            action,
            wait_for_shortcut: false,
            disable_alt_tab: false,
        }
    }
}

impl Menu {
    pub(super) fn decode_key<'a>(
        &self,
        sym: u32,
        mods: Modifiers,
        text: &'a str,
    ) -> DecodedKey<'a> {
        if let Some(binding) = self
            .cfg
            .bindings
            .iter()
            .find(|binding| binding.matches(sym, mods))
        {
            return DecodedKey::new(KeyAction::Binding(binding.key.clone()));
        }
        let shortcuts = self.has_shortcuts
            && !self.cfg.single_key
            && !self.cfg.input_only
            && !self.cfg.password
            && self.cfg.toast.is_none();
        let pending = self.search_pending();
        if shortcuts && !pending {
            if let Some(index) = self
                .matcher
                .matches
                .iter()
                .copied()
                .filter(|&index| {
                    let item = &self.matcher.items[index];
                    item.is_selectable()
                        && item
                            .entry
                            .shortcut
                            .is_some_and(|key| key.matches(sym, mods))
                })
                .min_by_key(|&index| self.matcher.items[index].arrival_index)
            {
                return DecodedKey::new(KeyAction::Entry(index));
            }
        }
        let mut decoded = self.decode_builtin(sym, mods, text);
        decoded.wait_for_shortcut = shortcuts
            && pending
            && self.matcher.items.iter().any(|item| {
                item.is_selectable()
                    && item
                        .entry
                        .shortcut
                        .is_some_and(|key| key.matches(sym, mods))
            });
        decoded
    }

    fn decode_builtin<'a>(
        &self,
        mut sym: u32,
        mut mods: Modifiers,
        text: &'a str,
    ) -> DecodedKey<'a> {
        use KeyAction::*;
        let action = if mods.ctrl {
            match sym {
                ks::KEY_a => self.decode_base(ks::KEY_Home, mods, text),
                ks::KEY_b => self.decode_base(ks::KEY_Left, mods, text),
                ks::KEY_c | ks::KEY_g | ks::KEY_bracketleft => Cancel,
                ks::KEY_d => Delete,
                ks::KEY_e => End,
                ks::KEY_f => self.decode_base(ks::KEY_Right, mods, text),
                ks::KEY_h => Backspace,
                ks::KEY_i => self.decode_base(ks::KEY_Tab, mods, text),
                ks::KEY_j | ks::KEY_J | ks::KEY_m | ks::KEY_M => {
                    mods.ctrl = false;
                    Confirm(mods)
                }
                ks::KEY_n => Next,
                ks::KEY_p => Previous,
                ks::KEY_s => Insert(".*"),
                ks::KEY_v => Paste { mods, redraw: true },
                ks::KEY_y | ks::KEY_Y => Paste {
                    mods,
                    redraw: false,
                },
                ks::KEY_k => TruncateRight,
                ks::KEY_u => DeleteLeft,
                ks::KEY_w => DeleteWord,
                ks::KEY_Left | ks::KEY_KP_Left => Word(Direction::Backward),
                ks::KEY_Right | ks::KEY_KP_Right => Word(Direction::Forward),
                ks::KEY_Return | ks::KEY_KP_Enter => Confirm(mods),
                ks::KEY_1..=ks::KEY_9 => Number {
                    offset: (sym - ks::KEY_1) as usize,
                    mods,
                },
                _ => Ignore,
            }
        } else if mods.shift {
            if self.alt_tab != AltTab::Off
                && matches!(sym, ks::KEY_ISO_Left_Tab | ks::KEY_Tab | ks::KEY_KP_Tab)
            {
                Cycle {
                    direction: Some(Direction::Backward),
                    absorb_release: sym == ks::KEY_Tab,
                }
            } else {
                self.decode_base(sym, mods, text)
            }
        } else if mods.alt {
            match sym {
                ks::KEY_F4 => Cancel,
                ks::KEY_b => Word(Direction::Backward),
                ks::KEY_f => Word(Direction::Forward),
                ks::KEY_g | ks::KEY_G | ks::KEY_h | ks::KEY_j | ks::KEY_k | ks::KEY_l => {
                    sym = match sym {
                        ks::KEY_g => ks::KEY_Home,
                        ks::KEY_G => ks::KEY_End,
                        ks::KEY_h => ks::KEY_Up,
                        ks::KEY_j => ks::KEY_Next,
                        ks::KEY_k => ks::KEY_Prior,
                        _ => ks::KEY_Down,
                    };
                    self.decode_base(sym, mods, text)
                }
                ks::KEY_space => {
                    let mut key = DecodedKey::new(self.decode_base(sym, mods, text));
                    key.disable_alt_tab = true;
                    return key;
                }
                ks::KEY_Tab if self.alt_tab != AltTab::Off => Cycle {
                    direction: Some(Direction::Forward),
                    absorb_release: true,
                },
                ks::KEY_Tab => Complete,
                _ => Ignore,
            }
        } else if mods.logo && sym == ks::KEY_q {
            Cancel
        } else {
            self.decode_base(sym, mods, text)
        };
        DecodedKey::new(action)
    }

    fn decode_base<'a>(&self, sym: u32, mods: Modifiers, text: &'a str) -> KeyAction<'a> {
        use KeyAction::*;
        match sym {
            ks::KEY_Escape => Cancel,
            ks::KEY_Delete | ks::KEY_KP_Delete => Delete,
            ks::KEY_BackSpace => Backspace,
            ks::KEY_Home | ks::KEY_KP_Home => {
                if (self.selection.selected.is_none() && self.matcher.matches.is_empty())
                    || self.selection.selected == Some(0)
                {
                    CursorHome
                } else {
                    Home
                }
            }
            ks::KEY_End | ks::KEY_KP_End => End,
            ks::KEY_Up | ks::KEY_KP_Up => Previous,
            ks::KEY_Down | ks::KEY_KP_Down => Next,
            ks::KEY_Prior | ks::KEY_KP_Prior => Page(Direction::Backward),
            ks::KEY_Next | ks::KEY_KP_Next => Page(Direction::Forward),
            ks::KEY_Return | ks::KEY_KP_Enter => Confirm(mods),
            ks::KEY_space if self.cfg.space_confirm => Confirm(mods),
            ks::KEY_Tab if self.alt_tab != AltTab::Off => Cycle {
                direction: None,
                absorb_release: true,
            },
            ks::KEY_Tab => Complete,
            ks::KEY_Left | ks::KEY_KP_Left => self.decode_horizontal(Direction::Backward, mods),
            ks::KEY_Right | ks::KEY_KP_Right => self.decode_horizontal(Direction::Forward, mods),
            _ if text.bytes().next().is_some_and(|b| !b.is_ascii_control()) => Insert(text),
            _ => Redraw,
        }
    }

    fn decode_horizontal(&self, direction: Direction, mods: Modifiers) -> KeyAction<'static> {
        use KeyAction::*;
        if self.layout.columns > 1 {
            return Column(direction);
        }
        if (mods.shift || mods.logo)
            && (self.cfg.left_command.is_some() || self.cfg.right_command.is_some())
        {
            return Command(match direction {
                Direction::Backward => Side::Left,
                Direction::Forward => Side::Right,
            });
        }
        match direction {
            Direction::Backward
                if self.editor.cursor > 0
                    && (self.selection.selected.is_none()
                        || self.selection.selected == Some(0)
                        || self.layout.lines > 0) =>
            {
                Cursor(direction)
            }
            Direction::Forward if self.editor.cursor < self.editor.text.len() => Cursor(direction),
            _ if self.layout.lines > 0 => Ignore,
            Direction::Backward => Previous,
            Direction::Forward => Next,
        }
    }
}
