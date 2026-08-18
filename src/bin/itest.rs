//! itest — file test utility feeding instantmenu (port of itest.c).
//!
//! Usage: itest [-abcdefghlpqrsuvwx] [-n file] [-o file] [file...]
//! With no file arguments, candidate names are read from stdin.

use std::fs::Metadata;
use std::io::{BufRead, Write};
use std::time::UNIX_EPOCH;

/// itest's flag storage: FLAG(x) = flag[x - 'a']
struct Flags {
    flag: [bool; 26],
    /// mtime of the -n reference (newer than file)
    new_mtime: Option<i64>,
    /// mtime of the -o reference (older than file)
    old_mtime: Option<i64>,
}

impl Flags {
    fn get(&self, c: u8) -> bool {
        self.flag[(c - b'a') as usize]
    }

    fn set(&mut self, c: u8, v: bool) {
        self.flag[(c - b'a') as usize] = v;
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: itest [-abcdefghlpqrsuvwx] [-n file] [-o file] [file...]"
    );
    std::process::exit(2); /* like test(1) return > 1 on error */
}

fn mtime(md: &Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mode_bits(md: &Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    md.mode()
}

/// access(2) wrapper (F_OK/R_OK/W_OK/X_OK).
fn access_ok(path: &str, mode: i32) -> bool {
    let c = std::ffi::CString::new(path).unwrap_or_default();
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

/// Port of test(): prints `name` when the path satisfies every given flag
/// (inverted by -v). Returns true when it matched.
fn test(path: &str, name: &str, f: &Flags, out: &mut impl Write) -> bool {
    /* stat() result — failures make the whole chain false, like C */
    let st = std::fs::metadata(path).ok();
    let ln = std::fs::symlink_metadata(path).ok();

    let mut result = true;
    macro_rules! check {
        ($cond:expr $(,)?) => {
            result = result && $cond
        };
    }

    if let Some(st) = &st {
        if f.get(b'a') || !name.starts_with('.') {
            check!(true); /* hidden files */
        } else {
            check!(false);
        }
        if f.get(b'b') {
            check!(mode_bits(st) & libc::S_IFMT == libc::S_IFBLK); /* block special */
        }
        if f.get(b'c') {
            check!(mode_bits(st) & libc::S_IFMT == libc::S_IFCHR); /* character special */
        }
        if f.get(b'd') {
            check!(st.is_dir()); /* directory */
        }
        if f.get(b'g') {
            check!(mode_bits(st) & libc::S_ISGID != 0); /* set-group-id flag */
        }
        if f.get(b'n') {
            /* newer than file */
            check!(f.new_mtime.map(|t| mtime(st) > t).unwrap_or(false));
        }
        if f.get(b'o') {
            /* older than file */
            check!(f.old_mtime.map(|t| mtime(st) < t).unwrap_or(false));
        }
        if f.get(b'p') {
            check!(mode_bits(st) & libc::S_IFMT == libc::S_IFIFO); /* named pipe */
        }
        if f.get(b's') {
            check!(st.len() > 0); /* not empty */
        }
        if f.get(b'u') {
            check!(mode_bits(st) & libc::S_ISUID != 0); /* set-user-id flag */
        }
    } else {
        /* stat failed: only -h (lstat) can still make the chain true */
        check!(false);
    }
    if f.get(b'e') {
        check!(access_ok(path, libc::F_OK)); /* exists */
    }
    if f.get(b'h') {
        check!(
            /* symbolic link */
            ln.as_ref()
                .map(|m| mode_bits(m) & libc::S_IFMT == libc::S_IFLNK)
                .unwrap_or(false),
        );
    }
    if f.get(b'r') {
        check!(access_ok(path, libc::R_OK)); /* readable */
    }
    if f.get(b'w') {
        check!(access_ok(path, libc::W_OK)); /* writable */
    }
    if f.get(b'x') {
        check!(access_ok(path, libc::X_OK)); /* executable */
    }

    if result != f.get(b'v') {
        if f.get(b'q') {
            std::process::exit(0);
        }
        println!("{name}");
        let _ = out.flush();
        return true;
    }
    false
}

fn main() {
    /* die silently on a closed pipe like the C version (| head etc.) */
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut f = Flags { flag: [false; 26], new_mtime: None, old_mtime: None };
    let mut files: Vec<String> = Vec::new();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        let Some(rest) = arg.strip_prefix('-') else {
            files.push(arg.clone());
            i += 1;
            continue;
        };
        let bytes: Vec<u8> = rest.bytes().collect();
        let mut j = 0;
        while j < bytes.len() {
            let c = bytes[j];
            j += 1;
            match c {
                /* newer/older than file: value is the rest of this arg or the
                 * next one (EARGF semantics) */
                b'n' | b'o' => {
                    let file: String = if j < bytes.len() {
                        let s = String::from_utf8_lossy(&bytes[j..]).into_owned();
                        j = bytes.len();
                        s
                    } else {
                        i += 1;
                        if i >= argv.len() {
                            usage();
                        }
                        argv[i].clone()
                    };
                    match std::fs::metadata(&file) {
                        Ok(md) => {
                            let t = mtime(&md);
                            if c == b'n' {
                                f.new_mtime = Some(t);
                            } else {
                                f.old_mtime = Some(t);
                            }
                            f.set(c, true);
                        }
                        Err(e) => {
                            eprintln!("{file}: {e}");
                            f.set(c, false);
                        }
                    }
                }
                _ => {
                    if b"abcdefghlpqrsuvwx".contains(&c) {
                        f.set(c, true);
                    } else {
                        usage(); /* unknown flag */
                    }
                }
            }
        }
        i += 1;
    }

    let mut matched = false;
    if files.is_empty() {
        /* read list from stdin */
        let stdin = std::io::stdin();
        for line in stdin.lock().split(b'\n') {
            let Ok(line) = line else { break };
            let s = String::from_utf8_lossy(&line).into_owned();
            if test(&s, &s, &f, &mut out) {
                matched = true;
            }
        }
    } else {
        for path in &files {
            /* -l on a directory: test its contents */
            if f.get(b'l') && std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let full = format!("{path}/{name}");
                        if test(&full, &name, &f, &mut out) {
                            matched = true;
                        }
                    }
                }
            } else if test(path, path, &f, &mut out) {
                matched = true;
            }
        }
    }

    std::process::exit(if matched { 0 } else { 1 });
}
