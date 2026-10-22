//! A real backup → verify → offsite → restore drill with SQLite.
//!
//! This is intentionally not a mocked argv test: it creates a database, backs it
//! up, destroys the live rows, restores from the snapshot, and checks the rows
//! are back. It exercises the same `run_backup`/`run_restore` paths the CLI and
//! service use. Skips when `sqlite3` or `rsync` is unavailable.

use std::path::{Path, PathBuf};
use std::process::Command;

use swapdock::backup::{run_backup, run_restore};
use swapdock::registry::App;
use swapdock::trace::{Run, RunMode, RunStatus, TraceLog};
use tempfile::TempDir;

fn tool(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn sqlite(db: &Path, sql: &str) -> String {
    let output = Command::new("sqlite3")
        .arg(db)
        .arg(sql)
        .output()
        .expect("sqlite3 must run");
    assert!(
        output.status.success(),
        "sqlite3 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn app(db: &Path, backup_dir: &Path, offsite: &Path) -> App {
    let offsite_arg = format!("{}/", offsite.display());
    let text = format!(
        r#"
name = "shop"
kind = "container"
strategy = "replace"
hostnames = ["shop.test"]
listen = ["127.0.0.1"]
front_port = 8001
slot = 0
writes_state = true
compose_dir = "{}"
compose_svc = "shop"
env_name = "SHOP_PORT"
branch = "main"
health_url = "http://127.0.0.1/up"

[backup]
command = ["sqlite3", "{}", ".backup {{file}}"]
restore_command = ["sqlite3", "{}", ".restore {{file}}"]
verify_command = ["sqlite3", "{{file}}", "PRAGMA integrity_check;"]
offsite_command = ["rsync", "-a", "{{file}}", "{}"]
dir = "{}"
keep = 2
"#,
        db.parent().unwrap().display(),
        db.display(),
        db.display(),
        offsite_arg,
        backup_dir.display(),
    );
    toml::from_str(&text).expect("test app TOML must parse")
}

fn run(path: &Path) -> Run {
    let log = TraceLog::open(path.join("swapdock.jsonl")).unwrap();
    Run::start(
        log,
        RunMode::Live,
        Some("shop".into()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap()
}

#[test]
fn a_sqlite_backup_can_be_verified_copied_offsite_and_restored() {
    if !tool("sqlite3") || !tool("rsync") {
        eprintln!("SKIP backup_restore: sqlite3 or rsync is unavailable");
        return;
    }
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("shop.db");
    let backup_dir = dir.path().join("backups");
    let offsite = dir.path().join("offsite");
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&offsite).unwrap();
    sqlite(
        &db,
        "CREATE TABLE items (name TEXT); INSERT INTO items VALUES ('one');",
    );

    let app = app(&db, &backup_dir, &offsite);
    let mut r = run(dir.path());
    let snapshot = run_backup(&mut r, &app, Some("abc1234"))
        .expect("backup")
        .expect("the command uses {file}");
    r.finish(RunStatus::Ok).unwrap();
    assert!(snapshot.is_file(), "snapshot {}", snapshot.display());
    assert!(
        offsite
            .read_dir()
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().is_file()),
        "offsite copy"
    );
    assert_eq!(sqlite(&snapshot, "PRAGMA integrity_check;"), "ok");

    // Destroy the live data, then restore it from the verified snapshot.
    sqlite(&db, "DELETE FROM items;");
    assert_eq!(sqlite(&db, "SELECT COUNT(*) FROM items;"), "0");
    let mut r = run(dir.path());
    run_restore(&mut r, &app, &snapshot).expect("restore");
    r.finish(RunStatus::Ok).unwrap();
    assert_eq!(sqlite(&db, "SELECT name FROM items;"), "one");
}

#[test]
fn a_backup_keep_prunes_old_snapshots() {
    if !tool("sqlite3") || !tool("rsync") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("shop.db");
    let backup_dir = dir.path().join("backups");
    let offsite = dir.path().join("offsite");
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&offsite).unwrap();
    sqlite(&db, "CREATE TABLE items (name TEXT);");
    let app = app(&db, &backup_dir, &offsite);
    for _ in 0..3 {
        let mut r = run(dir.path());
        run_backup(&mut r, &app, None).expect("backup");
        r.finish(RunStatus::Ok).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let snapshots: Vec<PathBuf> = std::fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(snapshots.len(), 2, "{snapshots:?}");
}
