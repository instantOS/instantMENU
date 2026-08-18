#!/usr/bin/env bash

make clean &>/dev/null

if [ -z "$2" ]; then
    make
    sudo make install
fi
