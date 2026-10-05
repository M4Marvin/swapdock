//! Tests against the real registry in `examples/`.
//!
//! The unit tests prove the logic. These prove the *data* — the 13 apps actually
//! running on the swapdock host, cross-checked against the tunnel config that is
//! actually in use. That turns "the validator works" into "the validator agrees
//! with production", and catches a hand-edited registry file immediately.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use swapdock::registry::{self, App};
use swapdock::render;
use swapdock::tunnel;
use swapdock::validator::{self, TunnelRoutes};

fn examples(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(sub)
}

/// The real apps, with the ports the tunnel sends them to.
const EXPECTED_APPS: &[(&str, u16)] = &[
    ("portfolio", 8001),
    ("morphotech", 8012),
    ("charts", 8006),
    ("forgejo", 8003),
    ("vaultwarden", 8004),
    ("kuma", 8005),
    ("copyparty", 8009),
    ("chat", 8002),
    ("chats", 8007),
    ("beszel", 8008),
    ("zeroclaw", 42617),
    ("ui", 8011),
    ("api", 8010),
];

fn load() -> registry::Loaded {
    registry::load_dir(&examples("apps")).expect("examples/apps must be readable")
}

fn routes() -> TunnelRoutes {
    tunnel::read_config(&examples("cloudflared.yaml"))
        .expect("examples/cloudflared.yaml must be readable")
        .routes
}

#[test]
fn the_example_registry_loads_without_a_single_problem() {
    let loaded = load();
    assert!(
        loaded.problems.is_empty(),
        "load problems: {:#?}",
        loaded.problems
    );
    assert_eq!(loaded.apps.len(), EXPECTED_APPS.len(), "app count changed");
}

#[test]
fn every_app_passes_its_own_checks() {
    // `hostnames-empty` is the one warning an app may carry while it is being
    // onboarded; anything else means the file is wrong.
    for app in load().apps {
        let errors: Vec<&str> = app
            .problems()
            .iter()
            .filter(|p| p.is_error())
            .map(|p| p.code)
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", app.name);

        let unexpected: Vec<&str> = app
            .problems()
            .iter()
            .filter(|p| !p.is_error() && p.code != "hostnames-empty")
            .map(|p| p.code)
            .collect();
        assert!(unexpected.is_empty(), "{}: {unexpected:?}", app.name);
    }
}

#[test]
fn the_front_ports_match_the_live_tunnel_config() {
    let apps = load().sorted();
    let routes = routes();
    let mut checked = 0;

    for app in &apps {
        for host in app.normalized_hostnames() {
            let port = routes
                .get(&host)
                .unwrap_or_else(|| panic!("the tunnel has no route for {host}"));
            assert_eq!(
                port, app.front_port,
                "{}: the tunnel sends {host} to {port} but the registry says {}",
                app.name, app.front_port
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 10, "the tunnel routes 10 hostnames");
}

#[test]
fn no_tunnel_route_is_orphaned_in_the_real_data() {
    let apps = load().sorted();
    let problems = validator::validate(&apps, &routes());
    let orphans: Vec<&str> = problems
        .iter()
        .filter(|p| p.code == "orphaned-tunnel-route")
        .map(|p| p.message.as_str())
        .collect();
    assert!(
        orphans.is_empty(),
        "every tunnel hostname must be claimed by an app: {orphans:#?}"
    );
}

#[test]
fn the_real_registry_has_no_errors() {
    let loaded = load();
    let apps = loaded.sorted();

    let mut problems = loaded.problems.clone();
    for app in &apps {
        problems.extend(app.problems());
    }
    problems.extend(validator::validate(&apps, &routes()));
    validator::sort_problems(&mut problems);

    let (errors, warnings) = validator::counts(&problems);
    assert_eq!(errors, 0, "real registry has errors: {problems:#?}");
    // Apps with no hostname yet are the only thing tolerated here.
    assert!(
        problems
            .iter()
            .all(|p| p.severity == registry::Severity::Warning),
        "unexpected warnings: {problems:#?}"
    );
    assert_eq!(warnings, 4, "the four apps with no hostname yet");
}

#[test]
fn every_slot_is_distinct() {
    let apps = load().sorted();
    let slots: BTreeSet<u8> = apps.iter().map(|a| a.slot).collect();
    assert_eq!(slots.len(), apps.len(), "two apps share a slot");
    assert_eq!(*slots.iter().next_back().unwrap(), 12, "slots 0..=12");
}

#[test]
fn every_stateful_app_uses_replace() {
    // The rule that a file-backed database must not be swapped is enforced in
    // App::problems, so this is a belt-and-braces check on real data.
    for app in load().apps {
        if app.writes_state {
            assert_eq!(
                app.strategy,
                registry::Strategy::Replace,
                "{} writes local state and must not use swap",
                app.name
            );
        }
    }
}

#[test]
fn only_the_stateless_apps_are_swapped() {
    let loaded = load();
    let swapped: Vec<&str> = loaded
        .apps
        .iter()
        .filter(|a| a.strategy == registry::Strategy::Swap)
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(
        swapped,
        ["morphotech", "portfolio"],
        "swap candidates changed"
    );
}

#[test]
fn chat_binds_loopback_and_the_tailnet() {
    let apps = load().sorted();
    let chat = apps.iter().find(|a| a.name == "chat").expect("chat");
    assert_eq!(
        chat.listen_addrs(),
        vec!["127.0.0.1".to_string(), "100.80.96.4".to_string()],
        "chat must stay reachable over the tailnet"
    );
}

#[test]
fn beszel_is_tailnet_only() {
    let apps = load().sorted();
    let beszel = apps.iter().find(|a| a.name == "beszel").expect("beszel");
    assert_eq!(beszel.listen_addrs(), vec!["100.80.96.4".to_string()]);
    assert!(
        beszel.hostnames.is_empty(),
        "beszel is not routed through the tunnel"
    );
}

#[test]
fn rendering_the_real_registry_is_deterministic() {
    let apps = load().sorted();
    let first = render::render(&apps);
    let second = render::render(&load().sorted());
    assert_eq!(first, second, "render must be byte-identical");

    // And independent of the order the files happened to be read in.
    let mut reversed = load().sorted();
    reversed.reverse();
    assert_eq!(first, render::render(&reversed));
}

#[test]
fn nothing_is_fronted_yet_so_no_listener_is_emitted() {
    // Today every container still publishes its own front port, so nginx cannot
    // bind any of them. Rendering blocks for all of them would be a config that
    // fails to load.
    let apps = load().sorted();
    assert!(
        apps.iter().all(|a: &App| a.live_port.is_none()),
        "an example app has a live_port; update this test and the migration state"
    );

    let out = render::render(&apps);
    assert!(!out.contains("server {"), "{out}");
    assert!(!out.contains("listen "), "{out}");
    assert_eq!(out.matches("# not yet fronted").count(), apps.len());
}

#[test]
fn a_migrated_app_renders_a_block_pointing_at_its_back_port() {
    let loaded = load();
    let mut app = loaded
        .apps
        .into_iter()
        .find(|a| a.name == "portfolio")
        .expect("portfolio");
    app.live_port = Some(9001);
    app.old_port = Some(9000);
    app.release = Some("9c1f2ab".into());

    let out = render::render(&[app]);
    assert!(out.contains("listen 127.0.0.1:8001;"), "{out}");
    assert!(out.contains("proxy_pass http://127.0.0.1:9001;"), "{out}");
    assert!(out.contains("server_name m4marvin.com;"), "{out}");
    assert!(out.contains("release 9c1f2ab"), "{out}");
    assert!(!out.contains("not yet fronted"), "{out}");
}

#[test]
fn the_rendered_config_is_free_of_untrusted_characters() {
    // Anything that could break out of an nginx directive must have been caught
    // by validation, so a config that renders with zero errors is safe to write.
    let apps = load().sorted();
    let mut problems = Vec::new();
    for app in &apps {
        problems.extend(app.problems());
    }
    problems.extend(validator::validate(&apps, &routes()));
    assert_eq!(validator::counts(&problems).0, 0);

    for app in &apps {
        for host in &app.hostnames {
            assert!(registry::is_valid_hostname(host), "{host:?}");
        }
        // Every value interpolated into a directive must be safe to interpolate.
        if let Some(root) = &app.root {
            assert!(
                registry::is_safe_nginx_value(root),
                "{}: {root:?}",
                app.name
            );
        }
        for addr in app.listen_addrs() {
            assert!(
                addr.parse::<std::net::IpAddr>().is_ok(),
                "{}: {addr:?}",
                app.name
            );
        }
    }
}
