//! Frecency: rank items by how often and how recently they were selected.
//!
//! A selection bumps an item's score to `score · 2^(−age/half-life) + 1`;
//! ranking applies the same decay on read, so uses fade over time without
//! extra writes. The decay math and file format are pure functions (ages
//! passed in) — only [`Frecency::open`] and [`Frecency::record`] touch the
//! filesystem.
//!
//! Cache file format, one entry per line:
//! `<score> <last_used-unix-secs> <escaped-text>` where the text escapes
//! `\` and tab (items cannot contain newlines — they come from stdin
//! lines). Malformed lines are skipped with a warning; a launcher must
//! still launch.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::matcher::Item;

/// Selections halve in weight over this span: apps launched daily stay hot,
/// a one-off experiment fades below [`PRUNE_MIN`] within two weeks.
const HALF_LIFE: Duration = Duration::from_secs(4 * 24 * 60 * 60);
/// Entries whose decayed score drops below this are dropped when the cache
/// is written.
const PRUNE_MIN: f64 = 0.1;

/// The decayed score of an entry that was last used `age` ago.
fn decayed(score: f64, age: Duration) -> f64 {
    score * 2f64.powf(-age.as_secs_f64() / HALF_LIFE.as_secs_f64())
}

/// A selection: decay the old score, then count this use.
fn touched(score: f64, age: Duration) -> f64 {
    decayed(score, age) + 1.0
}

/// Age of an entry with `last_used` as unix seconds; clock skew counts as
/// "just used".
fn age_of(last_used: u64, now: SystemTime) -> Duration {
    now.duration_since(UNIX_EPOCH + Duration::from_secs(last_used))
        .unwrap_or(Duration::ZERO)
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    score: f64,
    last_used: u64,
}

/// The frecency store for one menu run: loaded from the cache file at
/// startup, updated and persisted per selection.
pub(super) struct Frecency {
    path: PathBuf,
    entries: HashMap<String, Entry>,
}

/// Render one cache line; the inverse of [`parse_line`].
fn render_entry(score: f64, last_used: u64, text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(c),
        }
    }
    format!("{score:.6} {last_used} {escaped}")
}

/// Parse one cache line into `(text, entry)`; `None` for malformed input.
fn parse_line(line: &str) -> Option<(String, Entry)> {
    let (score, rest) = line.split_once(' ')?;
    let (last_used, text) = rest.split_once(' ')?;
    let score: f64 = score.parse().ok()?;
    if !score.is_finite() || score < 0.0 {
        return None;
    }
    let last_used: u64 = last_used.parse().ok()?;

    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('t') => unescaped.push('\t'),
                Some('\\') => unescaped.push('\\'),
                _ => return None, /* lone or unknown escape */
            },
            _ => unescaped.push(c),
        }
    }
    if unescaped.is_empty() {
        return None; /* selections are never empty — record() skips them */
    }
    Some((unescaped, Entry { score, last_used }))
}

impl Frecency {
    /// Load the cache at `path`. A missing file is an empty store; anything
    /// else unreadable or corrupt degrades to "start over" with a warning.
    pub(super) fn open(path: &Path) -> Self {
        let mut entries = HashMap::new();
        match fs::read_to_string(path) {
            Ok(contents) => {
                let mut bad = 0usize;
                for line in contents.lines().filter(|l| !l.is_empty()) {
                    match parse_line(line) {
                        Some((text, entry)) => {
                            entries.insert(text, entry);
                        }
                        None => bad += 1,
                    }
                }
                if bad > 0 {
                    eprintln!("instantmenu: skipping {bad} malformed frecency lines in {path:?}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("instantmenu: cannot read frecency cache {path:?}: {e}"),
        }
        Frecency {
            path: path.to_path_buf(),
            entries,
        }
    }

    /// Reorder selectable items best-frecency first within each heading
    /// section. Headings remain fixed boundaries, so ranking can never detach
    /// a section title from its entries. Ties and unseen items are stable.
    pub(super) fn rank(&self, items: &mut [Item], now: SystemTime) {
        let mut start = 0;
        while start < items.len() {
            if items[start].entry.is_heading() {
                start += 1;
                continue;
            }
            let end = items[start..]
                .iter()
                .position(|item| item.entry.is_heading())
                .map_or(items.len(), |offset| start + offset);
            items[start..end].sort_by(|a, b| {
                let sa = self.score_of(a.output(), now);
                let sb = self.score_of(b.output(), now);
                sb.total_cmp(&sa)
            });
            start = end;
        }
    }

    fn score_of(&self, text: &str, now: SystemTime) -> f64 {
        self.entries
            .get(text)
            .map_or(0.0, |e| decayed(e.score, age_of(e.last_used, now)))
    }

    /// Count a selection and persist. Empty lines are not selections; the
    /// caller's `now` doubles as the new `last_used`.
    pub(super) fn record(&mut self, text: &str, now: SystemTime) {
        if text.is_empty() {
            return;
        }
        let old = self.entries.get(text).cloned();
        let score = touched(
            old.as_ref().map_or(0.0, |e| e.score),
            old.as_ref()
                .map_or(Duration::ZERO, |e| age_of(e.last_used, now)),
        );
        let last_used = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.entries
            .insert(text.to_string(), Entry { score, last_used });

        /* prune faded entries — this is the only place they are forgotten */
        self.entries
            .retain(|_, e| decayed(e.score, age_of(e.last_used, now)) >= PRUNE_MIN);
        self.write();
    }

    /// Write the cache atomically (tmp file + rename), keys sorted so the
    /// file is stable across runs. Concurrent menus race last-write-wins,
    /// like the old shell history did.
    fn write(&self) {
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        let mut contents = String::new();
        for key in keys {
            let e = &self.entries[key];
            contents.push_str(&render_entry(e.score, e.last_used, key));
            contents.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent); /* write reports the failure */
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        if let Err(e) = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, &self.path)) {
            eprintln!(
                "instantmenu: cannot write frecency cache {:?}: {e}",
                self.path
            );
        }
    }
}

/// Resolve a `--frecency-cache` value: an absolute path is the cache file
/// itself, anything else is an ID under `<cache-root>/instantmenu/`.
/// `root` is the resolvable cache root (`None` = neither XDG_CACHE_HOME
/// nor HOME is set).
fn resolve_under(value: &Path, root: Option<&Path>) -> Result<PathBuf, String> {
    if value.as_os_str().is_empty() {
        return Err("empty frecency cache ID".to_string());
    }
    if value.is_absolute() {
        return Ok(value.to_path_buf());
    }
    let Some(root) = root else {
        return Err("cannot locate a cache directory: set XDG_CACHE_HOME or HOME".to_string());
    };
    Ok(root.join("instantmenu").join(value))
}

/// Resolve a `--frecency-cache` value against the XDG cache root:
/// `$XDG_CACHE_HOME` when absolute, else `$HOME/.cache`.
pub fn resolve_cache_path(value: &Path) -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|v| v.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        });
    resolve_under(value, root.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(n)
    }

    fn store(entries: &[(&str, f64, u64)]) -> Frecency {
        Frecency {
            path: PathBuf::from("/nonexistent-frecency-test"),
            entries: entries
                .iter()
                .map(|(text, score, last_used)| {
                    (
                        (*text).to_string(),
                        Entry {
                            score: *score,
                            last_used: *last_used,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Scores halve at the half-life; a selection decays then adds one use.
    #[test]
    fn decay_and_touch_math() {
        assert_eq!(decayed(8.0, Duration::ZERO), 8.0);
        assert!((decayed(8.0, HALF_LIFE) - 4.0).abs() < 1e-9);
        assert_eq!(touched(0.0, Duration::ZERO), 1.0);
        assert!((touched(1.0, HALF_LIFE) - 1.5).abs() < 1e-9);
    }

    /// Future timestamps (clock skew) count as just used.
    #[test]
    fn clock_skew_is_zero_age() {
        assert_eq!(age_of(10_000_000, secs(0)), Duration::ZERO);
    }

    /// Fresh light use beats faded heavy use; unseen items sort last, ties
    /// and unseen keep stdin order.
    #[test]
    fn rank_orders_by_decayed_score() {
        let day = 24 * 60 * 60;
        // stale: 3.0 three days (0.75 half-lives) ago → 1.78
        // fresh: 2.0 now → 2.0
        let f = store(&[("stale", 3.0, 10 * day - 3 * day), ("fresh", 2.0, 10 * day)]);
        let mut items: Vec<Item> = ["stale", "fresh", "unseen"]
            .iter()
            .map(|s| Item::new(*s))
            .collect();
        f.rank(&mut items, secs(10 * day));
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, vec!["fresh", "stale", "unseen"]);
    }

    #[test]
    fn rank_preserves_heading_sections() {
        let f = store(&[("beta", 1.0, 100), ("delta", 2.0, 100)]);
        let mut items: Vec<Item> = [
            "{heading} First",
            "alpha",
            "beta",
            "{heading} Second",
            "gamma",
            "delta",
        ]
        .iter()
        .map(|text| Item::new(*text))
        .collect();

        f.rank(&mut items, secs(100));
        let labels: Vec<&str> = items.iter().map(Item::label).collect();
        assert_eq!(
            labels,
            vec!["First", "beta", "alpha", "Second", "delta", "gamma"]
        );
    }

    /// Render/parse roundtrip, including tabs and backslashes in the text.
    #[test]
    fn line_format_roundtrips() {
        for text in ["firefox", "sh -c 'echo hi'", "tab\there", "back\\slash"] {
            let line = render_entry(1.5, 1234, text);
            let (parsed, entry) = parse_line(&line).unwrap();
            assert_eq!(parsed, text);
            assert_eq!(
                entry,
                Entry {
                    score: 1.5,
                    last_used: 1234
                }
            );
        }
    }

    /// Malformed lines are rejected: missing fields, non-numeric values,
    /// negative or NaN scores, lone or unknown escapes.
    #[test]
    fn parse_rejects_malformed_lines() {
        for line in [
            "",
            "onlytext",
            "1.0 2",
            "1.0 2 ",
            "abc 2 x",
            "1.0 abc x",
            "-1.0 2 x",
            "NaN 2 x",
            "1.0 2 lone\\",
            "1.0 2 bad\\q",
        ] {
            assert_eq!(parse_line(line), None, "{line:?}");
        }
    }

    /// Recording counts a selection and prunes faded entries.
    #[test]
    fn record_counts_and_prunes() {
        let now = secs(100 * 24 * 60 * 60);
        // ghost: 1.0 a hundred days (25 half-lives) ago ≈ 0 — below PRUNE_MIN
        let mut f = store(&[("ghost", 1.0, 0)]);
        f.record("alpha", now);
        assert_eq!(
            f.entries.get("alpha"),
            Some(&Entry {
                score: 1.0,
                last_used: 100 * 24 * 60 * 60
            })
        );
        assert!(!f.entries.contains_key("ghost"));
    }

    /// Empty lines are not selections and trigger no write.
    #[test]
    fn record_ignores_empty() {
        let mut f = store(&[]);
        f.record("", secs(5));
        assert!(f.entries.is_empty());
    }

    /// Open + write roundtrip through a real file.
    #[test]
    fn file_roundtrip() {
        let path =
            std::env::temp_dir().join(format!("instantmenu-frecency-unit-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let now = secs(1000);
        let mut f = Frecency::open(&path);
        f.record("alpha beta", now);
        f.record("alpha beta", now);
        let reopened = Frecency::open(&path);
        assert_eq!(
            reopened.entries.get("alpha beta"),
            Some(&Entry {
                score: 2.0,
                last_used: 1000
            })
        );
        let _ = fs::remove_file(&path);
    }

    /// Cache IDs land under <root>/instantmenu/<id>; absolute paths pass
    /// through; a missing root or empty ID is an error.
    #[test]
    fn resolve_ids_and_paths() {
        let root = Path::new("/xdg-cache");
        assert_eq!(
            resolve_under(Path::new("apps"), Some(root)).unwrap(),
            PathBuf::from("/xdg-cache/instantmenu/apps")
        );
        assert_eq!(
            resolve_under(Path::new("/tmp/absolute"), Some(root)).unwrap(),
            PathBuf::from("/tmp/absolute")
        );
        assert_eq!(
            resolve_under(Path::new("/tmp/absolute"), None).unwrap(),
            PathBuf::from("/tmp/absolute")
        );
        assert!(resolve_under(Path::new("apps"), None).is_err());
        assert!(resolve_under(Path::new(""), Some(root)).is_err());
    }

    /// A record into a not-yet-existing directory creates it (IDs map to
    /// <cache-root>/instantmenu/<id>, which starts absent).
    #[test]
    fn write_creates_missing_directory() {
        let dir =
            std::env::temp_dir().join(format!("instantmenu-frecency-dirs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut f = Frecency::open(&dir.join("nested").join("cache"));
        f.record("alpha", secs(50));
        assert!(dir.join("nested").join("cache").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rank_distinguishes_values_with_same_label() {
        // same label, different values have distinct keys
        let f = store(&[("/tmp/a", 2.0, 100), ("/tmp/b", 1.0, 100)]);
        let mut items: Vec<Item> = ["{value=/tmp/b} Report", "{value=/tmp/a} Report", "Other"]
            .iter()
            .map(|s| Item::new(*s))
            .collect();
        f.rank(&mut items, secs(100));
        // /tmp/a has higher score, so its item should come first
        assert_eq!(items[0].output(), "/tmp/a");
        assert_eq!(items[1].output(), "/tmp/b");
        // plain item falls back to label
        let g = store(&[("plain", 5.0, 100)]);
        let mut plain_items: Vec<Item> = ["plain", "other"].iter().map(|s| Item::new(*s)).collect();
        g.rank(&mut plain_items, secs(100));
        assert_eq!(plain_items[0].output(), "plain");
    }
}
