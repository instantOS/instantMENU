//! Port of `enums.h`.

/* color schemes */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Normal,
    Fade,
    Highlight,
    Hover,
    Selected,
    Output,
    Green,
    Yellow,
    Red,
}

pub const SCHEME_COUNT: usize = 9;

impl Scheme {
    pub const ALL: [Scheme; SCHEME_COUNT] = [
        Scheme::Normal,
        Scheme::Fade,
        Scheme::Highlight,
        Scheme::Hover,
        Scheme::Selected,
        Scheme::Output,
        Scheme::Green,
        Scheme::Yellow,
        Scheme::Red,
    ];

    /// X resource scheme names, port of `xresscheme`.
    pub fn x_resource_name(self) -> &'static str {
        match self {
            Scheme::Normal => "norm",
            Scheme::Fade => "fade",
            Scheme::Highlight => "highlight",
            Scheme::Hover => "hover",
            Scheme::Selected => "sel",
            Scheme::Output => "out",
            Scheme::Green => "green",
            Scheme::Red => "red",
            Scheme::Yellow => "yellow",
        }
    }
}

/* item categories */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCategory {
    Normal,
    Comment,
    ColoredComment,
    Colored,
    Icon,
}

impl ItemCategory {
    /// `>`-prefixed items are comments: they can not be selected/activated.
    pub fn is_comment(self) -> bool {
        matches!(self, ItemCategory::Comment | ItemCategory::ColoredComment)
    }

    /// The category an item text's `>`/`>>`/`:` prefix selects, plus the
    /// scheme its prefix implies (None = plain item; the scheme then depends
    /// on selection/output state).
    pub fn from_prefix(text: &str, is_selected: bool) -> (ItemCategory, Option<Scheme>) {
        let bytes = text.as_bytes();
        if bytes.first() == Some(&b'>') {
            if bytes.get(1) == Some(&b'>') {
                match comment_scheme(bytes.get(2)) {
                    Some(s) => (ItemCategory::ColoredComment, Some(s)),
                    None => (ItemCategory::Comment, Some(Scheme::Normal)),
                }
            } else {
                (ItemCategory::Comment, Some(Scheme::Normal))
            }
        } else if bytes.first() == Some(&b':') {
            match colored_scheme(bytes.get(1)) {
                /* a selected `:c ` item with an unknown color code falls
                 * back to a plain selected item */
                Some(s) if is_selected => (ItemCategory::Colored, Some(s)),
                Some(_) => (ItemCategory::Colored, None),
                None if is_selected => (ItemCategory::Normal, None),
                None => (ItemCategory::Colored, None),
            }
        } else {
            (ItemCategory::Normal, None)
        }
    }
}

/// The scheme for a `>>` color code: r/g/y/h/b.
fn comment_scheme(code: Option<&u8>) -> Option<Scheme> {
    match code {
        Some(b'r') => Some(Scheme::Red),
        Some(b'g') => Some(Scheme::Green),
        Some(b'y') => Some(Scheme::Yellow),
        Some(b'h') => Some(Scheme::Highlight),
        Some(b'b') => Some(Scheme::Selected),
        _ => None,
    }
}

/// The scheme for a `:` colored item: r/g/y/b only — the C `drawitem` `:x`
/// branch has no highlight case (a selected `:c ` item with an unknown
/// code, including `h`, falls back to a plain selected item).
fn colored_scheme(code: Option<&u8>) -> Option<Scheme> {
    match code {
        Some(b'r') => Some(Scheme::Red),
        Some(b'g') => Some(Scheme::Green),
        Some(b'y') => Some(Scheme::Yellow),
        Some(b'b') => Some(Scheme::Selected),
        _ => None,
    }
}

/// Port of `outputoffset`: how many chars of the item text are skipped when
/// drawing, by category. (`ItemCategory::Icon` keeps the nominal value for a
/// 3-byte icon here; `draw_item` computes the real label offset from the
/// actual icon bytes — see `Menu::draw_icon`.)
pub fn output_offset(category: ItemCategory) -> usize {
    match category {
        ItemCategory::Normal => 0,
        ItemCategory::Comment => 1,
        ItemCategory::ColoredComment => 4,
        ItemCategory::Colored => 2,
        ItemCategory::Icon => 6,
    }
}

/// A slot within a color scheme (fg / bg / detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRole {
    Foreground,
    Background,
    Detail,
}

impl ColorRole {
    pub const ALL: [ColorRole; 3] = [
        ColorRole::Foreground,
        ColorRole::Background,
        ColorRole::Detail,
    ];

    /// X resource color type name, port of `xrescolortype`.
    pub fn x_res_name(self) -> &'static str {
        match self {
            ColorRole::Foreground => "fg",
            ColorRole::Background => "bg",
            ColorRole::Detail => "detail",
        }
    }
}

/// Process exit status: 0 = success, 1 = failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success,
    Failure,
}

impl ExitStatus {
    /// The process exit code.
    pub fn code(self) -> i32 {
        match self {
            ExitStatus::Success => 0,
            ExitStatus::Failure => 1,
        }
    }

    /// Exit the process with this status.
    pub fn exit(self) -> ! {
        std::process::exit(self.code());
    }
}

/// Left or right: the command cell / word side an action targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Movement direction for cursor / word-edge navigation (the +1/-1 steps of
/// the C `nextrune`/`movewordedge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    /// The +1 (forward) / -1 (backward) char step this direction is.
    pub fn step(self) -> isize {
        match self {
            Direction::Forward => 1,
            Direction::Backward => -1,
        }
    }
}

/// A text edit: insert a string at the cursor, or delete `n` bytes before
/// it. The signed-n `insert()` of the C code split into readable forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOp<'a> {
    Insert(&'a str),
    /// Delete `n` bytes before the cursor (clamped at the text start).
    Delete(usize),
}
