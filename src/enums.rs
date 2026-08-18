//! Port of `enums.h`.

/* color schemes */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Norm,
    Fade,
    Highlight,
    Hover,
    Sel,
    Out,
    Green,
    Yellow,
    Red,
}

pub const SCHEME_COUNT: usize = 9;

impl Scheme {
    pub const ALL: [Scheme; SCHEME_COUNT] = [
        Scheme::Norm,
        Scheme::Fade,
        Scheme::Highlight,
        Scheme::Hover,
        Scheme::Sel,
        Scheme::Out,
        Scheme::Green,
        Scheme::Yellow,
        Scheme::Red,
    ];

    /// X resource scheme names, port of `xresscheme`.
    pub fn xres_name(self) -> &'static str {
        match self {
            Scheme::Norm => "norm",
            Scheme::Fade => "fade",
            Scheme::Highlight => "highlight",
            Scheme::Hover => "hover",
            Scheme::Sel => "sel",
            Scheme::Out => "out",
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

/// Port of `outputoffset`: how many chars of the item text are skipped when
/// drawing, by category.
pub fn outputoffset(category: ItemCategory) -> usize {
    match category {
        ItemCategory::Normal => 0,
        ItemCategory::Comment => 1,
        ItemCategory::ColoredComment => 4,
        ItemCategory::Colored => 2,
        ItemCategory::Icon => 6,
    }
}

/* color indices within a scheme */
pub const COL_FG: usize = 0;
pub const COL_BG: usize = 1;
pub const COL_DETAIL: usize = 2;
pub const COL_COUNT: usize = 3;
