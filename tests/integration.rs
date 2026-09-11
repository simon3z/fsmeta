//! Integration tests: exercise fsmeta binary as a black box.
//!
//! Tests create temp directories with known files, run `fsmeta dump` on them,
//! then verify output via `fsmeta show`, `fsmeta list`, `fsmeta diff`, and
//! `fsmeta compare`.

use std::process::Command;

fn bin() -> String {
    let exe = std::env::current_exe().unwrap();
    // e.g. target/debug/deps/cli-<hash>
    let mut bin = exe.parent().unwrap().parent().unwrap().to_path_buf();
    bin.push("fsmeta");
    bin.to_string_lossy().to_string()
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fsmeta_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create a unique snapshot path outside the dirs being walked.
fn snap_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("fsmeta_{}_snap.{}", name, std::process::id()))
}

/// Run fsmeta dump, writing output to a file.
fn dump_to(path: &std::path::Path, snapshot: &std::path::Path) {
    let file = std::fs::File::create(snapshot).unwrap();
    let status = Command::new(bin())
        .args(["dump", path.to_str().unwrap()])
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "fsmeta dump failed");
}

fn assert_line_kind(stdout: &str, kind: char, path: &str, label: &str) {
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with(kind) && l.contains(path)),
        "{label}, got:\n{}",
        stdout
    );
}

// No path may appear as both `-` (missing) and `!` (leftover).
fn assert_not_both_missing_and_leftover(stdout: &str) {
    let missing: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with('-'))
        .map(|l| l.trim())
        .collect();
    let leftover: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with('!'))
        .map(|l| l.trim())
        .collect();

    for m in &missing {
        let path: &str = &m[13..]; // skip "-.............  "
        assert!(
            !leftover.iter().any(|l| l.contains(path)),
            "path '{}' is both missing and leftover\nmissing: {}\nleftover: {}",
            path,
            m,
            leftover.iter().find(|l| l.contains(path)).unwrap()
        );
    }
}

fn dump_with_args(args: &[&str], snapshot: &std::path::Path) {
    let file = std::fs::File::create(snapshot).unwrap();
    let status = Command::new(bin())
        .args(args)
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "fsmeta {} failed", args[0]);
}

fn show(snapshot: &std::path::Path, file: &std::path::Path) -> String {
    let output = Command::new(bin())
        .args(["show", snapshot.to_str().unwrap(), file.to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

fn diff(a: &std::path::Path, b: &std::path::Path) -> String {
    let output = Command::new(bin())
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

fn diff_verbose(a: &std::path::Path, b: &std::path::Path) -> String {
    let output = Command::new(bin())
        .args([
            "diff",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--verbose",
        ])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

fn list(snapshot: &std::path::Path, args: &[&str]) -> String {
    let mut cmd = Command::new(bin());
    cmd.arg("list");
    cmd.arg(snapshot);
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().unwrap();
    String::from_utf8(output.stdout).unwrap()
}

// --- dump + show: file types ---

#[test]
fn show_regular_file() {
    let dir = tmpdir("show_reg");
    std::fs::write(dir.join("hello.txt"), b"hello world\n").unwrap();
    let snap = snap_path("show_reg");
    dump_to(&dir, &snap);

    let out = show(&snap, &dir.join("hello.txt"));
    assert!(out.contains("Path:    "));
    assert!(out.contains("hello.txt"));
    assert!(out.contains("Type:    Regular"));
    assert!(out.contains("Size:      12"));
}

#[test]
fn show_symlink() {
    let dir = tmpdir("show_sym");
    std::fs::write(dir.join("target.txt"), b"content").unwrap();
    std::os::unix::fs::symlink("target.txt", dir.join("link.txt")).unwrap();
    let snap = snap_path("show_sym");
    dump_to(&dir, &snap);

    let out = show(&snap, &dir.join("link.txt"));
    assert!(out.contains("Type:    Symlink"));
    assert!(out.contains("Symlink:   -> target.txt"));
}

#[test]
fn show_directory() {
    let dir = tmpdir("show_dir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let snap = snap_path("show_dir");
    dump_to(&dir, &snap);

    let out = show(&snap, &dir);
    assert!(out.contains("Type:    Directory"));
}

// --- checksum behavior ---

#[test]
fn checksum_present_for_small_files() {
    let dir = tmpdir("chk_small");
    std::fs::write(dir.join("small.txt"), b"tiny").unwrap();
    let snap = snap_path("chk_small");
    dump_to(&dir, &snap);

    let out = show(&snap, &dir.join("small.txt"));
    assert!(out.contains("SHA1:"), "expected SHA1, got:\n{}", out);
}

#[test]
fn checksum_skipped_when_over_limit() {
    let dir = tmpdir("chk_large");
    let file = dir.join("large.bin");
    std::fs::write(&file, vec![0u8; 2048]).unwrap();

    let snap = snap_path("chk_large");
    dump_with_args(
        &["dump", "--checksum-size-limit=1024", dir.to_str().unwrap()],
        &snap,
    );

    let out = show(&snap, &file);
    assert!(
        out.contains("(skipped"),
        "expected skipped message, got:\n{}",
        out
    );
}

// --- list ---

#[test]
fn list_all_paths() {
    let dir = tmpdir("list_all");
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    std::fs::write(dir.join("b.txt"), b"y").unwrap();
    let snap = snap_path("list_all");
    dump_to(&dir, &snap);

    let out = list(&snap, &[]);
    assert!(out.contains("a.txt"));
    assert!(out.contains("b.txt"));
}

#[test]
fn list_with_grep_filter() {
    let dir = tmpdir("list_grep");
    std::fs::write(dir.join("alpha.txt"), b"x").unwrap();
    std::fs::write(dir.join("beta.txt"), b"y").unwrap();
    let snap = snap_path("list_grep");
    dump_to(&dir, &snap);

    // Use the full absolute prefix to match only alpha.txt
    let prefix = format!("{}/alpha", dir.to_str().unwrap());
    let out = list(&snap, &["--grep", &prefix]);
    assert!(
        out.lines().any(|l| l.contains("alpha.txt")),
        "expected alpha.txt in: {}",
        out
    );
    assert!(
        !out.lines().any(|l| l.contains("beta.txt")),
        "should not contain beta.txt: {}",
        out
    );
}

#[test]
fn list_count() {
    let dir = tmpdir("list_count");
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    std::fs::write(dir.join("b.txt"), b"y").unwrap();
    let snap = snap_path("list_count");
    dump_to(&dir, &snap);

    // dir itself + a.txt + b.txt = 3
    let out = list(&snap, &["--count"]);
    assert_eq!(out.trim(), "3");
}

// --- diff: no changes ---

#[test]
fn diff_identical_snapshots_empty() {
    let dir = tmpdir("diff_same");
    std::fs::write(dir.join("stable.txt"), b"same").unwrap();

    let snap1 = snap_path("diff_same_1");
    dump_to(&dir, &snap1);

    let snap2 = snap_path("diff_same_2");
    dump_to(&dir, &snap2);

    let out = diff(&snap1, &snap2);
    assert!(out.is_empty(), "expected no diff, got:\n{}", out);
}

// --- diff: size + content change ---

#[test]
fn diff_detects_size_and_content_change() {
    let dir = tmpdir("diff_size");
    std::fs::write(dir.join("f.txt"), b"short").unwrap();
    let snap1 = snap_path("diff_size_1");
    dump_to(&dir, &snap1);

    std::fs::write(dir.join("f.txt"), b"much longer content").unwrap();
    let snap2 = snap_path("diff_size_2");
    dump_to(&dir, &snap2);

    let out = diff(&snap1, &snap2);
    // Should contain a line with 's' at position 1 (size flag)
    assert!(
        out.lines()
            .any(|l| { l.len() > 14 && l.as_bytes()[1] == b's' }),
        "expected size flag, got:\n{}",
        out
    );
}

// --- diff: leftover and missing ---

#[test]
fn diff_detects_missing_file() {
    let dir = tmpdir("diff_missing");
    std::fs::write(dir.join("original.txt"), b"v1").unwrap();
    std::fs::write(dir.join("doomed.txt"), b"bye").unwrap();
    let snap1 = snap_path("diff_missing_1");
    dump_to(&dir, &snap1);

    std::fs::remove_file(dir.join("doomed.txt")).unwrap();
    let snap2 = snap_path("diff_missing_2");
    dump_to(&dir, &snap2);

    let out = diff(&snap1, &snap2);
    assert!(
        out.lines()
            .any(|l| l.starts_with('-') && l.contains("doomed.txt")),
        "expected missing file marker, got:\n{}",
        out
    );
}

#[test]
fn diff_detects_leftover_file() {
    let dir = tmpdir("diff_leftover");
    std::fs::write(dir.join("original.txt"), b"v1").unwrap();
    let snap1 = snap_path("diff_leftover_1");
    dump_to(&dir, &snap1);

    std::fs::write(dir.join("new.txt"), b"new file").unwrap();
    let snap2 = snap_path("diff_leftover_2");
    dump_to(&dir, &snap2);

    let out = diff(&snap1, &snap2);
    assert!(
        out.lines()
            .any(|l| l.starts_with('!') && l.contains("new.txt")),
        "expected leftover file marker, got:\n{}",
        out
    );
}

// --- diff: verbose mode ---

#[test]
fn diff_verbose_shows_mode_change() {
    let dir = tmpdir("diff_verbose_mode");
    std::fs::write(dir.join("f.txt"), b"data").unwrap();
    let snap1 = snap_path("diff_verbose_1");
    dump_to(&dir, &snap1);

    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.join("f.txt"), Permissions::from_mode(0o600)).unwrap();

    let snap2 = snap_path("diff_verbose_2");
    dump_to(&dir, &snap2);

    let out = diff_verbose(&snap1, &snap2);
    assert!(
        out.contains("M:"),
        "expected mode detail in verbose output, got:\n{}",
        out
    );
}

// --- exclude flag ---

#[test]
fn dump_respects_exclude() {
    let dir = tmpdir("exclude");
    std::fs::create_dir_all(dir.join("keep")).unwrap();
    std::fs::create_dir_all(dir.join("skip")).unwrap();
    std::fs::write(dir.join("keep/a.txt"), b"a").unwrap();
    std::fs::write(dir.join("skip/b.txt"), b"b").unwrap();
    let snap = snap_path("exclude");

    // Use the absolute path to the skip subdir
    dump_with_args(
        &[
            "dump",
            &format!("--exclude={}", dir.join("skip").to_str().unwrap()),
            dir.to_str().unwrap(),
        ],
        &snap,
    );

    let out = list(&snap, &[]);
    assert!(out.contains("a.txt"));
    assert!(!out.contains("b.txt"));
}

// --- stdin mode ---

#[test]
fn dump_from_stdin() {
    let dir = tmpdir("stdin");
    std::fs::write(dir.join("f1.txt"), b"one").unwrap();
    std::fs::write(dir.join("f2.txt"), b"two").unwrap();
    let snap = snap_path("stdin");

    // --stdin reads paths from stdin but still requires a positional arg
    let file = std::fs::File::create(&snap).unwrap();
    let mut child = Command::new(bin())
        .args(["dump", "--stdin", dir.to_str().unwrap()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    use std::io::Write;
    // Feed just one file path via stdin
    let input = format!("{}\n", dir.join("f1.txt").to_str().unwrap());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    drop(child.stdin.take());

    let status = child.wait().unwrap();
    assert!(status.success(), "fsmeta dump --stdin failed");

    let out = list(&snap, &[]);
    assert!(out.contains("f1.txt"));
    assert!(!out.contains("f2.txt"));
}

// --- compare: live filesystem ---

#[test]
fn compare_no_changes_empty_output() {
    let dir = tmpdir("cmp_same");
    std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
    std::fs::write(dir.join("b.txt"), b"beta").unwrap();
    let snap = snap_path("cmp_same");
    dump_to(&dir, &snap);

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compare failed: {:?}",
        output.stderr
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.is_empty(), "expected no diff, got:\n{}", stdout);
}

#[test]
fn compare_detects_missing_file() {
    let dir = tmpdir("cmp_missing");
    std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
    std::fs::write(dir.join("doomed.txt"), b"bye").unwrap();
    let snap = snap_path("cmp_missing");
    dump_to(&dir, &snap);

    // Remove a file after snapshot
    std::fs::remove_file(dir.join("doomed.txt")).unwrap();

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('-') && l.contains("doomed.txt")),
        "expected missing marker, got:\n{}",
        stdout
    );
    // Should NOT have a leftover marker for doomed.txt
    assert!(
        !stdout
            .lines()
            .any(|l| l.starts_with('!') && l.contains("doomed.txt")),
        "should not be leftover, got:\n{}",
        stdout
    );
}

#[test]
fn compare_detects_leftover_file() {
    let dir = tmpdir("cmp_leftover");
    std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
    let snap = snap_path("cmp_leftover");
    dump_to(&dir, &snap);

    // Add a file after snapshot
    std::fs::write(dir.join("new.txt"), b"new").unwrap();

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('!') && l.contains("new.txt")),
        "expected leftover marker, got:\n{}",
        stdout
    );
    // Should NOT have a missing marker for new.txt
    assert!(
        !stdout
            .lines()
            .any(|l| l.starts_with('-') && l.contains("new.txt")),
        "should not be missing, got:\n{}",
        stdout
    );
}

#[test]
fn compare_detects_content_change() {
    let dir = tmpdir("cmp_content");
    std::fs::write(dir.join("f.txt"), b"v1").unwrap();
    let snap = snap_path("cmp_content");
    dump_to(&dir, &snap);

    // Change content
    std::fs::write(dir.join("f.txt"), b"v2-different").unwrap();

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Should have a change line (space prefix, with 's' or 'C' flag)
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with(" ") && l.contains("f.txt")),
        "expected change marker, got:\n{}",
        stdout
    );
}

#[test]
fn compare_subset_path_no_cross_tree_noise() {
    // Create a tree with two subdirectories
    let dir = tmpdir("cmp_subset");
    std::fs::create_dir_all(dir.join("alpha")).unwrap();
    std::fs::create_dir_all(dir.join("beta")).unwrap();
    std::fs::write(dir.join("alpha/a.txt"), b"A").unwrap();
    std::fs::write(dir.join("beta/b.txt"), b"B").unwrap();

    // Snapshot the whole tree
    let snap = snap_path("cmp_subset");
    dump_to(&dir, &snap);

    // Compare only the alpha subdirectory
    let alpha = dir.join("alpha");
    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), alpha.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compare failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Should NOT report beta/b.txt as missing (it's outside the subset)
    assert!(
        !stdout.contains("beta"),
        "should not report files outside subset, got:\n{}",
        stdout
    );
    // Should NOT report alpha files as missing or leftover (they're unchanged)
    assert!(
        stdout.lines().all(|l| !l.contains("a.txt")),
        "should not flag unchanged files, got:\n{}",
        stdout
    );
}

#[test]
fn compare_subset_path_detects_missing_in_subset() {
    let dir = tmpdir("cmp_subset_missing");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/a.txt"), b"A").unwrap();
    std::fs::write(dir.join("sub/b.txt"), b"B").unwrap();

    let snap = snap_path("cmp_subset_missing");
    dump_to(&dir, &snap);

    // Remove a file inside the subset
    std::fs::remove_file(dir.join("sub/b.txt")).unwrap();

    let sub = dir.join("sub");
    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), sub.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('-') && l.contains("b.txt")),
        "expected missing in subset, got:\n{}",
        stdout
    );
    // a.txt should NOT be reported
    assert!(
        !stdout.lines().any(|l| l.contains("a.txt")),
        "should not flag unchanged, got:\n{}",
        stdout
    );
}

// --- compare: invariant tests ---

/// A path must NEVER appear as both `-` (missing) and `!` (leftover) in the
/// same compare output. This would indicate the merge logic is broken.
#[test]
fn compare_no_path_is_both_missing_and_leftover() {
    let dir = tmpdir("cmp_invariant");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), b"1").unwrap();
    std::fs::write(dir.join("sub/b.txt"), b"2").unwrap();
    std::fs::write(dir.join("sub/c.txt"), b"3").unwrap();

    let snap = snap_path("cmp_invariant");
    dump_to(&dir, &snap);

    // Remove one, add one, modify one
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    std::fs::write(dir.join("sub/d.txt"), b"4").unwrap();
    std::fs::write(dir.join("sub/c.txt"), b"3-changed").unwrap();

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // No path may appear as both - and !
    assert_not_both_missing_and_leftover(&stdout);

    // Specific expectations
    assert_line_kind(&stdout, '-', "a.txt", "a.txt should be missing");
    assert_line_kind(&stdout, '!', "d.txt", "d.txt should be leftover");
    assert_line_kind(&stdout, ' ', "c.txt", "c.txt should be changed");
}

/// When comparing a snapshot against itself (no changes), output must be empty.
/// This catches any merge logic that would emit spurious - or ! lines.
#[test]
fn compare_snapshot_vs_itself_is_empty() {
    let dir = tmpdir("cmp_self");
    std::fs::create_dir_all(dir.join("deep/nested/dir")).unwrap();
    std::fs::write(dir.join("root.txt"), b"root").unwrap();
    std::fs::write(dir.join("deep/nested/dir/file.bin"), [0u8; 100]).unwrap();
    std::fs::write(dir.join("deep/nested/empty.txt"), b"").unwrap();

    let snap = snap_path("cmp_self");
    dump_to(&dir, &snap);

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.is_empty(),
        "comparing snapshot against itself should produce no output, got:\n{}",
        stdout
    );
}

/// Simulate a file that is in the baseline but the live walk cannot reach.
/// It should appear as `-` (missing) but NOT also as `!` (leftover).
#[test]
fn compare_file_in_baseline_but_not_in_live() {
    let dir = tmpdir("cmp_base_only");
    std::fs::write(dir.join("exists.txt"), b"exists").unwrap();
    std::fs::write(dir.join("ghost.txt"), b"ghost").unwrap();

    let snap = snap_path("cmp_base_only");
    dump_to(&dir, &snap);

    // Remove ghost.txt so it's in baseline but not on disk
    std::fs::remove_file(dir.join("ghost.txt")).unwrap();

    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // ghost.txt should be missing
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with('-') && l.contains("ghost.txt")),
        "ghost.txt should be missing, got:\n{}",
        stdout
    );
    // ghost.txt must NOT be leftover
    assert!(
        !stdout
            .lines()
            .any(|l| l.starts_with('!') && l.contains("ghost.txt")),
        "ghost.txt must not be leftover, got:\n{}",
        stdout
    );
    // exists.txt should not be flagged
    assert!(
        !stdout.lines().any(|l| l.contains("exists.txt")),
        "exists.txt should not be flagged, got:\n{}",
        stdout
    );
}

/// Compare against a subdirectory of the snapshot: only that subtree should
/// be considered. No spurious missing or leftover for the subset root itself.
#[test]
fn compare_subset_no_spurious_entries() {
    let dir = tmpdir("cmp_subset_spurious");
    std::fs::create_dir_all(dir.join("x")).unwrap();
    std::fs::create_dir_all(dir.join("y")).unwrap();
    std::fs::write(dir.join("x/f1.txt"), b"f1").unwrap();
    std::fs::write(dir.join("y/f2.txt"), b"f2").unwrap();

    let snap = snap_path("cmp_subset_spurious");
    dump_to(&dir, &snap);

    // Compare against the x/ subdirectory only
    let x = dir.join("x");
    let output = Command::new(bin())
        .args(["compare", snap.to_str().unwrap(), x.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    // f1.txt should NOT be flagged (unchanged)
    assert!(
        !stdout.lines().any(|l| l.contains("f1.txt")),
        "f1.txt should not be flagged, got:\n{}",
        stdout
    );
    // y/ should NOT appear at all
    assert!(
        !stdout.lines().any(|l| l.contains("y/")),
        "y/ should not appear, got:\n{}",
        stdout
    );
}

// --- snapshot ordering: DFS order with directory vs sibling file ---

/// When a directory "journal" and file "journal.conf" are siblings, the
/// snapshot must be in DFS (pre-order) order: the directory and all its
/// descendants come before the sibling file.
#[test]
fn dump_snapshot_order_with_prefix_collision() {
    let dir = tmpdir("prefix_collision");
    // Create: dir/journal/profile and dir/journal.conf
    std::fs::create_dir_all(dir.join("journal")).unwrap();
    std::fs::write(dir.join("journal/profile"), b"data").unwrap();
    std::fs::write(dir.join("journal.conf"), b"file").unwrap();

    let snap = snap_path("prefix_collision");
    dump_to(&dir, &snap);

    // Use `fsmeta list` to get paths in file order
    let out = list(&snap, &[]);
    let paths: Vec<&str> = out.lines().collect();

    // In DFS order, journal and its contents come before journal.conf
    let journal_dir = paths.iter().position(|p| p.ends_with("/journal"));
    let journal_profile = paths.iter().position(|p| p.ends_with("journal/profile"));
    let journal_conf = paths.iter().position(|p| p.ends_with("journal.conf"));

    assert!(journal_dir.is_some(), "journal dir not found in list");
    assert!(
        journal_profile.is_some(),
        "journal/profile not found in list"
    );
    assert!(journal_conf.is_some(), "journal.conf not found in list");

    // DFS: dir < dir/child < sibling_file
    let (dir_pos, profile_pos, conf_pos) = (
        journal_dir.unwrap(),
        journal_profile.unwrap(),
        journal_conf.unwrap(),
    );
    assert!(
        dir_pos < profile_pos && profile_pos < conf_pos,
        "DFS order violated: journal({}) profile({}) journal.conf({})",
        dir_pos,
        profile_pos,
        conf_pos
    );
}
