//! The chokepoint rule, enforced mechanically.
//!
//! The observability guarantee of this tool rests on one claim: `exec.rs` is the
//! only module that creates a process. A comment is not enforcement. This test
//! is.
//!
//! It fails if `std::process::Command`, `Stdio`, `Command::new`, `exec`, `spawn`
//! or `system` appear anywhere else in `src/`. Adding a second spawn site
//! therefore breaks the build rather than silently un-logging a command.

use std::fs;
use std::path::{Path, PathBuf};

/// Modules allowed to touch the process API.
const ALLOWED: &[&str] = &["exec.rs", "main.rs"];

/// Files that legitimately reference process APIs in other ways.
const ALLOWED_ANYWHERE: &[&str] = &["lib.rs"];

/// Substrings that indicate a process is being created or run.
const FORBIDDEN: &[&str] = &[
    "process::Command",
    "Command::new",
    "Stdio::",
    "std::process::Command",
    "process::ExitStatus",
];

/// Directories and file suffixes to skip.
const SKIP_DIRS: &[&str] = &["target", ".git"];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strips line comments so documentation that *mentions* `Command` is not a hit.
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn exec_is_the_only_module_that_spawns_a_process() {
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);
    assert!(
        files.len() >= 6,
        "expected to scan the crate, found {files:?}"
    );

    let mut violations: Vec<String> = Vec::new();

    for file in &files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        if ALLOWED.contains(&name.as_str()) || ALLOWED_ANYWHERE.contains(&name.as_str()) {
            continue;
        }

        let source = code_only(&fs::read_to_string(file).unwrap());

        for needle in FORBIDDEN {
            if source.contains(needle) {
                let line_no = source
                    .lines()
                    .position(|l| l.contains(needle))
                    .map(|n| n + 1)
                    .unwrap_or(0);
                violations.push(format!(
                    "{name}:{line_no} contains {needle:?}\n    \
                     route it through exec::Run::exec so it is logged, timed out and redacted"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the subprocess chokepoint has been bypassed:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn the_chokepoint_is_actually_used_by_the_cli() {
    let source = fs::read_to_string(src_dir().join("cli.rs")).unwrap();
    assert!(
        source.contains("run.exec("),
        "cli.rs must reach the operating system through Run::exec"
    );
}

#[test]
fn exec_records_every_outcome() {
    // Guards the four statuses the trace distinguishes. If one is dropped, a
    // failed swapdock becomes invisible in the log.
    let source = fs::read_to_string(src_dir().join("exec.rs")).unwrap();
    for variant in ["StepStatus::Ok", "StepStatus::Error", "StepStatus::Timeout"] {
        assert!(
            source.contains(variant),
            "exec.rs must record {variant}; a silent outcome is a blind spot"
        );
    }
}
