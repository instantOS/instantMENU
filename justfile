# build the debug binary
build:
    cargo build

# install to ~/.local/bin (debug build)
install:
    cargo build
    install ./target/debug/instantmenu ~/.local/bin/
    install ./target/debug/itest ~/.local/bin/
    install ./instantmenu_path ~/.local/bin/
    install ./instantmenu_run ~/.local/bin/
    install ./instantmenu_smartrun ~/.local/bin/
    chmod +x ~/.local/bin/instantmenu_*

# install to system (root)
rootinstall:
    cargo build
    sudo install ./target/debug/instantmenu /usr/local/bin/
    sudo install ./target/debug/itest /usr/local/bin/


# format code
format:
    cargo clippy --fix --allow-dirty
    cargo fmt

# release build
release:
    cargo build --release
