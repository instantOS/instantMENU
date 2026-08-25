#!/usr/bin/env python3
"""Regenerate src/icons/names.bin from the official Nerd Fonts glyph names.

Usage:
    utils/gen_icons.py [glyphnames.json] > src/icons/names.bin
    utils/gen_icons.py <url-or-path>     # fetch from the nerd-fonts repo

Without an argument the glyph names are downloaded from the nerd-fonts
master branch. The JSON is the one published for the cheat sheet
(https://www.nerdfonts.com/cheat-sheet) and is MIT licensed.

Two packed tables are emitted, both sorted by key and meant for binary search:

- QUALIFIED: every glyph under its full "<family><name>" key
  (e.g. "md-power_off" -> "mdpower"), the "nf-" prefix already stripped.
- BARE: the same glyphs under their family-less name ("poweroff"), with
  collisions between sets resolved by FAMILY_PREFERENCE (Material Design
  first: the largest and most consistent set, so unqualified names keep
  a uniform look inside a menu).

Normalization matches icons::normalize on the Rust side: lowercase, keep
[ae0-9] alphanumerics only, drop every separator.
"""

import json
import struct
import sys
import urllib.request

DEFAULT_URL = (
    "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/master/glyphnames.json"
)

# Which set wins when the same bare name exists in several (see module doc).
FAMILY_PREFERENCE = [
    "md",        # Material Design Icons — largest, most consistent set
    "fa",        # Font Awesome (classic names people know)
    "cod",       # VS Code codicons
    "oct",       # GitHub octicons
    "dev",       # Devicons
    "seti",
    "linux",
    "fae",
    "custom",
    "ple",
    "pom",
    "pl",
    "iec",
    "weather",
    "indent",
    "indentation",
    "extra",
]
RANK = {family: rank for rank, family in enumerate(FAMILY_PREFERENCE)}


def normalize(name: str) -> str:
    return "".join(c for c in name.lower() if c.isalnum())


def load(source: str | None):
    if source is None:
        source = DEFAULT_URL
    if source.startswith(("http://", "https://")):
        with urllib.request.urlopen(source) as response:
            return json.load(response)
    with open(source) as file:
        return json.load(file)


def packed_table(pairs):
    """Encode names in one blob plus compact (offset, codepoint) records.

    A Rust (&str, u32) tuple occupies 24 bytes on a 64-bit target before the
    string bytes. Offsets keep each record at 8 bytes; NUL terminators let
    lookup recover a name without also storing its length.
    """
    records = bytearray()
    names = bytearray()
    offset = 0
    count = 0
    for key, code in pairs:
        encoded = key.encode("ascii") + b"\0"
        records += struct.pack("<II", offset, code)
        names += encoded
        offset += len(encoded)
        count += 1
    return count, bytes(records), bytes(names)


def main() -> None:
    data = load(sys.argv[1] if len(sys.argv) > 1 else None)
    data.pop("METADATA")

    qualified = {}
    bare = {}
    for key, value in data.items():
        code = int(value["code"], 16)
        family, _, name = key.partition("-")
        qualified_key = normalize(key)
        # the old "nf-" prefixed spelling (Nerd Fonts v2 naming) normalizes
        # to the same key, so both spellings share one entry
        qualified[qualified_key.removeprefix("nf")] = code

        bare_key = normalize(name)
        current = bare.get(bare_key)
        if current is None or RANK.get(family, 99) < RANK.get(current[1], 99):
            bare[bare_key] = (code, family)

    qualified_table = packed_table(sorted(qualified.items()))
    bare_table = packed_table(sorted((name, code) for name, (code, _) in bare.items()))
    q_count, q_records, q_names = qualified_table
    b_count, b_records, b_names = bare_table

    # Header: magic, then count/record-bytes/name-bytes for each table.
    header = struct.pack(
        "<4sIIIIII",
        b"IMIC",
        q_count,
        len(q_records),
        len(q_names),
        b_count,
        len(b_records),
        len(b_names),
    )
    sys.stdout.buffer.write(header + q_records + q_names + b_records + b_names)


if __name__ == "__main__":
    main()
