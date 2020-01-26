#!/usr/bin/env bash

./theme.sh "$1"

make clean &>/dev/null

if [ -z "$2" ]; then
    make
    sudo make install
fi
