//! Integration tests for `swapdock import`.
//!
//! These run the real binary against a temporary compose project and a temporary
//! registry. `docker compose config` is read-only, so no container is started,
//! stopped or built. Without Docker Compose v2 the tests skip with a clear
//! message rather than failing for an unrelated reason.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use swapdock::registry::{self, ImageRegistry, Strategy};
use tempfile::TempDir;

const SINGLE: &str = r#"
services:
  web:
    image: ghcr.io/acme/shop:1.4.2
    ports:
      - "127.0.0.1:3000:3000"
    healthcheck:
      test: ["CMD-SHELL", "wget -q -O /dev/null http://127.0.0.1:3000/healthz || exit 1"]
"#;

const STATEFUL: &str = r#"
services:
  web:
    image: ghcr.io/acme/shop:1.4.2
    ports:
      - "127.0.0.1:3000:3000"
    volumes:
      - shop-data:/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://127.0.0.1:3000/up"]
volumes:
  shop-data:
"#;

const MULTI: &str = r#"
services:
  web:
    image: nginx:alpine
  db:
    image: postgres:16
"#;

const NO_HEALTHCHECK: &str = r#"
services:
  web:
    image: nginx:alpine
"#;

/// `docker compose` must exist, but no daemon is needed for `config`.
fn compose_available() -> bool {
    let available = Command::new("docker")
        .args(["compose", "version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !available {
        eprintln!(
            "SKIP tests/import.rs: `docker compose` is not available; install Docker Compose v2 \
             to run these tests"
        );
    }
    available
}

struct Fixture {
    /// Keeps the temporary directory alive for the test.
    _dir: TempDir,
    root: PathBuf,
    compose_dir: PathBuf,
    registry: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new(suffix: &str, compose: &str) -> Self {
        let dir = TempDir::new().unwrap();
        // Compose project names must be lowercase and start with a letter/digit.
        // The suffix keeps parallel tests in one process from sharing a project.
        let project = format!("import{}{suffix}", std::process::id());
        let compose_dir = dir.path().join(&project);
        std::fs::create_dir_all(&compose_dir).unwrap();
        std::fs::write(compose_dir.join("compose.yaml"), compose).unwrap();
        let root = dir.path().to_path_buf();
        let registry = root.join("apps");
        let state = root.join("state/green");
        Self {
            _dir: dir,
            root,
            compose_dir,
            registry,
            state,
        }
    }

    fn run(&self, extra: &[&str]) -> Output {
        let mut args: Vec<String> = vec![
            "--registry".into(),
            self.registry.display().to_string(),
            "import".into(),
            "--compose-dir".into(),
            self.compose_dir.display().to_string(),
            "--state-dir".into(),
            self.state.display().to_string(),
        ];
        args.extend(extra.iter().map(|arg| arg.to_string()));
        Command::new(env!("CARGO_BIN_EXE_swapdock"))
            .args(&args)
            .current_dir(&self.root)
            .output()
            .expect("the swapdock test binary must run")
    }

    fn override_path(&self, name: &str) -> PathBuf {
        self.state
            .parent()
            .expect("state has a parent")
            .join("overrides")
            .join(format!("{name}.yml"))
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "import failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

#[test]
fn a_dry_run_prints_the_plan_and_writes_nothing() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("dry", SINGLE);
    let output = fixture.run(&[
        "--name",
        "shop",
        "--domain",
        "shop.example.com",
        "--dry-run",
    ]);
    assert_ok(&output);

    let text = stdout(&output);
    assert!(text.contains("dry run"), "{text}");
    assert!(text.contains("name = \"shop\""), "{text}");
    assert!(text.contains("compose override (would write)"), "{text}");
    assert!(text.contains("!override"), "{text}");
    assert!(
        !fixture.registry.join("shop.toml").exists(),
        "a dry run must not write the registry file"
    );
    assert!(
        !fixture.override_path("shop").exists(),
        "a dry run must not write the override"
    );
    assert!(
        !fixture.registry.exists(),
        "a dry run must not create the registry directory"
    );
}

#[test]
fn an_import_writes_a_valid_registry_entry_and_override() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("live", SINGLE);
    let output = fixture.run(&["--name", "shop", "--domain", "shop.example.com"]);
    assert_ok(&output);

    let loaded = registry::load_dir(&fixture.registry).unwrap();
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
    assert_eq!(loaded.apps.len(), 1, "expected one imported app");
    let app = &loaded.apps[0];

    assert_eq!(app.name, "shop");
    assert_eq!(app.compose_svc, "web");
    assert_eq!(
        app.compose_dir,
        std::fs::canonicalize(&fixture.compose_dir).unwrap()
    );
    assert_eq!(app.image_repo.as_deref(), Some("ghcr.io/acme/shop"));
    assert_eq!(app.front_port, 8001);
    assert_eq!(app.slot, 0);
    assert_eq!(app.strategy, Strategy::Swap);
    assert!(!app.writes_state);
    assert_eq!(app.hostnames, vec!["shop.example.com".to_string()]);
    assert_eq!(app.health_url.as_deref(), Some("http://127.0.0.1/healthz"));
    assert_eq!(app.env_name, "SHOP_PORT");
    assert_eq!(app.registry, Some(ImageRegistry::Local));
    assert_eq!(app.build_host.as_deref(), Some("local"));
    assert_eq!(app.branch.as_deref(), Some("main"));
    assert!(app.live_port.is_none() && app.old_port.is_none());
    assert!(
        app.problems().iter().all(|problem| !problem.is_error()),
        "{:?}",
        app.problems()
    );

    let override_path = app
        .compose_override
        .as_ref()
        .expect("import records the generated override");
    assert!(override_path.is_absolute(), "{override_path:?}");
    assert_eq!(override_path, &fixture.override_path("shop"));
    let override_text = std::fs::read_to_string(override_path).unwrap();
    assert!(
        override_text.contains("${IMAGE:-ghcr.io/acme/shop}"),
        "{override_text}"
    );
    assert!(
        override_text.contains("127.0.0.1:${SHOP_PORT:-9000}:3000"),
        "{override_text}"
    );

    let text = stdout(&output);
    assert!(text.contains("wrote"), "{text}");
    assert!(text.contains("next steps"), "{text}");
    assert!(text.contains("swapdock"), "{text}");

    // A second import must not overwrite the first.
    let again = fixture.run(&["--name", "shop", "--domain", "shop.example.com"]);
    assert!(!again.status.success(), "a second import must be refused");
    let error = stderr(&again);
    assert!(error.contains("already exists"), "{error}");
}

#[test]
fn named_volumes_choose_replace_and_state() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("state", STATEFUL);
    let output = fixture.run(&["--name", "shop", "--domain", "shop.example.com"]);
    assert_ok(&output);

    let loaded = registry::load_dir(&fixture.registry).unwrap();
    let app = &loaded.apps[0];
    assert_eq!(app.strategy, Strategy::Replace);
    assert!(app.writes_state);
    assert_eq!(app.strategy.as_str(), "replace");
    assert!(stdout(&output).contains("replace"), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("named volume"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn several_services_require_the_service_flag() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("multi", MULTI);
    let output = fixture.run(&["--name", "shop"]);
    assert!(!output.status.success(), "ambiguous imports must fail");
    let error = stderr(&output);
    assert!(error.contains("--service"), "{error}");
    assert!(error.contains("db") && error.contains("web"), "{error}");
    assert!(!fixture.registry.join("shop.toml").exists());
}

#[test]
fn a_missing_healthcheck_is_a_warning_and_the_flag_installs_one() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("health", NO_HEALTHCHECK);
    let output = fixture.run(&[
        "--name",
        "shop",
        "--healthcheck",
        "curl -f http://127.0.0.1/health",
    ]);
    assert_ok(&output);

    let loaded = registry::load_dir(&fixture.registry).unwrap();
    let app = &loaded.apps[0];
    assert_eq!(app.health_url.as_deref(), Some("http://127.0.0.1/health"));
    let override_text = std::fs::read_to_string(app.compose_override.as_ref().unwrap()).unwrap();
    assert!(override_text.contains("healthcheck:"), "{override_text}");
    assert!(
        override_text.contains("curl -f http://127.0.0.1/health"),
        "{override_text}"
    );
}

#[test]
fn no_healthcheck_without_the_flag_still_imports_with_a_warning() {
    if !compose_available() {
        return;
    }
    let fixture = Fixture::new("nohealth", NO_HEALTHCHECK);
    let output = fixture.run(&["--name", "shop"]);
    assert_ok(&output);

    let error = stderr(&output);
    assert!(error.contains("no healthcheck"), "{error}");
    assert!(error.contains("deploy gate"), "{error}");

    let loaded = registry::load_dir(&fixture.registry).unwrap();
    let app = &loaded.apps[0];
    assert_eq!(app.health_url.as_deref(), Some("http://127.0.0.1/"));
    let override_text = std::fs::read_to_string(app.compose_override.as_ref().unwrap()).unwrap();
    assert!(!override_text.contains("healthcheck:"), "{override_text}");
}
