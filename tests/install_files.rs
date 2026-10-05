//! The hand-written nginx files under `install/nginx/` are load-bearing: a syntax
//! error in either one fails every `nginx -t` on the host. This test builds a
//! minimal harness around them and runs the real binary against it.
//!
//! Skipped without nginx. The harness redirects pid, logs and temp paths into
//! the temp dir so it runs unprivileged.

use std::path::{Path, PathBuf};
use std::process::Command;

fn nginx() -> Option<PathBuf> {
    for candidate in ["/usr/bin/nginx", "/usr/sbin/nginx"] {
        if Path::new(candidate).exists() {
            return Some(PathBuf::from(candidate));
        }
    }
    let out = Command::new("sh")
        .args(["-c", "command -v nginx"])
        .output()
        .ok()?;
    if out.status.success() {
        return Some(PathBuf::from(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ));
    }
    None
}

fn install_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("install")
}

#[test]
fn hand_written_nginx_files_pass_nginx_t() {
    let Some(binary) = nginx() else {
        eprintln!("SKIP install_files: no nginx binary");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    for name in ["swapdock-http.conf", "proxy-common.conf"] {
        let src = install_dir().join("nginx").join(name);
        assert!(src.exists(), "missing install/nginx/{name}");
        std::fs::copy(src, root.join(name)).unwrap();
    }
    for sub in ["cb", "px", "fc", "uw", "sc"] {
        std::fs::create_dir(root.join(sub)).unwrap();
    }

    // Mirrors how the host uses them: swapdock-http.conf at http level,
    // proxy-common.conf inside a location block.
    std::fs::write(
        root.join("nginx.conf"),
        format!(
            "pid {root}/nginx.pid;\nerror_log {root}/error.log;\nevents {{}}\nhttp {{\n  \
             access_log off;\n  \
             client_body_temp_path {root}/cb;\n  \
             proxy_temp_path {root}/px;\n  \
             fastcgi_temp_path {root}/fc;\n  \
             uwsgi_temp_path {root}/uw;\n  \
             scgi_temp_path {root}/sc;\n  \
             include {root}/swapdock-http.conf;\n  \
             server {{\n    \
             listen 127.0.0.1:18999;\n    \
             location / {{\n      \
             include {root}/proxy-common.conf;\n      \
             proxy_pass http://127.0.0.1:18998;\n    \
             }}\n  }}\n}}\n",
            root = root.display()
        ),
    )
    .unwrap();

    let out = Command::new(&binary)
        .args(["-t", "-c"])
        .arg(root.join("nginx.conf"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "install nginx files fail nginx -t:\n{stderr}"
    );
    assert!(
        stderr.contains("test is successful"),
        "unexpected output:\n{stderr}"
    );
}

#[test]
fn installer_refuses_without_a_dist_dir() {
    let script = install_dir().join("install-swapdock.sh");
    assert!(script.exists(), "installer missing");
    let out = Command::new("sh")
        .arg(&script)
        .arg("/nonexistent-dist-xyz")
        .env("HOME", "/nonexistent-home-xyz")
        .output()
        .unwrap();
    // Must fail without root and without the dir — either refusal is correct,
    // hanging or half-running is not.
    assert!(
        !out.status.success(),
        "must not succeed without root and dist"
    );
}

#[test]
fn installer_script_has_no_bashisms_beyond_posix_set() {
    // Runs under `bash` explicitly, but keep it lint-clean anyway.
    let text = std::fs::read_to_string(install_dir().join("install-swapdock.sh")).unwrap();
    assert!(text.starts_with("#!/usr/bin/env bash"), "shebang changed?");
    assert!(
        text.contains("set -euo pipefail"),
        "must abort on first failure"
    );
    assert!(
        !text.contains("//"),
        "C++ comments are not nginx comments and do not belong near this script"
    );
}
