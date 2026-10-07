//! The registry: one file per app, the single source of truth.
//!
//! Layout on the host:
//!
//! ```text
//! /srv/swapdock/apps/portfolio.toml
//! /srv/swapdock/apps/docs.toml
//! ```
//!
//! One file per app rather than one file for all apps, so two concurrent deploys
//! never race on the same write.
//!
//! ## Unknown fields are an error
//!
//! `deny_unknown_fields` is deliberate. A typo like `front_prot = 8001` would
//! otherwise be silently ignored and the app would fall back to a default port —
//! on a tool that rewrites the nginx config for every service, that is not a
//! warning, it is an outage with a confusing cause. The cost is that adding a
//! field to this struct breaks older files, which is the correct trade for this
//! tool.

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ports::{self, MAX_SLOTS};

/// What kind of thing is behind the front port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A container publishing a loopback port, reached through `proxy_pass`.
    Container,
    /// Files on disk behind a symlink, reached through `root`.
    Static,
}

/// How a release replaces the running one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// Start the new version alongside the old, then flip. No gap.
    Swap,
    /// Stop the old, then start the new. A short gap, but no two writers.
    Replace,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Strategy::Swap => "swap",
            Strategy::Replace => "replace",
        }
    }
}

/// Where images live.
/// `Ghcr` is the default and is declared first; the derive relies on that, and
/// `the_default_registry_is_ghcr` pins it so reordering the variants fails a test
/// rather than silently changing where images are pushed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageRegistry {
    #[default]
    Ghcr,
    DockerHub,
    /// A `registry:2` container on the swapdock host.
    Local,
}

/// How serious a finding is. Any `Error` makes the command exit non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// One thing wrong with the configuration.
///
/// `code` is a stable kebab-case slug so findings can be grepped, matched in
/// tests, and referred to in documentation without depending on wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    /// The app this concerns, or `None` for a whole-estate finding.
    pub app: Option<String>,
}

impl Problem {
    pub fn error(code: &'static str, app: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            app: app.map(str::to_string),
        }
    }

    pub fn warning(code: &'static str, app: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            app: app.map(str::to_string),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.app {
            Some(app) => write!(f, "{}: [{}] {}", app, self.code, self.message),
            None => write!(f, "[{}] {}", self.code, self.message),
        }
    }
}

/// One app's registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// Short slug, equal to the file stem.
    pub name: String,
    pub kind: Kind,
    pub strategy: Strategy,

    /// Hostnames routed to this app. May be empty while an app is being set up.
    pub hostnames: Vec<String>,

    /// Addresses nginx binds for the front port. Empty means `127.0.0.1`.
    ///
    /// A second address is for services also reachable over the tailnet, such as
    /// `chat` on `192.0.2.10:8002`. Kept as `String` so a typo becomes a
    /// validation message instead of a deserialization failure.
    #[serde(default)]
    pub listen: Vec<String>,

    /// The port cloudflared points at. nginx owns it permanently.
    pub front_port: u16,
    /// Slot that owns this app's back-end port pair.
    pub slot: u8,

    #[serde(default)]
    pub live_port: Option<u16>,
    #[serde(default)]
    pub old_port: Option<u16>,

    /// True when the app writes to a database file or other shared local state.
    ///
    /// This field exists to make one hard-won rule machine-checkable: such an app
    /// must not use `swap`, because two containers would hold the same file open.
    #[serde(default)]
    pub writes_state: bool,

    // ---- release identity ----
    #[serde(default)]
    pub image_repo: Option<String>,
    #[serde(default)]
    pub registry: Option<ImageRegistry>,
    /// Commit name of the running release. Never `latest`.
    #[serde(default)]
    pub release: Option<String>,
    /// Commit name to roll back to.
    #[serde(default)]
    pub old_release: Option<String>,
    /// `local`, or a host to build on. A config value, never a code path.
    #[serde(default)]
    pub build_host: Option<String>,

    // ---- static only ----
    #[serde(default)]
    pub root: Option<String>,

    // ---- runtime ----
    #[serde(default)]
    pub health_url: Option<String>,
    pub compose_dir: PathBuf,
    pub compose_svc: String,
    /// Environment variable that carries the back-end port, e.g. `PORTFOLIO_PORT`.
    pub env_name: String,
    #[serde(default)]
    pub git_remote: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    /// Local checkout of the app source, when the tool manages it.
    ///
    /// Used by `swapdock sync` (fetch and fast-forward). Absent for apps whose
    /// source lives elsewhere or is vendored.
    #[serde(default)]
    pub repo: Option<PathBuf>,
}

impl App {
    /// The addresses nginx should bind. Empty list means loopback only.
    pub fn listen_addrs(&self) -> Vec<String> {
        if self.listen.is_empty() {
            vec!["127.0.0.1".to_string()]
        } else {
            self.listen.clone()
        }
    }

    /// Hostnames lowercased with any trailing dot removed.
    ///
    /// DNS is case-insensitive and `example.com.` is the same name as
    /// `example.com`, so two spellings of one hostname would otherwise slip
    /// past a duplicate check.
    pub fn normalized_hostnames(&self) -> Vec<String> {
        self.hostnames
            .iter()
            .map(|h| normalize_hostname(h))
            .collect()
    }

    pub fn image_registry(&self) -> ImageRegistry {
        self.registry.unwrap_or_default()
    }

    pub fn git_branch(&self) -> &str {
        self.branch.as_deref().unwrap_or("main")
    }

    /// Project name for `docker compose -p`. Explicitly set in a future field;
    /// for now, the compose directory name, which matches `apps` and `jobs`.
    pub fn compose_project(&self) -> String {
        self.compose_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "apps".to_string())
    }

    /// Container name for the green candidate. The live container keeps its
    /// compose-assigned name; the green one must differ globally, because
    /// `container_name` is not namespaced by project.
    pub fn green_container_name(&self) -> String {
        format!("{}-green", self.compose_svc)
    }

    /// Advances the recorded release after a successful swapdock.
    ///
    /// The previous release becomes the rollback target, so `rollback` is always
    /// one step back with no extra bookkeeping.
    pub fn advance(&mut self, release: String, new_live_port: u16) {
        self.old_release = self.release.take();
        self.release = Some(release);
        self.old_port = self.live_port;
        self.live_port = Some(new_live_port);
    }

    /// The image reference for a release: `repo:release`.
    ///
    /// Repos that already contain a registry path (`ghcr.io/example-org/git`)
    /// are used as-is; bare names (`apps-portfolio`) are local tags.
    pub fn image_ref(&self, release: &str) -> Option<String> {
        self.image_repo
            .as_ref()
            .map(|repo| format!("{repo}:{release}"))
    }

    /// True when the image must be pulled rather than assumed present.
    pub fn needs_pull(&self) -> bool {
        !matches!(self.image_registry(), ImageRegistry::Local)
    }

    /// Every check that can be made by looking at this app alone.
    ///
    /// Cross-app checks live in `crate::validator`; keeping the split means this
    /// function stays a pure per-item test.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        let name = self.name.as_str();

        // --- name ---
        if self.name.is_empty() {
            out.push(Problem::error("name-empty", None, "name must not be empty"));
        } else if !is_valid_slug(&self.name) {
            out.push(Problem::error(
                "name-invalid",
                Some(name),
                format!(
                    "name {:?} must be lowercase letters, digits and dashes, \
                     starting with a letter and not ending with a dash",
                    self.name
                ),
            ));
        }

        // --- hostnames ---
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for host in &self.hostnames {
            if !is_valid_hostname(host) {
                out.push(Problem::error(
                    "hostname-invalid",
                    Some(name),
                    format!(
                        "hostname {host:?} is not a bare host name: no scheme, port, path or wildcard"
                    ),
                ));
                continue;
            }
            let key = normalize_hostname(host);
            if seen.insert(key.clone(), ()).is_some() {
                out.push(Problem::error(
                    "hostname-duplicate-in-app",
                    Some(name),
                    format!("hostname {key:?} is listed more than once"),
                ));
            }
        }
        if self.hostnames.is_empty() {
            out.push(Problem::warning(
                "hostnames-empty",
                Some(name),
                "no hostname routes to this front port yet",
            ));
        }

        // --- listen addresses ---
        for addr in self.listen_addrs() {
            if addr.parse::<IpAddr>().is_err() {
                out.push(Problem::error(
                    "listen-invalid",
                    Some(name),
                    format!("listen address {addr:?} is not an IP address"),
                ));
            }
        }

        // --- ports ---
        if self.front_port == 0 {
            out.push(Problem::error(
                "front-port-zero",
                Some(name),
                "front_port 0 is not a usable port",
            ));
        }
        if ports::is_reserved(self.front_port) {
            out.push(Problem::error(
                "front-port-reserved",
                Some(name),
                format!(
                    "front_port {} is inside the reserved back-end range {}..={}",
                    self.front_port,
                    ports::RESERVED_BACK.start(),
                    ports::RESERVED_BACK.end()
                ),
            ));
        }
        if self.slot >= MAX_SLOTS {
            out.push(Problem::error(
                "slot-out-of-range",
                Some(name),
                format!("slot {} is out of range 0..={}", self.slot, MAX_SLOTS - 1),
            ));
        } else {
            let slot = self.slot;
            let (a, b) = ports::pair(slot).expect("slot checked above");
            for (label, port) in [("live_port", self.live_port), ("old_port", self.old_port)] {
                if let Some(p) = port
                    && p != a
                    && p != b
                {
                    out.push(Problem::error(
                        "port-not-in-slot",
                        Some(name),
                        format!("{label} {p} is not in the slot {slot} pair ({a} or {b})"),
                    ));
                }
            }
            if let (Some(live), Some(old)) = (self.live_port, self.old_port)
                && live == old
            {
                out.push(Problem::error(
                    "live-equals-old",
                    Some(name),
                    format!("live_port and old_port are both {live}"),
                ));
            }
        }

        // --- the two-writers rule ---
        if self.writes_state && self.strategy == Strategy::Swap {
            out.push(Problem::error(
                "swap-with-shared-state",
                Some(name),
                "writes_state is set, so strategy must be 'replace': a swap runs two \
                 containers against the same database file",
            ));
        }

        // --- kind-conditional fields ---
        match self.kind {
            Kind::Static => {
                match &self.root {
                    None => out.push(Problem::error(
                        "root-missing",
                        Some(name),
                        "a static app needs root, the directory nginx serves",
                    )),
                    Some(r) if !r.starts_with('/') => out.push(Problem::error(
                        "root-not-absolute",
                        Some(name),
                        format!("root {r:?} must be an absolute path"),
                    )),
                    // A path is interpolated into an nginx directive. Whitespace,
                    // a semicolon or a quote would end or alter the directive, so
                    // such a path is rejected rather than escaped.
                    Some(r) if !is_safe_nginx_value(r) => out.push(Problem::error(
                        "root-unsafe",
                        Some(name),
                        format!(
                            "root {r:?} contains whitespace, a quote, a semicolon, a brace \
                             or a backslash, any of which would break the nginx directive"
                        ),
                    )),
                    Some(_) => {}
                }
                for (label, present) in [
                    ("live_port", self.live_port.is_some()),
                    ("image_repo", self.image_repo.is_some()),
                    ("release", self.release.is_some()),
                ] {
                    if present {
                        out.push(Problem::warning(
                            "field-unused-for-static",
                            Some(name),
                            format!("{label} has no meaning for a static app"),
                        ));
                    }
                }
                if self.strategy != Strategy::Swap {
                    out.push(Problem::error(
                        "static-strategy",
                        Some(name),
                        "a static app must use 'swap': the symlink rename is atomic, \
                         so there is no reason to accept a gap",
                    ));
                }
            }
            Kind::Container => {
                if self.root.is_some() {
                    out.push(Problem::warning(
                        "root-unused-for-container",
                        Some(name),
                        "root has no meaning for a container app",
                    ));
                }
                match &self.image_repo {
                    None => out.push(Problem::error(
                        "image-repo-missing",
                        Some(name),
                        "a container app needs image_repo",
                    )),
                    Some(r) if r.trim().is_empty() => out.push(Problem::error(
                        "image-repo-empty",
                        Some(name),
                        "image_repo must not be empty",
                    )),
                    Some(_) => {}
                }
                match &self.health_url {
                    None => out.push(Problem::error(
                        "health-url-missing",
                        Some(name),
                        "a container app needs health_url: it is the gate a swapdock waits on",
                    )),
                    Some(u) if !(u.starts_with("http://") || u.starts_with("https://")) => {
                        out.push(Problem::error(
                            "health-url-invalid",
                            Some(name),
                            format!("health_url {u:?} must start with http:// or https://"),
                        ));
                    }
                    Some(_) => {}
                }
            }
        }

        // --- release identity ---
        for (label, value) in [
            ("release", &self.release),
            ("old_release", &self.old_release),
        ] {
            if let Some(v) = value
                && !is_valid_release(v)
            {
                out.push(Problem::error(
                    "release-not-a-commit-name",
                    Some(name),
                    format!("{label} {v:?} must be a hex commit name of 7 to 40 characters"),
                ));
            }
        }
        if self.release.as_deref() == Some("latest") {
            out.push(Problem::error(
                "release-is-latest",
                Some(name),
                "release must not be 'latest': a moving tag gives a rollback no target",
            ));
        }
        if let Some(host) = &self.build_host
            && host.trim().is_empty()
        {
            out.push(Problem::error(
                "build-host-empty",
                Some(name),
                "build_host must name a host or be 'local'",
            ));
        }

        // --- compose wiring ---
        if !self.compose_dir.is_absolute() {
            out.push(Problem::error(
                "compose-dir-not-absolute",
                Some(name),
                format!(
                    "compose_dir {} must be absolute so it does not depend on the working directory",
                    self.compose_dir.display()
                ),
            ));
        }
        if !is_valid_compose_service(&self.compose_svc) {
            out.push(Problem::error(
                "compose-svc-invalid",
                Some(name),
                format!(
                    "compose_svc {:?} must start with a letter or digit and contain only \
                     letters, digits, dashes and underscores",
                    self.compose_svc
                ),
            ));
        }
        if !is_valid_env_name(&self.env_name) {
            out.push(Problem::error(
                "env-name-invalid",
                Some(name),
                format!(
                    "env_name {:?} must be upper case letters, digits and underscores, \
                     and must not start with a digit",
                    self.env_name
                ),
            ));
        }

        out
    }
}

/// Result of reading a directory of registry files.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Every app that parsed.
    pub apps: Vec<App>,
    /// Problems found while loading, including unreadable files.
    pub problems: Vec<Problem>,
}

impl Loaded {
    /// Apps sorted by name, which is the order the renderer uses.
    pub fn sorted(&self) -> Vec<App> {
        let mut apps = self.apps.clone();
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        apps
    }

    pub fn errors(&self) -> usize {
        self.problems.iter().filter(|p| p.is_error()).count()
    }
}

/// Reads every `*.toml` in `dir`.
///
/// A file that fails to parse becomes a problem rather than an error return, so
/// one bad file does not hide the state of the other thirteen.
pub fn load_dir(dir: &Path) -> std::io::Result<Loaded> {
    let mut loaded = Loaded::default();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(loaded),
        Err(e) => return Err(e),
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();

    for path in paths {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) => {
                loaded.problems.push(Problem::error(
                    "file-unreadable",
                    Some(&stem),
                    format!("{}: {e}", path.display()),
                ));
                continue;
            }
        };

        match toml::from_str::<App>(&raw) {
            Ok(app) => {
                // The file name is part of the identity: `portfolio.toml` must be
                // the app named `portfolio`, or `swapdock show portfolio` and a
                // human reading the directory disagree.
                if app.name != stem {
                    loaded.problems.push(Problem::error(
                        "filename-name-mismatch",
                        Some(&stem),
                        format!(
                            "{} declares name {:?}; the file stem must match",
                            path.display(),
                            app.name
                        ),
                    ));
                }
                loaded.apps.push(app);
            }
            Err(e) => {
                loaded.problems.push(Problem::error(
                    "file-unparseable",
                    Some(&stem),
                    format!("{}: {e}", path.display()),
                ));
            }
        }
    }

    Ok(loaded)
}

/// Serializes one app back to TOML, for `register` and later edits.
pub fn to_toml(app: &App) -> String {
    toml::to_string_pretty(app).expect("App is always serializable")
}

/// Writes one app's file atomically: temp file, fsync, rename.
///
/// Reads back what it wrote, so a torn write fails loudly instead of leaving a
/// half-file registry behind.
pub fn save_app(dir: &Path, app: &App) -> std::io::Result<()> {
    let target = dir.join(format!("{}.toml", app.name));
    let staging = dir.join(format!(".{}.toml.staging", app.name));

    std::fs::write(&staging, to_toml(app))?;
    {
        let file = std::fs::File::open(&staging)?;
        file.sync_all()?;
    }
    // swapdock usually runs as root (the sudoers entry is what allows
    // `apply` to reload nginx). A staging-file rename would then reset the
    // registry file to root ownership and the daemon's umask — the operator
    // loses edit access. Copy the existing file's mode and uid/gid forward
    // so a rewrite changes content, never access.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(&target) {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            &staging,
            std::fs::Permissions::from_mode(meta.mode() & 0o777),
        );
        // chown needs privilege; as an unprivileged user the existing
        // owner already survives the rename, so ignore failures.
        let _ = std::os::unix::fs::chown(&staging, Some(meta.uid()), Some(meta.gid()));
    }
    std::fs::rename(&staging, &target)?;

    // Prove the write: parse the file back and compare.
    let raw = std::fs::read_to_string(&target)?;
    let back: App = toml::from_str(&raw).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("wrote {} but it does not parse back: {e}", target.display()),
        )
    })?;
    if back != *app {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("wrote {} but it reads back differently", target.display()),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// small helpers, kept here so validation and tests agree on one definition
// ---------------------------------------------------------------------------

/// Lowercases and drops a trailing dot, so two spellings of one name compare equal.
pub fn normalize_hostname(host: &str) -> String {
    let trimmed = host.trim();
    trimmed
        .strip_suffix('.')
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

/// True when a value can be interpolated into one nginx directive unchanged.
///
/// Deliberately strict. An allow-list is safer than escaping: a path we cannot
/// prove safe is refused, rather than escaped with rules that might miss a case.
pub fn is_safe_nginx_value(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ';' | '{'
                        | '}'
                        | '"'
                        | '\''
                        | '\\'
                        | '$'
                        | '#'
                        | '('
                        | ')'
                        | ','
                        | '<'
                        | '>'
                        | '?'
                )
        })
}

/// True for a bare host name: labels of letters, digits and dashes, at least two
/// labels, no scheme, port, path or wildcard.
pub fn is_valid_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || host.contains('*') {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// True for a lowercase slug usable as a file name and a CLI argument.
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug.starts_with(|c: char| c.is_ascii_lowercase())
        && slug.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// True for a compose service name.
pub fn is_valid_compose_service(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// True for an environment variable name.
pub fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// True for a hex commit name of 7 to 40 characters.
pub fn is_valid_release(release: &str) -> bool {
    (7..=40).contains(&release.len()) && release.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A valid container app to mutate in tests.
    pub(crate) fn sample() -> App {
        App {
            name: "portfolio".into(),
            kind: Kind::Container,
            strategy: Strategy::Swap,
            hostnames: vec!["example.com".into()],
            listen: vec![],
            front_port: 8001,
            slot: 0,
            live_port: Some(9000),
            old_port: Some(9001),
            writes_state: false,
            image_repo: Some("apps-portfolio".into()),
            registry: None,
            release: Some("9c1f2ab".into()),
            old_release: Some("4d8e0f1".into()),
            build_host: Some("local".into()),
            root: None,
            health_url: Some("http://127.0.0.1/".into()),
            compose_dir: PathBuf::from("/home/marv/apps"),
            compose_svc: "portfolio".into(),
            env_name: "PORTFOLIO_PORT".into(),
            git_remote: Some("ExampleOrg/main-site".into()),
            branch: Some("master".into()),
            repo: Some(PathBuf::from("/home/marv/apps/main-site")),
        }
    }

    /// A valid static app.
    pub(crate) fn sample_static() -> App {
        App {
            name: "docs".into(),
            kind: Kind::Static,
            strategy: Strategy::Swap,
            hostnames: vec!["docs.example.com".into(), "www.shop.example.com".into()],
            front_port: 8012,
            slot: 3,
            live_port: None,
            old_port: None,
            writes_state: false,
            image_repo: None,
            release: None,
            old_release: None,
            root: Some("/srv/www/docs/current".into()),
            health_url: None,
            compose_svc: "docs".into(),
            env_name: "DOCS_PORT".into(),
            ..sample()
        }
    }

    fn codes(app: &App) -> Vec<&'static str> {
        let mut c: Vec<&'static str> = app.problems().iter().map(|p| p.code).collect();
        c.sort_unstable();
        c
    }

    #[test]
    fn a_valid_container_app_has_no_problems() {
        assert_eq!(sample().problems(), vec![], "expected a clean entry");
    }

    #[test]
    fn a_valid_static_app_has_no_problems() {
        let app = sample_static();
        // `..sample()` carries container-only fields; clear the ones that warn.
        let app = App {
            image_repo: None,
            release: None,
            old_release: None,
            health_url: None,
            writes_state: false,
            ..app
        };
        assert_eq!(app.problems(), vec![], "expected a clean static entry");
    }

    // ---- round trip ----

    #[test]
    fn toml_round_trips_exactly() {
        let app = sample();
        let text = to_toml(&app);
        let back: App = toml::from_str(&text).expect("must parse");
        assert_eq!(app, back, "serialize then parse must be lossless");
    }

    #[test]
    fn toml_round_trips_a_static_app() {
        let app = sample_static();
        let back: App = toml::from_str(&toml::to_string_pretty(&app).unwrap()).unwrap();
        assert_eq!(app, back);
    }

    #[test]
    fn omitted_optional_fields_use_their_defaults() {
        let minimal = r#"
            name = "charts"
            kind = "container"
            strategy = "replace"
            hostnames = ["shop.example.com"]
            front_port = 8006
            slot = 1
            writes_state = false
            image_repo = "apps-charts"
            health_url = "http://127.0.0.1:8001/"
            compose_dir = "/home/marv/apps"
            compose_svc = "charts"
            env_name = "CHARTS_PORT"
        "#;
        let app: App = toml::from_str(minimal).expect("must parse");
        assert!(app.listen.is_empty());
        assert_eq!(app.listen_addrs(), vec!["127.0.0.1".to_string()]);
        assert_eq!(app.image_registry(), ImageRegistry::Ghcr);
        assert_eq!(app.git_branch(), "main");
        assert!(app.release.is_none());
        assert!(app.problems().is_empty(), "{:?}", app.problems());
    }

    // ---- deny_unknown_fields ----

    #[test]
    fn a_typo_in_a_field_name_is_rejected() {
        let typo = r#"
            name = "portfolio"
            kind = "container"
            strategy = "swap"
            hostnames = ["example.com"]
            front_prot = 8001
            slot = 0
            image_repo = "apps-portfolio"
            health_url = "http://127.0.0.1/"
            compose_dir = "/home/marv/apps"
            compose_svc = "portfolio"
            env_name = "PORTFOLIO_PORT"
        "#;
        let err = toml::from_str::<App>(typo).expect_err("a typo must not be ignored");
        assert!(
            err.to_string().contains("front_prot"),
            "the error must name the offending key: {err}"
        );
    }

    #[test]
    fn an_unknown_kind_variant_is_rejected() {
        let bad = r#"
            name = "x"
            kind = "vm"
            strategy = "swap"
            hostnames = []
            front_port = 8001
            slot = 0
            compose_dir = "/a"
            compose_svc = "x"
            env_name = "X_PORT"
        "#;
        assert!(toml::from_str::<App>(bad).is_err());
    }

    // ---- hostname handling ----

    #[test]
    fn hostnames_that_differ_only_in_case_collide() {
        // DNS is case-insensitive, so two spellings of one name must not both
        // be accepted as separate entries.
        let mut app = sample();
        app.hostnames = vec!["Example.com".into(), "example.COM".into()];
        assert_eq!(
            app.normalized_hostnames(),
            vec!["example.com", "example.com"]
        );
        let found = codes(&app);
        assert!(found.contains(&"hostname-duplicate-in-app"), "{found:?}");
        assert!(!found.contains(&"hostname-invalid"), "{found:?}");
    }

    #[test]
    fn a_trailing_dot_in_a_hostname_is_rejected() {
        // A trailing dot is a typo in a config file. It is normalised away for
        // comparison, but never accepted as input, and never emitted into nginx.
        let mut app = sample();
        app.hostnames = vec!["example.com.".into()];
        assert!(
            codes(&app).contains(&"hostname-invalid"),
            "{:?}",
            codes(&app)
        );
        assert_eq!(normalize_hostname("example.com."), "example.com");
        assert!(!is_valid_hostname("example.com."));
    }

    #[test]
    fn hostname_syntax_is_checked() {
        for bad in [
            "",
            "a",
            "localhost",
            "*.example.com",
            "https://example.com",
            "example.com:8001",
            "example.com/path",
            "-lead.example.com",
            "trail-.example.com",
            "double..dot.com",
            "sp ace.com",
        ] {
            assert!(!is_valid_hostname(bad), "{bad:?} must be rejected");
        }
        for good in [
            "example.com",
            "a.b",
            "shop.example.com",
            "xn--80ak6aa92e.com",
            "my-site.co.uk",
        ] {
            assert!(is_valid_hostname(good), "{good:?} must be accepted");
        }
    }

    #[test]
    fn a_hostname_longer_than_253_characters_is_rejected() {
        let long = format!("{}.com", "a".repeat(250));
        assert!(!is_valid_hostname(&long));
    }

    // ---- the two-writers rule ----

    #[test]
    fn a_stateful_app_may_not_swap() {
        let mut app = sample();
        app.writes_state = true;
        assert!(app.strategy == Strategy::Swap, "precondition");
        let problems = app.problems();
        let hit = problems
            .iter()
            .find(|p| p.code == "swap-with-shared-state")
            .expect("swap on a stateful app must be an error");
        assert!(hit.is_error());
        assert!(
            hit.message.contains("replace"),
            "the message should say what to do instead: {}",
            hit.message
        );
    }

    #[test]
    fn a_stateful_app_may_replace() {
        let mut app = sample();
        app.writes_state = true;
        app.strategy = Strategy::Replace;
        assert!(app.problems().is_empty(), "{:?}", app.problems());
    }

    // ---- port checks ----

    #[test]
    fn a_front_port_inside_the_reserved_range_is_rejected() {
        let mut app = sample();
        app.front_port = 9001;
        assert!(codes(&app).contains(&"front-port-reserved"));
    }

    #[test]
    fn a_live_port_outside_its_slot_pair_is_rejected() {
        let mut app = sample();
        app.live_port = Some(9100);
        assert!(codes(&app).contains(&"port-not-in-slot"));
    }

    #[test]
    fn live_and_old_may_not_be_the_same_port() {
        let mut app = sample();
        app.old_port = app.live_port;
        assert!(codes(&app).contains(&"live-equals-old"));
    }

    #[test]
    fn a_slot_beyond_the_maximum_is_rejected() {
        let mut app = sample();
        app.slot = MAX_SLOTS;
        assert!(codes(&app).contains(&"slot-out-of-range"));
    }

    // ---- kind-conditional rules ----

    #[test]
    fn a_static_app_needs_an_absolute_root() {
        let mut app = sample_static();
        app.root = Some("relative/path".into());
        assert!(codes(&app).contains(&"root-not-absolute"));

        app.root = None;
        assert!(codes(&app).contains(&"root-missing"));
    }

    #[test]
    fn a_static_app_may_not_use_replace() {
        let mut app = sample_static();
        app.strategy = Strategy::Replace;
        assert!(codes(&app).contains(&"static-strategy"));
    }

    #[test]
    fn a_container_app_needs_a_health_url() {
        let mut app = sample();
        app.health_url = None;
        assert!(codes(&app).contains(&"health-url-missing"));

        app.health_url = Some("127.0.0.1/".into());
        assert!(codes(&app).contains(&"health-url-invalid"));
    }

    #[test]
    fn a_container_app_needs_an_image_repo() {
        let mut app = sample();
        app.image_repo = None;
        assert!(codes(&app).contains(&"image-repo-missing"));
    }

    // ---- release identity ----

    #[test]
    fn latest_is_rejected_as_a_release() {
        let mut app = sample();
        app.release = Some("latest".into());
        let codes = codes(&app);
        assert!(codes.contains(&"release-is-latest"), "{codes:?}");
        // It is also not a commit name, so both codes may fire.
        assert!(codes.contains(&"release-not-a-commit-name"), "{codes:?}");
    }

    #[test]
    fn a_release_that_is_not_a_commit_name_is_rejected() {
        for bad in [
            "abc",
            "9c1f2abZZZ",
            "",
            "9c1f2ab-too-long-to-be-a-short-sha-but-long-enough",
        ] {
            let mut app = sample();
            app.release = Some(bad.into());
            assert!(
                codes(&app).contains(&"release-not-a-commit-name"),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_short_sha_is_accepted() {
        let forty = "a".repeat(40);
        for good in ["9c1f2ab", "9C1F2AB", forty.as_str()] {
            let mut app = sample();
            app.release = Some(good.to_string());
            assert!(
                !codes(&app).contains(&"release-not-a-commit-name"),
                "{good:?} must be accepted"
            );
        }
    }

    // ---- name and wiring validation ----

    #[test]
    fn name_syntax_is_checked() {
        // An empty name reports `name-empty`, which has its own test below.
        for bad in [
            "Portfolio",
            "-lead",
            "trail-",
            "has_underscore",
            "has space",
        ] {
            let mut app = sample();
            app.name = bad.into();
            assert!(codes(&app).contains(&"name-invalid"), "{bad:?}");
        }
        for good in ["a", "portfolio", "chat-alt", "a1", "charts", "files"] {
            assert!(is_valid_slug(good), "{good:?} must be accepted");
        }
    }

    #[test]
    fn the_default_registry_is_ghcr() {
        assert_eq!(ImageRegistry::default(), ImageRegistry::Ghcr);
    }

    #[test]
    fn an_empty_name_gets_its_own_code() {
        let mut app = sample();
        app.name = String::new();
        let found = codes(&app);
        assert!(found.contains(&"name-empty"), "{found:?}");
        assert!(!found.contains(&"name-invalid"), "{found:?}");
    }

    #[test]
    fn compose_and_env_wiring_is_checked() {
        let mut app = sample();
        app.compose_dir = PathBuf::from("relative");
        assert!(codes(&app).contains(&"compose-dir-not-absolute"));

        let mut app = sample();
        app.compose_svc = "_bad".into();
        assert!(codes(&app).contains(&"compose-svc-invalid"));

        let mut app = sample();
        app.env_name = "lowercase".into();
        assert!(codes(&app).contains(&"env-name-invalid"));

        let mut app = sample();
        app.env_name = "1PORT".into();
        assert!(codes(&app).contains(&"env-name-invalid"));
    }

    #[test]
    fn a_listen_address_must_be_an_ip() {
        let mut app = sample();
        app.listen = vec!["127.0.0.1".into(), "not-an-ip".into()];
        assert!(codes(&app).contains(&"listen-invalid"));
    }

    #[test]
    fn a_second_listen_address_is_allowed() {
        let mut app = sample();
        app.listen = vec!["127.0.0.1".into(), "192.0.2.10".into()];
        assert!(app.problems().is_empty(), "{:?}", app.problems());
    }

    #[test]
    fn a_root_that_would_break_the_directive_is_rejected() {
        for bad in [
            "/srv/www/with space",
            "/srv/www/app;rm",
            "/srv/www/\"quoted\"",
            "/srv/www/$var",
            "/srv/www/{brace}",
            "/srv/www/back\\slash",
        ] {
            let mut app = sample_static();
            app.root = Some(bad.into());
            assert!(
                codes(&app).contains(&"root-unsafe"),
                "{bad:?} must be rejected, got {:?}",
                codes(&app)
            );
        }
        for good in ["/srv/www/portfolio/current", "/var/www/docsdata"] {
            assert!(is_safe_nginx_value(good), "{good:?}");
        }
    }

    #[test]
    fn an_empty_hostname_list_is_a_warning_not_an_error() {
        let mut app = sample();
        app.hostnames = vec![];
        let problems = app.problems();
        let hit = problems
            .iter()
            .find(|p| p.code == "hostnames-empty")
            .unwrap();
        assert!(!hit.is_error(), "a new app has no hostname yet");
    }

    // ---- loading a directory ----

    #[test]
    fn load_dir_reads_every_toml_file() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("portfolio.toml"), to_toml(&sample())).unwrap();
        let static_app = sample_static();
        std::fs::write(dir.path().join("docs.toml"), to_toml(&static_app)).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let loaded = load_dir(dir.path()).unwrap();
        assert_eq!(loaded.apps.len(), 2);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);

        let sorted = loaded.sorted();
        let names: Vec<&str> = sorted.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["docs", "portfolio"], "sorted by name");
    }

    #[test]
    fn load_dir_reports_a_file_that_does_not_parse_without_failing() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("portfolio.toml"), to_toml(&sample())).unwrap();
        std::fs::write(dir.path().join("broken.toml"), "this is not = = toml").unwrap();

        let loaded = load_dir(dir.path()).unwrap();
        assert_eq!(loaded.apps.len(), 1, "the good file still loads");
        assert!(
            loaded.problems.iter().any(|p| p.code == "file-unparseable"),
            "{:?}",
            loaded.problems
        );
    }

    #[test]
    fn load_dir_rejects_a_name_that_contradicts_the_file_name() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("charts.toml"), to_toml(&sample())).unwrap();
        let loaded = load_dir(dir.path()).unwrap();
        assert!(
            loaded
                .problems
                .iter()
                .any(|p| p.code == "filename-name-mismatch"),
            "{:?}",
            loaded.problems
        );
    }

    #[test]
    fn load_dir_on_a_missing_directory_is_empty_not_an_error() {
        let dir = TempDir::new().unwrap();
        let loaded = load_dir(&dir.path().join("nope")).unwrap();
        assert!(loaded.apps.is_empty());
        assert!(loaded.problems.is_empty());
    }

    #[test]
    fn advance_moves_the_release_chain_forward() {
        let mut app = sample();
        assert_eq!(app.release.as_deref(), Some("9c1f2ab"));
        assert_eq!(app.old_release.as_deref(), Some("4d8e0f1"));

        app.advance("d34db33".into(), 9001);

        assert_eq!(app.release.as_deref(), Some("d34db33"));
        assert_eq!(app.old_release.as_deref(), Some("9c1f2ab"));
        assert_eq!(app.live_port, Some(9001));
        assert_eq!(app.old_port, Some(9000));
        assert!(app.problems().is_empty(), "{:?}", app.problems());
    }

    #[test]
    fn advance_from_no_release_starts_the_chain() {
        let mut app = sample();
        app.release = None;
        app.old_release = None;
        app.live_port = None;

        app.advance("9c1f2ab".into(), 9000);

        assert_eq!(app.release.as_deref(), Some("9c1f2ab"));
        assert_eq!(app.old_release, None);
        assert_eq!(app.live_port, Some(9000));
    }

    #[test]
    fn image_ref_pins_the_release_as_the_tag() {
        let app = sample();
        assert_eq!(
            app.image_ref("d34db33").as_deref(),
            Some("apps-portfolio:d34db33")
        );

        let mut remote = sample();
        remote.image_repo = Some("ghcr.io/example-org/git".into());
        assert_eq!(
            remote.image_ref("abc1234").as_deref(),
            Some("ghcr.io/example-org/git:abc1234")
        );

        let mut none = sample();
        none.image_repo = None;
        assert_eq!(none.image_ref("abc1234"), None);
    }

    #[test]
    fn pull_is_skipped_for_local_images() {
        let mut app = sample();
        app.registry = Some(ImageRegistry::Local);
        assert!(!app.needs_pull());

        app.registry = None;
        assert!(app.needs_pull(), "default registry is remote");

        app.registry = Some(ImageRegistry::Ghcr);
        assert!(app.needs_pull());
    }

    #[test]
    fn compose_project_comes_from_the_directory_name() {
        let app = sample();
        assert_eq!(app.compose_project(), "apps");

        let mut jobs = sample();
        jobs.compose_dir = PathBuf::from("/home/marv/jobs");
        assert_eq!(jobs.compose_project(), "jobs");
    }

    #[test]
    fn green_container_name_derives_from_the_service() {
        assert_eq!(sample().green_container_name(), "portfolio-green");
    }

    #[test]
    fn save_app_round_trips_through_the_filesystem() {
        let dir = TempDir::new().unwrap();
        let mut app = sample();
        app.advance("d34db33".into(), 9001);
        save_app(dir.path(), &app).unwrap();

        let raw = std::fs::read_to_string(dir.path().join("portfolio.toml")).unwrap();
        let back: App = toml::from_str(&raw).unwrap();
        assert_eq!(back, app);
        assert!(!dir.path().join(".portfolio.toml.staging").exists());
    }

    #[test]
    fn save_app_preserves_the_existing_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let mut app = sample();
        app.advance("d34db33".into(), 9001);
        save_app(dir.path(), &app).unwrap();

        let path = dir.path().join("portfolio.toml");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

        app.advance("b00b1e5".into(), 9000);
        save_app(dir.path(), &app).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "rewrite must keep the operator's mode");
    }

    #[test]
    fn save_app_with_a_path_separator_fails_instead_of_writing_elsewhere() {
        // `App.name` is validated to a slug long before saving, so this cannot
        // occur through the CLI. The save still must not write outside the dir.
        let dir = TempDir::new().unwrap();
        let mut app = sample();
        app.name = "../escape".into();
        let err = save_app(dir.path(), &app).unwrap_err();
        assert!(
            !dir.path().parent().unwrap().join("escape.toml").exists(),
            "must not write outside the registry dir (got {err})"
        );
    }

    #[test]
    fn loaded_errors_are_counted() {
        let mut loaded = Loaded::default();
        loaded.problems.push(Problem::warning("w", None, "w"));
        assert_eq!(loaded.errors(), 0);
        loaded.problems.push(Problem::error("e", None, "e"));
        assert_eq!(loaded.errors(), 1);
    }
}
