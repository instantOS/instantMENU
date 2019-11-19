#!/usr/bin/env bash

rm config.h
make clean
THEME=themes/${1:-dracula}.theme
grep -q 'font' <$THEME || exit 1

replacetheme() {
    themecolor=$(grep "$1" <$THEME)
    sed -i 's/.*'"$1"'.*/'"$themecolor"'./' config.def.h
}

replacetheme SchemeNorm
replacetheme SchemeSel
replacetheme SchemeOut
replacetheme "size="

make
sudo make install
