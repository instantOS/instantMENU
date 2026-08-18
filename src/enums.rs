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

/// Port of `outputoffset`: how many chars of the item text are skipped when
/// drawing, by category.
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
    pub const ALL: [ColorRole; 3] = [ColorRole::Foreground, ColorRole::Background, ColorRole::Detail];

    /// X resource color type name, port of `xrescolortype`.
    pub fn x_res_name(self) -> &'static str {
        match self {
            ColorRole::Foreground => "fg",
            ColorRole::Background => "bg",
            ColorRole::Detail => "detail",
        }
    }
}
