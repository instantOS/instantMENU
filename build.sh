#!/usr/bin/env bash

make clean &>/dev/null

THEME=themes/${1:-dracula}.theme
echo "$THEME"
grep -q 'size' <$THEME || { echo "theme not valid" && exit 1; }

replacetheme() {
    themecolor=$(grep "$1" <$THEME)
    sed -i 's/.*'"$1"'.*/'"$themecolor"'/' config.def.h
}

replacetheme SchemeNorm
replacetheme SchemeSel
replacetheme SchemeOut
replacetheme "size="

if [ -z "$2" ]; then
    make
    sudo make install
fi
