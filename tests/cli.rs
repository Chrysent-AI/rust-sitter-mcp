use std::process::Command;

#[test]
fn version_reports_embedded_build_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rust-sitter-mcp {}\n", env!("VERSION_FULL"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_succeeds_and_unknown_options_fail() {
    let help = Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("--help") && text.contains("--version"));
    let invalid = Command::new(env!("CARGO_BIN_EXE_rust-sitter-mcp"))
        .arg("--not-an-option")
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(!invalid.stderr.is_empty());
}
