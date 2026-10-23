---
name: instantmenu
description: "Write scripts that use instantMENU for graphical selection menus, input prompts, and sliders on X11 or Wayland. Use when asked to invoke instantmenu or build a script around it."
---

# instantMENU

instantmenu is a graphical menu: feed newline-separated items on stdin and read
the chosen value from stdout. It needs a running X11 or Wayland session for
interactive use. `instantmenu skill`, `--help`, and icon discovery work without
a display.

Use the installed CLI's `--help` for options and `slide --help` for slider syntax.
Put mode-specific options after the subcommand. Bare `instantmenu` opens a menu;
it is not a discovery command.

## Write a selection script

Give items stable `value=` identifiers and dispatch on those values instead of
visible labels. This lets labels, icons, and search aliases change independently
of script behavior. For example, in a POSIX shell:

```sh
if choice=$(instantmenu --reject-no-match -l 8 -p 'Open' <<'MENU'
{heading blue} Locations
{icon=folder value=projects match="code repositories"} Projects
{icon=download value=downloads} Downloads
MENU
); then
    case "$choice" in
        projects) xdg-open "$HOME/projects" ;;
        downloads) xdg-open "$HOME/Downloads" ;;
    esac
fi
```

Normal acceptance exits 0. Cancellation and runtime failures exit 1; CLI syntax
errors exit 2. Check status before acting, including in scripts using `set -e`.
Do not `eval` the returned value: it is data, and ordinary menus can also return
typed text. `--reject-no-match` prevents unmatched input and Shift-Return's raw
input submission; still validate output against the script's expected values.

Ctrl-Return and Ctrl-click print a value and keep the menu open, so stdout can
contain several lines. Command substitution collects them until the menu exits;
consume stdout as a stream when actions should run immediately. Earlier output
can exist even if the user eventually cancels.

## Compose item markup

A plain line is both its label and output. One leading attribute block adds
metadata; follow the closing brace with whitespace before the label:

```text
{blue icon=display match="monitor screen" value=display-settings} Displays
{heading green icon=settings} System
{red icon=power key=q value=quit} Quit
```

- `value=TEXT` is the selected output, hidden from display and matching. Without
  it, selecting the item prints its label. All item activation paths use this
  value, including mouse, single-key, auto-confirm, and bound actions. Tab
  completion uses the label; Shift-Return explicitly submits input text.
- `match=TEXT` adds hidden search terms to the label for regular-menu matching.
- `icon=NAME` adds an icon gutter. Discover names with
  `instantmenu icons search <words>` or `instantmenu icons list`; these return
  tab-separated name, glyph, and codepoint. Names, literal icon/emoji glyphs,
  and icon hex codepoints are accepted. Prefer names from discovery over guesses.
- `key=CHAR` supplies one non-whitespace, non-control Unicode character. With
  `--single-key`, only keyed entries are shown and pressing a key immediately
  outputs that entry's value. Use distinct keys; search is disabled in this mode.
- `shortcut=KEY` adds a per-entry accelerator to a regular menu, with a right-aligned
  hint beside the full label. Examples: `shortcut=super-o`, `shortcut=ctrl-shift-e`,
  `shortcut=f2`. Combine `ctrl-`, `alt-`, `shift-`, and `super-` in any order.
  Characters, function keys (`f1`–`f35`), and named keys such as `space`, `left`,
  and `page-down` work. Letter names ignore case; specify Shift explicitly.
  Plain characters override search typing. Escape, Ctrl-C, Ctrl-G, and plain
  Enter/Tab are reserved. Only current search results can be activated, including
  other pages; duplicate shortcuts pick the first match in input order. Global
  `--bind` actions take precedence. Shortcuts print the entry's value and exit;
  they are inactive in single-key, input-only, password, and toast modes.
- `heading` creates a non-selectable structural row. It can have a color and
  icon, but cannot have `key`, `shortcut`, `match`, or `value`.
- Colors are palette roles: `normal` (`default`), `fade`, `highlight`, `hover`,
  `selected` (`blue`), `output`, `green`, `yellow`, and `red`. Use a bare role or
  `color=ROLE`. Item colors apply while selected; heading colors apply always.

Attributes can appear in any order, once each. Use `name=value` without spaces
around `=`. Single- or double-quote values containing whitespace or braces;
inside quoted values a backslash escapes the next character. This is instantmenu
markup quoting, separate from the shell's quoting. A quoted here-document, as
above, preserves static markup without shell interpolation.

Unknown attributes, unresolved icons, duplicate attributes, empty `match` or
`value`, and malformed blocks make the **entire line literal**, rather than
raising an error or applying partial metadata. A typo can therefore change the
output a script receives. Check generated metadata carefully.

To display a literal label beginning with `{`, prepend another `{`:
`{{red} literal` displays and outputs `{red} literal`. For arbitrary plain-text
input, escape every leading `{` this way before piping it to the menu. The
protocol is line-oriented, so embedded newlines cannot represent one item.
For dynamic metadata, escape backslashes and the chosen quote inside quoted
values; use stable identifiers to avoid embedding arbitrary paths in markup.

## Choose the script's interaction contract

Menus stream stdin while open; a slow producer does not delay opening. Close the
producer when its list is complete. Input-only and password prompts skip stdin;
toast mode reads its items to EOF before displaying. A bare menu reading from a
terminal waits for input and EOF, so provide a pipe or here-document for scripts.

`--bind KEY:LABEL` adds global actions and changes stdout framing: the first line
is the activated key, or an empty line for ordinary Enter, followed by selected
values. An action can fire with no selection and then prints only its key line.
The label is a hint, not a shell command. Preserve that initial empty line when
parsing; do not treat the whole response as an item ID.

Slider mode appends the numeric value to its shell command. It runs the command
at startup and on changes, detached; commands can overlap. Use an absolute setter
that tolerates repeated calls, and do not assume Escape undoes changes. Return
prints the final value; cancellation prints no final value. The command receives
a bare number (for example `50`), so adapt it when the target tool requires units.
For percentage volume, use a wrapper:

```sh
#!/bin/sh
# set-volume: make executable and put on PATH
pactl set-sink-volume @DEFAULT_SINK@ "${1}%"
```

```sh
instantmenu slide --min 0 --max 100 --value 50 set-volume
```

Appearance belongs to the user's config by default. Add geometry or theme
overrides when the requested UI needs them; a script usually does not need to
replace the user's font and palette.
