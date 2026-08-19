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
./build.sh
```

## Features

- alt-tab functionality
- mouse support
- animations and hover over indicators
- icon prefixes
- comments

## is this dmenu?

instantMENU started as a fork of dmenu and keeps the dmenu workflow (items on stdin,
selection on stdout, full keyboard control), with all extra features optional. The
command line uses modern long options (`--width`, `--right-command`, ...) with a few
single-letter shortcuts (`-i`, `-p`, `-l`, ...) instead of dmenu's historical flags.

--------
### instantOS is still in early beta, contributions always welcome
