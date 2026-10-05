//! Tests against the fictional estate in `examples/`.
//!
//! The unit tests prove the logic. These prove the *shape* — six apps covering
//! every supported configuration (swap, replace with state, static files, a
//! dual-homed service, an unrouted internal), cross-checked against a matching
//! tunnel config. A newcomer reads these as documentation; a broken example
//! fails the build instead of misleading them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use swapdock::registry::{self, App, Kind};
use swapdock::render;
use swapdock::tunnel;
use swapdock::validator::{self, TunnelRoutes};

fn examples(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(sub)
}

/// The fictional apps, with the ports the tunnel sends them to.
const EXPECTED_APPS: &[(&str, u16)] = &[
    ("site", 8001),
    ("shop", 8002),
    ("docs", 8003),
    ("git", 8004),
    ("chat", 8005),
    ("metrics", 8006),
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
    let mut names: Vec<&str> = loaded.apps.iter().map(|a| a.name.as_str()).collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = EXPECTED_APPS.iter().map(|(n, _)| *n).collect();
    expected.sort_unstable();
    assert_eq!(names, expected, "estate membership changed");
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
    assert_eq!(checked, 6, "the tunnel routes 6 hostnames");
}

#[test]
fn no_tunnel_route_is_orphaned_in_the_example_data() {
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
    assert_eq!(errors, 0, "example registry has errors: {problems:#?}");
    // Apps with no hostname yet are the only thing tolerated here.
    assert!(
        problems
            .iter()
            .all(|p| p.severity == registry::Severity::Warning),
        "unexpected warnings: {problems:#?}"
    );
    assert_eq!(warnings, 1, "only metrics has no hostname yet");
}

#[test]
fn every_slot_is_distinct() {
    let apps = load().sorted();
    let slots: BTreeSet<u8> = apps.iter().map(|a| a.slot).collect();
    assert_eq!(slots.len(), apps.len(), "two apps share a slot");
    assert_eq!(*slots.iter().next_back().unwrap(), 5, "slots 0..=5");
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
    let mut swapped: Vec<&str> = loaded
        .apps
        .iter()
        .filter(|a| a.strategy == registry::Strategy::Swap)
        .map(|a| a.name.as_str())
        .collect();
    swapped.sort_unstable();
    assert_eq!(swapped, ["docs", "site"], "swap candidates changed");
}

#[test]
fn chat_binds_loopback_and_the_private_net() {
    // 192.0.2.10 is TEST-NET-1 (RFC 5737): documentation-only, never routed.
    let apps = load().sorted();
    let chat = apps.iter().find(|a| a.name == "chat").expect("chat");
    assert_eq!(
        chat.listen_addrs(),
        vec!["127.0.0.1".to_string(), "192.0.2.10".to_string()],
        "chat must stay reachable on the private address"
    );
}

#[test]
fn metrics_is_private_only() {
    let apps = load().sorted();
    let metrics = apps.iter().find(|a| a.name == "metrics").expect("metrics");
    assert_eq!(metrics.listen_addrs(), vec!["192.0.2.10".to_string()]);
    assert!(
        metrics.hostnames.is_empty(),
        "metrics is not routed through the tunnel"
    );
}

#[test]
fn rendering_the_example_registry_is_deterministic() {
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
    // The example estate starts unmigrated: every container still publishes its
    // own front port, so nginx cannot bind any of them. Rendering blocks for
    // all of them would be a config that fails to load. Static apps have no
    // port at all, so docs is the one block that does render.
    let apps = load().sorted();
    assert!(
        apps.iter().all(|a: &App| a.live_port.is_none()),
        "an example app has a live_port; update this test and the migration state"
    );

    let out = render::render(&apps);
    let containers: Vec<&App> = apps
        .iter()
        .filter(|a| a.kind == registry::Kind::Container)
        .collect();
    assert_eq!(out.matches("server {").count(), 1, "only docs renders: {out}");
    assert!(out.contains("root /srv/www/docs/current;"), "{out}");
    assert_eq!(
        out.matches("# not yet fronted").count(),
        containers.len(),
        "{out}"
    );
}

#[test]
fn a_migrated_app_renders_a_block_pointing_at_its_back_port() {
    let loaded = load();
    let mut app = loaded
        .apps
        .into_iter()
        .find(|a| a.name == "site")
        .expect("site");
    app.live_port = Some(9000);
    app.old_port = Some(9001);
    app.release = Some("abc1234".into());

    let out = render::render(&[app]);
    assert!(out.contains("listen 127.0.0.1:8001;"), "{out}");
    assert!(out.contains("proxy_pass http://127.0.0.1:9000;"), "{out}");
    assert!(out.contains("server_name example.com;"), "{out}");
    assert!(out.contains("release abc1234"), "{out}");
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
