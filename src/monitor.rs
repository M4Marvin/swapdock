//! Read-only estate checks for the optional monitor timer.
//!
//! The timer checks three things that a deploy console otherwise learns too
//! late: an app's live port no longer answers, a TLS certificate is close to
//! expiry, or the root filesystem is close to full. Findings are sent through
//! the same webhook as deploy failures, with a short de-duplication window so a
//! five-minute timer does not spam the operator.
//!
//! This is not a full monitoring system: there is no history, no paging, no
//! email, and no per-app thresholds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::exec::{self, StepSpec};
use crate::registry::App;
use crate::time::Timestamp;

const DISK_TIMEOUT: Duration = Duration::from_secs(15);
const CERT_TIMEOUT: Duration = Duration::from_secs(15);
const CERT_WARN_SECONDS: u64 = 14 * 24 * 60 * 60;
const DEFAULT_REPEAT_SECS: u64 = 6 * 60 * 60;
const DEFAULT_DISK_PERCENT: u8 = 90;

/// One condition worth telling the operator about.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub key: String,
    pub kind: &'static str,
    pub message: String,
    pub detail: Value,
}

impl Finding {
    fn app_down(app: &App, detail: Value) -> Self {
        Self {
            key: format!("app:{}", app.name),
            kind: "app_down",
            message: format!("app {} is not answering its live port", app.name),
            detail,
        }
    }
}

/// What one monitor pass found and sent.
#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub sent: usize,
    pub suppressed: usize,
    pub errors: Vec<String>,
}

/// Checks every deployed app's live port.
pub fn app_findings(apps: &[App]) -> (Vec<Finding>, Vec<String>) {
    let mut findings = Vec::new();
    let mut errors = Vec::new();
    for app in apps {
        if app.live_port.is_none() {
            continue;
        }
        match crate::deploy::live_status(app) {
            Ok(status) if status.up => {}
            Ok(status) => findings.push(Finding::app_down(
                app,
                json!({
                    "app": app.name,
                    "live_port": app.live_port,
                    "release": status.release,
                    "http_status": status.code,
                }),
            )),
            Err(e) => errors.push(format!("{}: {e}", app.name)),
        }
    }
    (findings, errors)
}

/// Parses `df -P /` and returns the used percentage.
pub fn parse_df_percent(output: &str) -> Option<u8> {
    let line = output
        .lines()
        .rev()
        .find(|line| line.split_whitespace().count() >= 6)?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    let capacity = fields.get(4)?;
    capacity.trim_end_matches('%').parse::<u8>().ok()
}

/// Checks the root filesystem's used percentage.
pub fn disk_finding(threshold: u8) -> Result<Option<Finding>, String> {
    let spec = StepSpec::new("monitor-disk", "df")
        .args(["-P", "/"])
        .timeout(DISK_TIMEOUT);
    let outcome = exec::run_capture(&spec).map_err(|e| e.to_string())?;
    if !outcome.success() {
        return Err(format!("df failed: {}", outcome.stderr_trimmed()));
    }
    let percent = parse_df_percent(outcome.stdout_trimmed())
        .ok_or_else(|| "could not parse df output".to_string())?;
    if percent < threshold {
        return Ok(None);
    }
    Ok(Some(Finding {
        key: "disk:/".into(),
        kind: "disk_high",
        message: format!("root filesystem is {percent}% full"),
        detail: json!({"mount": "/", "used_percent": percent, "threshold_percent": threshold}),
    }))
}

/// Checks one PEM certificate with `openssl -checkend`.
pub fn cert_finding(path: &Path) -> Result<Option<Finding>, String> {
    if !path.is_file() {
        return Ok(Some(Finding {
            key: format!("cert:{}", path.display()),
            kind: "cert_expiring",
            message: format!("certificate {} is missing or unreadable", path.display()),
            detail: json!({"path": path.display().to_string(), "error": "certificate is missing or unreadable"}),
        }));
    }
    let spec = StepSpec::new("monitor-cert", "openssl")
        .args([
            "x509",
            "-checkend",
            &CERT_WARN_SECONDS.to_string(),
            "-noout",
            "-in",
        ])
        .arg(path.display().to_string())
        .timeout(CERT_TIMEOUT);
    let outcome = exec::run_capture(&spec).map_err(|e| e.to_string())?;
    if outcome.success() {
        return Ok(None);
    }
    let detail = if path.is_file() {
        json!({"path": path.display().to_string(), "warn_days": 14, "detail": outcome.stderr_trimmed()})
    } else {
        json!({"path": path.display().to_string(), "error": "certificate is missing or unreadable"})
    };
    Ok(Some(Finding {
        key: format!("cert:{}", path.display()),
        kind: "cert_expiring",
        message: format!(
            "certificate {} expires within 14 days or is unreadable",
            path.display()
        ),
        detail,
    }))
}

/// The de-duplication state persisted between timer runs.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct State {
    /// key -> epoch milliseconds of the last send.
    #[serde(default)]
    sent: BTreeMap<String, i64>,
}

fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("monitor-state.json")
}

fn load_state(path: &Path) -> State {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state)?;
    std::fs::write(path, bytes)
}

/// Runs the app, disk and certificate checks, sends de-duplicated alerts, and
/// returns the report. A missing webhook simply skips sending.
pub fn run(
    apps: &[App],
    state_dir: &Path,
    disk_threshold: u8,
    certs: &[PathBuf],
    repeat_secs: u64,
    send: bool,
) -> Report {
    let mut report = Report::default();
    let (app_findings, errors) = app_findings(apps);
    report.findings.extend(app_findings);
    report.errors.extend(errors);
    match disk_finding(disk_threshold) {
        Ok(Some(finding)) => report.findings.push(finding),
        Ok(None) => {}
        Err(e) => report.errors.push(e),
    }
    for cert in certs {
        match cert_finding(cert) {
            Ok(Some(finding)) => report.findings.push(finding),
            Ok(None) => {}
            Err(e) => report.errors.push(format!("{}: {e}", cert.display())),
        }
    }

    if !send || crate::notify::configured_url().is_none() {
        return report;
    }
    let path = state_path(state_dir);
    let mut state = load_state(&path);
    let before = state.sent.len();
    let now = Timestamp::now().epoch_millis();
    let repeat_ms = (repeat_secs.saturating_mul(1000)) as i64;
    let mut dirty = false;
    for finding in &report.findings {
        let due = state
            .sent
            .get(&finding.key)
            .is_none_or(|last| now.saturating_sub(*last) >= repeat_ms);
        if !due {
            report.suppressed += 1;
            continue;
        }
        match crate::notify::send_alert(finding.kind, &finding.message, &finding.detail) {
            Ok(()) => {
                report.sent += 1;
                state.sent.insert(finding.key.clone(), now);
                dirty = true;
            }
            Err(e) => report.errors.push(format!("webhook: {e}")),
        }
    }
    // Forget keys that are healthy again, so a later failure alerts promptly.
    let live_keys: std::collections::BTreeSet<&str> =
        report.findings.iter().map(|f| f.key.as_str()).collect();
    state.sent.retain(|key, _| live_keys.contains(key.as_str()));
    if (dirty || before != state.sent.len() || path.exists())
        && let Err(e) = save_state(&path, &state)
    {
        report.errors.push(format!("monitor state: {e}"));
    }
    report
}

/// Reads the disk threshold from the environment, clamping it to 1..=100.
pub fn disk_threshold_from_env() -> u8 {
    std::env::var("SWAPDOCK_DISK_THRESHOLD")
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|value| (1..=100).contains(value))
        .unwrap_or(DEFAULT_DISK_PERCENT)
}

/// Reads `SWAPDOCK_TLS_CERTS` (comma-separated paths).
pub fn certs_from_env() -> Vec<PathBuf> {
    std::env::var("SWAPDOCK_TLS_CERTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Reads `SWAPDOCK_ALERT_REPEAT_SECS`; default six hours.
pub fn repeat_secs_from_env() -> u64 {
    std::env::var("SWAPDOCK_ALERT_REPEAT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 60)
        .unwrap_or(DEFAULT_REPEAT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_percent_is_parsed_from_the_last_row() {
        let output =
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100 91 9 91% /\n";
        assert_eq!(parse_df_percent(output), Some(91));
        assert_eq!(parse_df_percent("garbage"), None);
    }

    #[test]
    fn app_without_a_live_port_is_not_a_finding() {
        let mut app = crate::registry::tests::sample();
        app.live_port = None;
        assert!(app_findings(&[app]).0.is_empty());
    }

    #[test]
    fn a_closed_live_port_is_an_app_down_finding() {
        let mut app = crate::registry::tests::sample();
        app.live_port = Some(1);
        app.health_url = Some("http://127.0.0.1/".into());
        let (findings, errors) = app_findings(&[app]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, "app_down");
    }

    #[test]
    fn missing_certificate_is_a_finding_without_spawning_openssl() {
        let finding = cert_finding(Path::new("/definitely/missing/cert.pem")).unwrap();
        assert!(finding.is_some(), "missing cert should be reported");
        assert_eq!(finding.unwrap().kind, "cert_expiring");
    }

    #[test]
    fn dry_run_does_not_write_monitor_state() {
        let dir = tempfile::TempDir::new().unwrap();
        let _ = run(&[], dir.path(), 99, &[], 60, false);
        assert!(!dir.path().join("monitor-state.json").exists());
    }

    #[test]
    fn env_defaults_are_safe() {
        assert!((1..=100).contains(&disk_threshold_from_env()));
        assert!(repeat_secs_from_env() >= 60);
    }
}
