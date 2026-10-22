//! Best-effort deploy-failure webhook.
//!
//! This is deliberately small: one operator-controlled HTTPS endpoint, one
//! JSON POST, no retry queue and no secrets in argv. The URL lives in
//! `SWAPDOCK_WEBHOOK_URL`; an absent or empty value makes notifications a
//! no-op. The call happens after the run has been closed, so a slow webhook can
//! never turn a successful deploy into a failed one.
//!
//! This is not a pager: it covers API-triggered job failures. App-down probes,
//! certificate expiry and disk thresholds still need a monitor timer (G4.9).

use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::exec::{self, StepSpec};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY: usize = 16 * 1024;

/// Why a webhook could not be sent.
#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("SWAPDOCK_WEBHOOK_URL must be an http(s) URL without whitespace or control characters")]
    InvalidUrl,

    #[error("webhook POST failed: {0}")]
    Failed(String),

    #[error("could not run the webhook command: {0}")]
    Exec(#[from] exec::ExecError),
}

/// Returns the configured webhook, or `None` when notifications are disabled.
pub fn configured_url() -> Option<String> {
    let raw = std::env::var("SWAPDOCK_WEBHOOK_URL").ok()?;
    let url = raw.trim();
    if url.is_empty() {
        return None;
    }
    valid_url(url).then(|| url.to_string())
}

fn valid_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// The JSON body sent for one failed job.
pub fn failure_payload(app: Option<&str>, run_id: &str, error: &str) -> serde_json::Value {
    let mut error = crate::redact::Redactor::new().text(error);
    if error.chars().count() > MAX_BODY {
        error = error.chars().take(MAX_BODY).collect();
        error.push('…');
    }
    json!({
        "event": "job_failed",
        "service": "swapdock",
        "app": app,
        "run_id": run_id,
        "error": error,
        "sent_at": crate::time::Timestamp::now().to_rfc3339(),
    })
}

/// Sends one failure payload. Missing configuration is a successful no-op.
pub fn send_failure(app: Option<&str>, run_id: &str, error: &str) -> Result<(), NotifyError> {
    send_payload(failure_payload(app, run_id, error))
}

/// Sends one monitor alert (app down, certificate expiry, disk pressure).
pub fn send_alert(
    kind: &str,
    message: &str,
    detail: &serde_json::Value,
) -> Result<(), NotifyError> {
    let message = crate::redact::Redactor::new().text(message);
    send_payload(json!({
        "event": "alert",
        "kind": kind,
        "message": message,
        "detail": detail,
        "sent_at": crate::time::Timestamp::now().to_rfc3339(),
    }))
}

fn send_payload(payload: serde_json::Value) -> Result<(), NotifyError> {
    let Some(url) = configured_url() else {
        return Ok(());
    };
    let body = serde_json::to_string(&payload).map_err(|e| NotifyError::Failed(e.to_string()))?;
    let spec = StepSpec::new("notify-webhook", "curl")
        .args([
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
        ])
        .arg(body)
        .arg(url)
        .timeout(NOTIFY_TIMEOUT);
    let outcome = exec::run_capture(&spec)?;
    if outcome.success() {
        Ok(())
    } else {
        Err(NotifyError::Failed(outcome.stderr_trimmed().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_the_stable_job_failure_shape() {
        let payload = failure_payload(Some("chat"), "run-1", "deploy failed");
        assert_eq!(payload["event"], "job_failed");
        assert_eq!(payload["service"], "swapdock");
        assert_eq!(payload["app"], "chat");
        assert_eq!(payload["run_id"], "run-1");
        assert!(payload["sent_at"].is_string());
    }

    #[test]
    fn only_http_urls_are_accepted() {
        assert!(valid_url("https://hooks.example.com/swapdock"));
        assert!(valid_url("http://127.0.0.1:9000/hook"));
        assert!(!valid_url("file:///etc/passwd"));
        assert!(!valid_url("https://example.com/hook with space"));
        assert!(!valid_url("https://example.com/\n"));
        assert!(!valid_url("--data-binary"));
    }
}
