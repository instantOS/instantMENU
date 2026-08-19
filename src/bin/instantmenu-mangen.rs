//! Generates `instantmenu.1` from the clap `Args` definition.
//!
//! Run with `cargo run --bin instantmenu-mangen` (or `just man`). The output
//! is written to the repository root and committed, so the man page stays a
//! checked-in artifact while its content comes from `src/cli.rs`.

use std::io;
use std::path::Path;

use clap::CommandFactory;
use instantmenu::cli::Args;

fn main() -> io::Result<()> {
    let man = clap_mangen::Man::new(Args::command());

    let mut buffer: Vec<u8> = Default::default();
    man.render(&mut buffer)?;

    // clap_mangen leaves a space at the end of a few generated lines. Keep
    // the checked-in artifact friendly to diff and whitespace checks.
    let rendered = String::from_utf8(buffer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let rendered = rendered
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("instantmenu.1");
    std::fs::write(&out, rendered)?;
    println!("wrote {}", out.display());
    Ok(())
}
