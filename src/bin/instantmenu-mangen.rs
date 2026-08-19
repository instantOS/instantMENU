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
    let mut cmd = Args::command();
    cmd.build();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let out = root.join("instantmenu.1");
    std::fs::write(&out, render(&cmd)?)?;
    println!("wrote {}", out.display());

    Ok(())
}

/// Render the top-level command and all its subcommands into a single,
/// comprehensive man page. Trailing spaces per line are stripped so the
/// checked-in artifact stays diff-friendly.
fn render(cmd: &clap::Command) -> io::Result<String> {
    let man = clap_mangen::Man::new(cmd.clone())
        .source(format!("instantmenu {}", instantmenu::config::VERSION));
    let mut buffer: Vec<u8> = Default::default();
    man.render(&mut buffer)?;

    let rendered = String::from_utf8(buffer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    // Build the expanded SUBCOMMANDS section including options for each subcommand
    let mut sub_sections = Vec::new();
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let mut sub_section = format!(".SS instantmenu {}\n", sub.get_name());
        if let Some(about) = sub.get_long_about().or_else(|| sub.get_about()) {
            sub_section.push_str(&about.to_string());
            sub_section.push('\n');
        }

        // Render subcommand-specific options
        let specific_args: Vec<_> = sub
            .get_arguments()
            .filter(|a| !a.is_global_set() && a.get_id() != "help" && a.get_id() != "version")
            .cloned()
            .collect();

        if !specific_args.is_empty() {
            let mut opts_cmd = clap::Command::new("slide");
            for arg in specific_args {
                opts_cmd = opts_cmd.arg(arg);
            }
            let sub_man = clap_mangen::Man::new(opts_cmd);
            let mut sub_buf: Vec<u8> = Default::default();
            sub_man.render(&mut sub_buf)?;
            let sub_rendered = String::from_utf8_lossy(&sub_buf);

            if let Some(options_pos) = sub_rendered.find(".SH OPTIONS") {
                let options_section = &sub_rendered[options_pos + ".SH OPTIONS\n".len()..];
                let options_text = if let Some(end_pos) = options_section.find("\n.SH ") {
                    &options_section[..end_pos]
                } else {
                    options_section.trim_end()
                };
                sub_section.push_str(options_text);
                sub_section.push('\n');
            }
        }
        sub_sections.push(sub_section);
    }

    let result = if !sub_sections.is_empty() {
        let replacement = format!(".SH SUBCOMMANDS\n{}", sub_sections.join("\n"));
        // Replace clap_mangen's default .SH SUBCOMMANDS section
        if let Some(sub_pos) = rendered.find(".SH SUBCOMMANDS") {
            let before = &rendered[..sub_pos];
            let after_sub = &rendered[sub_pos + ".SH SUBCOMMANDS\n".len()..];
            let after = if let Some(next_sh) = after_sub.find("\n.SH ") {
                &after_sub[next_sh..]
            } else {
                ""
            };
            format!("{}{}{}", before, replacement, after)
        } else {
            rendered
        }
    } else {
        rendered
    };

    Ok(result
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
}
