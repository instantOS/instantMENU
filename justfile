# build the debug binary
build:
    cargo build

# install to ~/.local (debug build)
install: man
    cargo build
    install ./target/debug/instantmenu ~/.local/bin/
    install ./target/debug/itest ~/.local/bin/
    install ./instantmenu_path ~/.local/bin/
    install ./instantmenu_run ~/.local/bin/
    install ./instantmenu_smartrun ~/.local/bin/
    install -D -m 644 ./instantmenu.1 ~/.local/share/man/man1/instantmenu.1
    chmod +x ~/.local/bin/instantmenu_*

# install to system (root)
rootinstall: man
    cargo build
    sudo install ./target/debug/instantmenu /usr/local/bin/
    sudo install ./target/debug/itest /usr/local/bin/
    sudo install ./instantmenu_path /usr/local/bin/
    sudo install ./instantmenu_run /usr/local/bin/
    sudo install ./instantmenu_smartrun /usr/local/bin/
    sudo install -D -m 644 ./instantmenu.1 /usr/local/share/man/man1/instantmenu.1


# regenerate instantmenu.1 from the clap CLI definition
man:
    cargo run --bin instantmenu-mangen

# regenerate src/icons/names.rs from the nerd-fonts glyph names
icons:
    utils/gen_icons.py > src/icons/names.rs

# format code
format:
    cargo clippy --fix --allow-dirty
    cargo fmt

# release build
release:
    cargo build --release
