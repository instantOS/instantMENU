# Emoji picker layout cache benchmark

Measured on 2026-09-30 with the 3,944-entry catalog from
`../instantCLI/src/assist/actions/emoji/catalog.tsv`, an AMD Ryzen 5 3600,
Rust 1.98.1 and the release profile. The benchmark uses the picker's value
markup and 700 px width, 12 rows and 32 px line height. Fonts are DejaVu Sans
and Noto Color Emoji (see the BENCH_FONT_* overrides in the test).

Run from instantMENU with the sibling instantCLI checkout present:

```sh
cargo test --release bench_scroll -- --ignored --nocapture
```

The real menu, text renderer, canvas and paging run against a stub backend.
Display transfer, compositor latency, frecency and subprocess startup are
excluded. Each pass contains 200 scroll frames. Results below are the median
of the reported percentiles across three alternating before/after runs of
the same benchmark, without concurrent builds.

| Pass | Before p50 | After p50 | Before p99 | After p99 |
| --- | ---: | ---: | ---: | ---: |
| Fresh pages, down | 3.88 ms | 3.93 ms | 5.84 ms | 4.83 ms |
| Revisited pages, up | 0.45 ms | 0.19 ms | 2.43 ms | 0.29 ms |
| Second visit, down | 0.45 ms | 0.19 ms | 2.36 ms | 0.29 ms |

Revisited-frame median cost fell about 58%, and p99 about 88%. Fresh emoji
pages still take roughly 4 ms: retaining shaped layouts does not eliminate
first-time color glyph rasterization. These results do not establish an
improvement to end-to-end display latency.

Previously, inserting the 1,025th layout cleared the entire cache. The new
8,192-entry LRU cache retains the catalog plus transient headers, queries and
truncated prefixes. Both measuring and drawing refresh recency, and overflow
evicts only the oldest layout. This permits more retained layout memory than
the previous 1,024-entry limit; the entry count remains bounded.

## Integration with incoming font optimizations

The incoming `9f28469` change separates measured widths from shaped buffers
and improves font fallback discovery/loading. Its buffer cache still clears
at 1,024 entries, and its width cache clears at 4,096 entries.

The merge retains those font changes and the separate width cache, using
8,192-entry LRU storage for layouts and 16,384-entry LRU storage for widths.
Width-only hits do not recreate an evicted layout. Existing width entries
are refreshed without allocating another label string. The independent
X11 event-ordering fix remains in place.

The same benchmark was rerun with the incoming renderer and the combined
renderer, with identical incoming fontconfig code and three alternating
release runs. Values are medians of the reported percentiles.

| Pass | Incoming p50 | Combined p50 | Incoming p99 | Combined p99 |
| --- | ---: | ---: | ---: | ---: |
| Fresh pages, down | 4.03 ms | 4.21 ms | 6.14 ms | 5.61 ms |
| Revisited pages, up | 0.54 ms | 0.27 ms | 3.11 ms | 0.45 ms |
| Second visit, down | 0.55 ms | 0.26 ms | 2.79 ms | 0.45 ms |

The combination cuts revisited-frame median cost roughly in half compared
with the incoming renderer. It does not improve fresh-page median cost in
this measurement. Font fallback changes are shared by both variants; this
comparison does not measure their separate benefit or cold startup.
