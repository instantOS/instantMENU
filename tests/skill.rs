use std::process::{Command, Stdio};

#[test]
fn skill_forms_print_the_embedded_document_without_initializing_the_menu() {
    for argument in ["skill", "--skill"] {
        let output = Command::new(env!("CARGO_BIN_EXE_instantmenu"))
            .arg(argument)
            .args(["--config", "/nonexistent/instantmenu-skill-test.toml"])
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::piped())
            .output()
            .unwrap();

        assert!(output.status.success(), "{argument}: {output:?}");
        assert!(output.stderr.is_empty(), "{argument}: {output:?}");
        assert_eq!(output.stdout, include_bytes!("../src/skill.md"));
    }
}
