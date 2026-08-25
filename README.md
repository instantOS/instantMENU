<div align="center">
    <h1>instantMENU</h1>
    <p>General purpose menu for instantOS</p>
    <img width="300" height="300" src="https://raw.githubusercontent.com/instantOS/instantLOGO/main/png/menu.png">
</div>

instantMENU is a simple menu for instantOS
it is used for instantASSIST and in some other places

## Installation from source

```sh
git clone https://github.com/instantOS/instantMENU
cd instantMENU
cargo build --locked
# Or install the debug build and helper scripts for your user:
just install
```

Release archives, Arch Linux packages, and Debian packages are also attached to
each [GitHub release](https://github.com/instantOS/instantMENU/releases).

## Releases

The version-bump workflow opens a `release/version-bump` pull request using
conventional commits to choose the next version. Merging that PR creates the
matching `v*` tag and publishes the release artifacts. A patch, minor, or major
bump can also be selected manually from the workflow dispatch form.

## Features

- alt-tab functionality
- mouse support
- animations and hover over indicators
- composable item metadata for icons, colors, headings, hidden match terms,
  and single-key menus
- slider mode (`instantmenu slide 'brightnessctl s'`)
- streamed input: the menu opens instantly and items appear as stdin produces them
- Catppuccin, classic and Gruvbox themes (`--theme`)

## Appearance configuration

instantmenu reads one optional file: `$XDG_CONFIG_HOME/instantmenu/config.toml`,
or `~/.config/instantmenu/config.toml` when `XDG_CONFIG_HOME` is unset. Use
`--config PATH` to require another file, or `--no-config` to skip configuration.
The file is deliberately limited to font and colors:

```toml
font = "Iosevka:size=13"
theme = "gruvbox" # catppuccin (default), classic, or gruvbox

[colors.normal]
foreground = "#EBDBB2"
background = "#282828"
detail = "#504945"

[colors.selected]
foreground = "#282828"
background = "#83A598"
detail = "#8EC07C"
```

Every scheme is named: `normal`, `fade`, `highlight`, `hover`, `selected`,
`output`, `green`, `yellow`, and `red`; each accepts `foreground`, `background`,
and `detail`. Unknown keys and invalid colors are errors. A configured theme is
applied first, then configured colors, then command-line `--theme` and color
overrides.

## is this dmenu?

instantMENU started as a fork of dmenu and keeps the dmenu workflow (items on stdin,
selection on stdout, full keyboard control), with all extra features optional. The
command line uses modern long options (`--width`, `--right-command`, ...) with a few
single-letter shortcuts (`-i`, `-p`, `-l`, ...) instead of dmenu's historical flags.

## Item metadata

Each input line is normally its visible label and selected output. A leading
attribute block adds metadata without becoming part of either:

```text
Display
{blue icon=display match="monitor screen"} Display
{heading green} System actions
{red icon=power key=q} Power off
```

Known color names may be bare (`{red}`) or explicit (`{color=red}`). Other
attributes are `icon`, `key`, `match`, and the `heading` flag. Quote values
that contain spaces. Run with `--single-key` to show and activate only entries
that have `key=…`. Prefix a literal markup-like label with an extra opening
brace: `{{red} literal` displays `{red} literal`.

--------
### instantOS is still in early beta, contributions always welcome
