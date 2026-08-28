//! CLI argument parsing tests (via the binary).

fn bin() -> String {
    let exe = std::env::current_exe().unwrap();
    // e.g. target/debug/deps/cli-<hash>
    let mut bin = exe.parent().unwrap().parent().unwrap().to_path_buf();
    bin.push("fsmeta");
    bin.to_string_lossy().to_string()
}

#[test]
fn version_flag() {
    let output = std::process::Command::new(bin())
        .arg("--version")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("fsmeta"));
    assert!(stdout.contains("0.1.0"));
}

#[test]
fn help_flag() {
    let output = std::process::Command::new(bin())
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("dump"));
    assert!(stdout.contains("compare"));
    assert!(stdout.contains("diff"));
    assert!(stdout.contains("show"));
    assert!(stdout.contains("list"));
}

#[test]
fn no_subcommand_fails() {
    let output = std::process::Command::new(bin()).output().unwrap();
    // clap exits with code 2 for missing required subcommand
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn dump_requires_paths() {
    let output = std::process::Command::new(bin())
        .arg("dump")
        .output()
        .unwrap();
    // Should fail because paths are required
    assert!(!output.status.success());
}

#[test]
fn subcommand_help() {
    for cmd in ["dump", "compare", "diff", "show", "list"] {
        let output = std::process::Command::new(bin())
            .args([cmd, "--help"])
            .output()
            .unwrap();
        assert!(output.status.success(), "help for '{}' failed", cmd);
    }
}
