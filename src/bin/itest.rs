//! itest — file test utility feeding instantmenu (port of itest.c).
//!
//! Usage: itest [-abcdefghlpqrsuvwx] [-n file] [-o file] [file...]
//! With no file arguments, candidate names are read from stdin.

use std::fs::Metadata;
use std::io::{BufRead, Write};
use std::time::UNIX_EPOCH;

/// itest's flag set, one named field per `-x` option (port of `FLAG(x)`).
#[derive(Default)]
struct Flags {
    /// -a: include hidden files
    all: bool,
    /// -b: block special
    block: bool,
    /// -c: character special
    character: bool,
    /// -d: directory
    directory: bool,
    /// -e: exists
    exists: bool,
    /// -f: regular file
    regular: bool,
    /// -g: set-group-id flag
    set_group_id: bool,
    /// -h: symbolic link
    symlink: bool,
    /// -l: test a directory's contents
    list_dir: bool,
    /// -n: newer than reference file
    newer: bool,
    /// -o: older than reference file
    older: bool,
    /// -p: named pipe
    pipe: bool,
    /// -q: quit on first match
    quiet: bool,
    /// -r: readable
    readable: bool,
    /// -s: not empty
    not_empty: bool,
    /// -u: set-user-id flag
    set_user_id: bool,
    /// -v: invert the result
    invert: bool,
    /// -w: writable
    writable: bool,
    /// -x: executable
    executable: bool,
    /// mtime of the -n reference (newer than file)
    new_mtime: Option<i64>,
    /// mtime of the -o reference (older than file)
    old_mtime: Option<i64>,
}

impl Flags {
    /// Enable the flag named by `flag`; Err on an unknown letter.
    fn set(&mut self, flag: u8) -> Result<(), ()> {
        match flag {
            b'a' => self.all = true,
            b'b' => self.block = true,
            b'c' => self.character = true,
            b'd' => self.directory = true,
            b'e' => self.exists = true,
            b'f' => self.regular = true,
            b'g' => self.set_group_id = true,
            b'h' => self.symlink = true,
            b'l' => self.list_dir = true,
            b'p' => self.pipe = true,
            b'q' => self.quiet = true,
            b'r' => self.readable = true,
            b's' => self.not_empty = true,
            b'u' => self.set_user_id = true,
            b'v' => self.invert = true,
            b'w' => self.writable = true,
            b'x' => self.executable = true,
            _ => return Err(()),
        }
        Ok(())
    }
}

fn usage() -> ! {
    eprintln!("usage: itest [-abcdefghlpqrsuvwx] [-n file] [-o file] [file...]");
    std::process::exit(2); /* like test(1) return > 1 on error */
}

fn mtime(metadata: &Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mode_bits(metadata: &Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    metadata.mode()
}

/// access(2) wrapper (F_OK/R_OK/W_OK/X_OK).
fn access_ok(path: &str, mode: i32) -> bool {
    let c = std::ffi::CString::new(path).unwrap_or_default();
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

/// Port of test(): prints `name` when the path satisfies every given flag
/// (inverted by -v). Returns true when it matched.
fn test(path: &str, name: &str, flags: &Flags, out: &mut impl Write) -> bool {
    /* stat() result — failures make the whole chain false, like C */
    let stat = std::fs::metadata(path).ok();
    let link_stat = std::fs::symlink_metadata(path).ok();

    let mut result = true;
    macro_rules! check {
        ($cond:expr $(,)?) => {
            result = result && $cond
        };
    }

    if let Some(stat) = &stat {
        check!(flags.all || !name.starts_with('.'));
        if flags.block {
            check!(mode_bits(stat) & libc::S_IFMT == libc::S_IFBLK);
        }
        if flags.character {
            check!(mode_bits(stat) & libc::S_IFMT == libc::S_IFCHR);
        }
        if flags.directory {
            check!(stat.is_dir());
        }
        if flags.regular {
            check!(stat.is_file());
        }
        if flags.set_group_id {
            check!(mode_bits(stat) & libc::S_ISGID != 0);
        }
        if flags.newer {
            check!(flags.new_mtime.map(|t| mtime(stat) > t).unwrap_or(false));
        }
        if flags.older {
            check!(flags.old_mtime.map(|t| mtime(stat) < t).unwrap_or(false));
        }
        if flags.pipe {
            check!(mode_bits(stat) & libc::S_IFMT == libc::S_IFIFO);
        }
        if flags.not_empty {
            check!(stat.len() > 0);
        }
        if flags.set_user_id {
            check!(mode_bits(stat) & libc::S_ISUID != 0);
        }
    } else {
        /* stat failed: only -h (lstat) can still make the chain true */
        check!(false);
    }
    if flags.exists {
        check!(access_ok(path, libc::F_OK));
    }
    if flags.symlink {
        check!(link_stat
            .as_ref()
            .map(|m| mode_bits(m) & libc::S_IFMT == libc::S_IFLNK)
            .unwrap_or(false),);
    }
    if flags.readable {
        check!(access_ok(path, libc::R_OK));
    }
    if flags.writable {
        check!(access_ok(path, libc::W_OK));
    }
    if flags.executable {
        check!(access_ok(path, libc::X_OK));
    }

    if result != flags.invert {
        if flags.quiet {
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
    let mut flags = Flags::default();
    let mut files: Vec<String> = Vec::new();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(rest) = arg.strip_prefix('-') else {
            files.push(arg);
            continue;
        };

        /* flags may be bundled (-abc) or take the rest of the arg / the next
         * arg as their value (-n file / -nfile, EARGF semantics) */
        let mut remaining = rest.as_bytes();
        while let Some((&flag, tail)) = remaining.split_first() {
            remaining = tail;
            match flag {
                /* newer/older than file */
                b'n' | b'o' => {
                    let file = if tail.is_empty() {
                        match args.next() {
                            Some(next) => next,
                            None => usage(),
                        }
                    } else {
                        let file = String::from_utf8_lossy(tail).into_owned();
                        remaining = &[];
                        file
                    };
                    match std::fs::metadata(&file) {
                        Ok(metadata) => {
                            let mtime_value = mtime(&metadata);
                            if flag == b'n' {
                                flags.new_mtime = Some(mtime_value);
                                flags.newer = true;
                            } else {
                                flags.old_mtime = Some(mtime_value);
                                flags.older = true;
                            }
                        }
                        Err(e) => {
                            eprintln!("{file}: {e}");
                            if flag == b'n' {
                                flags.newer = false;
                            } else {
                                flags.older = false;
                            }
                        }
                    }
                }
                _ => {
                    if flags.set(flag).is_err() {
                        usage(); /* unknown flag */
                    }
                }
            }
        }
    }

    let mut matched = false;
    if files.is_empty() {
        /* read list from stdin */
        let stdin = std::io::stdin();
        for line in stdin.lock().split(b'\n') {
            let Ok(line) = line else { break };
            let s = String::from_utf8_lossy(&line).into_owned();
            if test(&s, &s, &flags, &mut out) {
                matched = true;
            }
        }
    } else {
        for path in &files {
            /* -l on a directory: test its contents */
            if flags.list_dir && std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let full = format!("{path}/{name}");
                        if test(&full, &name, &flags, &mut out) {
                            matched = true;
                        }
                    }
                }
            } else if test(path, path, &flags, &mut out) {
                matched = true;
            }
        }
    }

    std::process::exit(if matched { 0 } else { 1 });
}
