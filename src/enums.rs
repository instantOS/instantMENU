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

/// A slot within a color scheme (fg / bg / detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRole {
    Foreground,
    Background,
    Detail,
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
