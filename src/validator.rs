//! Cross-app validation.
//!
//! Single-app checks live in [`crate::registry::App::problems`]. This module
//! covers what can only be seen by looking at every app together, which is where
//! the dangerous mistakes are: two apps claiming one port means nginx fails to
//! start or silently binds only one of them.
//!
//! Pure as well. The tunnel routes are passed in rather than read, so this can be
//! tested without cloudflared installed and without a running tunnel.
//!
//! Findings are sorted before they are returned, so two runs over the same
//! registry produce byte-identical output and a diff means something changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::registry::{App, Problem, normalize_hostname};
use crate::render;

/// Hostname to front port, as the tunnel currently routes.
#[derive(Debug, Default, Clone)]
pub struct TunnelRoutes {
    routes: BTreeMap<String, u16>,
}

impl TunnelRoutes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one route. The hostname is normalised, so casing and a trailing dot
    /// cannot disguise a duplicate.
    pub fn add(&mut self, hostname: &str, front_port: u16) -> &mut Self {
        self.routes.insert(normalize_hostname(hostname), front_port);
        self
    }

    pub fn get(&self, hostname: &str) -> Option<u16> {
        self.routes.get(&normalize_hostname(hostname)).copied()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn hostnames(&self) -> impl Iterator<Item = &str> {
        self.routes.keys().map(String::as_str)
    }
}

/// Every check that needs more than one app.
pub fn validate(apps: &[App], tunnel: &TunnelRoutes) -> Vec<Problem> {
    let mut out = Vec::new();

    check_duplicate_names(apps, &mut out);
    check_duplicate_hostnames(apps, &mut out);
    check_duplicate_listeners(apps, &mut out);
    check_duplicate_slots(apps, &mut out);
    check_live_ports(apps, &mut out);
    check_old_ports(apps, &mut out);
    check_front_ports_against_reserved(apps, &mut out);
    if !tunnel.is_empty() {
        check_tunnel_routes(apps, tunnel, &mut out);
    }

    sort_problems(&mut out);
    out
}

fn check_duplicate_names(apps: &[App], out: &mut Vec<Problem>) {
    let mut seen = BTreeSet::new();
    for app in apps {
        if !seen.insert(app.name.clone()) {
            out.push(Problem::error(
                "duplicate-name",
                Some(&app.name),
                "two registry files declare this name",
            ));
        }
    }
}

fn check_duplicate_hostnames(apps: &[App], out: &mut Vec<Problem>) {
    // hostname -> apps claiming it
    let mut owners: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for app in apps {
        for host in app.normalized_hostnames() {
            owners.entry(host).or_default().push(app.name.as_str());
        }
    }
    for (host, mut claimants) in owners {
        claimants.sort_unstable();
        claimants.dedup();
        if claimants.len() > 1 {
            out.push(Problem::error(
                "duplicate-hostname",
                Some(claimants[0]),
                format!(
                    "hostname {host:?} is claimed by {}: traffic would go to only one",
                    claimants.join(", ")
                ),
            ));
        }
    }
}

fn check_duplicate_listeners(apps: &[App], out: &mut Vec<Problem>) {
    // (address, port) -> apps. Two apps on one address and port is the case that
    // makes nginx refuse to start, or bind only the first block it read.
    let mut owners: BTreeMap<(String, u16), Vec<&str>> = BTreeMap::new();
    for app in apps {
        for (addr, port) in render::rendered_listeners(std::slice::from_ref(app)) {
            owners
                .entry((addr, port))
                .or_default()
                .push(app.name.as_str());
        }
    }
    for ((addr, port), mut claimants) in owners {
        claimants.sort_unstable();
        claimants.dedup();
        if claimants.len() > 1 {
            out.push(Problem::error(
                "duplicate-listener",
                Some(claimants[0]),
                format!(
                    "{addr}:{port} is claimed by {}: nginx can bind only one",
                    claimants.join(", ")
                ),
            ));
        }
    }
}

fn check_duplicate_slots(apps: &[App], out: &mut Vec<Problem>) {
    let mut owners: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
    for app in apps {
        owners.entry(app.slot).or_default().push(app.name.as_str());
    }
    for (slot, mut claimants) in owners {
        claimants.sort_unstable();
        claimants.dedup();
        if claimants.len() > 1 {
            out.push(Problem::error(
                "duplicate-slot",
                Some(claimants[0]),
                format!(
                    "slot {slot} is shared by {}: they would use the same back-end ports",
                    claimants.join(", ")
                ),
            ));
        }
    }
}

fn check_live_ports(apps: &[App], out: &mut Vec<Problem>) {
    let mut owners: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    for app in apps {
        if let Some(port) = app.live_port {
            owners.entry(port).or_default().push(app.name.as_str());
        }
    }
    for (port, mut claimants) in owners {
        claimants.sort_unstable();
        claimants.dedup();
        if claimants.len() > 1 {
            out.push(Problem::error(
                "duplicate-live-port",
                Some(claimants[0]),
                format!(
                    "live_port {port} is claimed by {}: one would shadow the other",
                    claimants.join(", ")
                ),
            ));
        }
    }
}

fn check_old_ports(apps: &[App], out: &mut Vec<Problem>) {
    // A rollback target that another app is already serving on is a trap: the
    // rollback looks fine until the next swapdock of the other app.
    let live: BTreeMap<u16, &str> = apps
        .iter()
        .filter_map(|a| a.live_port.map(|p| (p, a.name.as_str())))
        .collect();

    let mut reported: BTreeSet<(String, u16)> = BTreeSet::new();
    for app in apps {
        let Some(old) = app.old_port else { continue };
        if let Some(other) = live.get(&old)
            && *other != app.name
            && reported.insert((app.name.clone(), old))
        {
            out.push(Problem::error(
                "old-port-in-use",
                Some(&app.name),
                format!(
                    "old_port {old} is the live_port of {other:?}: rolling back would \
                     collide with it"
                ),
            ));
        }
    }
}

fn check_front_ports_against_reserved(apps: &[App], out: &mut Vec<Problem>) {
    // Defense in depth: App::problems already refuses a front port inside the
    // reserved range. This catches it again across the whole estate, in case a
    // caller assembled the list without running the per-app checks.
    for app in apps {
        for (addr, port) in render::rendered_listeners(std::slice::from_ref(app)) {
            if crate::ports::is_reserved(port) {
                out.push(Problem::error(
                    "front-port-reserved",
                    Some(&app.name),
                    format!(
                        "{addr}:{port} is inside the reserved back-end range {}..={}",
                        crate::ports::RESERVED_BACK.start(),
                        crate::ports::RESERVED_BACK.end()
                    ),
                ));
            }
        }
    }
}

fn check_tunnel_routes(apps: &[App], tunnel: &TunnelRoutes, out: &mut Vec<Problem>) {
    // Hostnames the registry claims that the tunnel does not route.
    for app in apps {
        for host in app.normalized_hostnames() {
            match tunnel.get(&host) {
                None => out.push(Problem::error(
                    "no-tunnel-route",
                    Some(&app.name),
                    format!("the tunnel has no route for hostname {host:?}"),
                )),
                Some(port) if port != app.front_port => out.push(Problem::error(
                    "tunnel-route-wrong-port",
                    Some(&app.name),
                    format!(
                        "the tunnel routes {host:?} to port {port}, but this app's \
                         front_port is {}",
                        app.front_port
                    ),
                )),
                Some(_) => {}
            }
        }
    }

    // Hostnames the tunnel routes that no app claims. A warning, because during
    // a migration an app may not have its registry file yet.
    let claimed: BTreeSet<String> = apps.iter().flat_map(|a| a.normalized_hostnames()).collect();
    for host in tunnel.hostnames() {
        if !claimed.contains(host) {
            out.push(Problem::warning(
                "orphaned-tunnel-route",
                None,
                format!("the tunnel routes {host:?} but no app claims it"),
            ));
        }
    }
}

/// Sorts findings so output is stable: by app, then code, then message.
pub fn sort_problems(problems: &mut [Problem]) {
    problems.sort_by(|a, b| {
        a.app
            .cmp(&b.app)
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
}

/// Counts findings by severity.
pub fn counts(problems: &[Problem]) -> (usize, usize) {
    let errors = problems.iter().filter(|p| p.is_error()).count();
    (errors, problems.len() - errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Kind;
    use crate::registry::tests::{sample, sample_static};

    fn container_named(name: &str, front: u16, slot: u8, host: &str) -> App {
        let mut app = sample();
        app.name = name.into();
        app.front_port = front;
        app.slot = slot;
        app.hostnames = vec![host.into()];
        app.compose_svc = name.into();
        app.env_name = format!("{}_PORT", name.to_uppercase().replace('-', "_"));
        app.live_port = Some(crate::ports::pair(slot).unwrap().0);
        app.old_port = Some(crate::ports::pair(slot).unwrap().1);
        app
    }

    fn codes(problems: &[Problem]) -> Vec<&'static str> {
        let mut c: Vec<&'static str> = problems.iter().map(|p| p.code).collect();
        c.sort_unstable();
        c
    }

    fn tunnel_of(apps: &[App]) -> TunnelRoutes {
        let mut t = TunnelRoutes::new();
        for app in apps {
            for host in &app.hostnames {
                t.add(host, app.front_port);
            }
        }
        t
    }

    #[test]
    fn a_clean_estate_has_no_cross_app_problems() {
        let apps = vec![
            container_named("charts", 8006, 1, "charts.m4marvin.com"),
            container_named("portfolio", 8001, 0, "m4marvin.com"),
        ];
        let tunnel = tunnel_of(&apps);
        assert_eq!(validate(&apps, &tunnel), vec![]);
    }

    #[test]
    fn an_empty_estate_is_valid() {
        assert!(validate(&[], &TunnelRoutes::new()).is_empty());
    }

    #[test]
    fn two_apps_claiming_one_hostname_is_an_error() {
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("mirror", 8002, 1, "m4marvin.com");
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-hostname"), "{found:?}");
    }

    #[test]
    fn a_hostname_collision_is_found_despite_different_casing() {
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("mirror", 8002, 1, "M4Marvin.COM");
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-hostname"), "{found:?}");
    }

    #[test]
    fn two_apps_listening_on_one_address_and_port_is_an_error() {
        // Distinct hostnames, so only the listener collides: exactly the case that
        // makes nginx refuse to start.
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("mirror", 8001, 1, "www.m4marvin.com");
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-listener"), "{found:?}");
    }

    #[test]
    fn the_same_port_on_different_addresses_is_allowed() {
        // chat binds loopback and the tailnet address on the same port. That is
        // two sockets, not a collision.
        let mut a = container_named("chat", 8002, 1, "chat.m4marvin.com");
        a.listen = vec!["127.0.0.1".into()];
        let mut b = container_named("beszel", 8002, 2, "status.m4marvin.com");
        b.listen = vec!["100.80.96.4".into()];
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(!found.contains(&"duplicate-listener"), "{found:?}");
    }

    #[test]
    fn two_apps_sharing_a_slot_is_an_error() {
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("mirror", 8002, 0, "www.m4marvin.com");
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-slot"), "{found:?}");
    }

    #[test]
    fn two_apps_live_on_one_port_is_an_error() {
        let mut a = container_named("portfolio", 8001, 0, "m4marvin.com");
        a.live_port = Some(9000);
        a.old_port = Some(9001);
        let mut b = container_named("charts", 8006, 1, "charts.m4marvin.com");
        b.live_port = Some(9000); // outside its own slot pair, but that is per-app
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-live-port"), "{found:?}");
    }

    #[test]
    fn a_rollback_target_used_by_another_app_is_an_error() {
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("charts", 8006, 1, "charts.m4marvin.com");
        let (a_port, b_live) = (a.old_port.unwrap(), b.live_port.unwrap());
        let mut a = a;
        a.old_port = Some(b_live);
        assert_eq!(a_port, 9001);
        assert_eq!(b_live, 9002);

        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"old-port-in-use"), "{found:?}");
    }

    #[test]
    fn an_app_may_roll_back_to_its_own_old_port() {
        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let b = container_named("charts", 8006, 1, "charts.m4marvin.com");
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(!found.contains(&"old-port-in-use"), "{found:?}");
    }

    #[test]
    fn two_files_declaring_one_name_is_an_error() {
        let a = container_named("portfolio", 8001, 0, "a.m4marvin.com");
        let mut b = container_named("portfolio", 8002, 1, "b.m4marvin.com");
        b.hostnames = vec!["b.m4marvin.com".into()];
        let found = codes(&validate(&[a, b], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-name"), "{found:?}");
    }

    // ---- tunnel cross-checks ----

    #[test]
    fn a_hostname_with_no_tunnel_route_is_an_error() {
        let app = container_named("portfolio", 8001, 0, "m4marvin.com");
        let mut tunnel = TunnelRoutes::new();
        tunnel.add("something.else.com", 8001);
        let found = codes(&validate(&[app], &tunnel));
        assert!(found.contains(&"no-tunnel-route"), "{found:?}");
    }

    #[test]
    fn a_tunnel_route_to_the_wrong_port_is_an_error() {
        let app = container_named("portfolio", 8001, 0, "m4marvin.com");
        let mut tunnel = TunnelRoutes::new();
        tunnel.add("m4marvin.com", 8009); // wrong front port
        let found = codes(&validate(&[app], &tunnel));
        assert!(found.contains(&"tunnel-route-wrong-port"), "{found:?}");
    }

    #[test]
    fn a_tunnel_hostname_no_app_claims_is_only_a_warning() {
        // Expected during a migration, before the registry file exists.
        let app = container_named("portfolio", 8001, 0, "m4marvin.com");
        let mut tunnel = TunnelRoutes::new();
        tunnel.add("m4marvin.com", 8001);
        tunnel.add("not-yet.m4marvin.com", 8099);

        let problems = validate(&[app], &tunnel);
        let orphan = problems
            .iter()
            .find(|p| p.code == "orphaned-tunnel-route")
            .expect("must report the orphan");
        assert!(
            !orphan.is_error(),
            "a missing registry file is not an outage yet"
        );
    }

    #[test]
    fn tunnel_checks_are_skipped_when_there_are_no_routes() {
        // An empty route set means "not supplied", not "nothing is routed".
        let app = container_named("portfolio", 8001, 0, "m4marvin.com");
        assert!(validate(&[app], &TunnelRoutes::new()).is_empty());
    }

    #[test]
    fn tunnel_lookup_normalises_the_hostname() {
        let mut t = TunnelRoutes::new();
        t.add("M4Marvin.COM.", 8001);
        assert_eq!(t.get("m4marvin.com"), Some(8001));
        assert_eq!(t.get("M4Marvin.com"), Some(8001));
        assert_eq!(t.len(), 1);
    }

    // ---- static apps participate equally ----

    #[test]
    fn a_static_app_is_checked_like_any_other() {
        let mut s = sample_static();
        s.name = "morphotech".into();
        s.front_port = 8012;
        s.slot = 3;
        s.kind = Kind::Static;
        s.image_repo = None;
        s.release = None;
        s.old_release = None;
        s.health_url = None;

        let a = container_named("portfolio", 8001, 0, "m4marvin.com");
        let mut collide = s.clone();
        collide.front_port = 8001;
        let found = codes(&validate(&[a, collide], &TunnelRoutes::new()));
        assert!(found.contains(&"duplicate-listener"), "{found:?}");
    }

    // ---- stability ----

    #[test]
    fn findings_are_sorted_so_output_is_stable() {
        let a = container_named("zeta", 8001, 0, "z.m4marvin.com");
        let b = container_named("alpha", 8001, 0, "a.m4marvin.com");
        let forward = validate(&[a.clone(), b.clone()], &TunnelRoutes::new());
        let reversed = validate(&[b, a], &TunnelRoutes::new());
        assert_eq!(forward, reversed, "order of input must not change output");
        assert_eq!(codes(&forward), codes(&reversed));
    }

    #[test]
    fn counts_separates_errors_from_warnings() {
        let app = container_named("portfolio", 8001, 0, "m4marvin.com");
        let mut tunnel = TunnelRoutes::new();
        tunnel.add("m4marvin.com", 8001);
        tunnel.add("stray.m4marvin.com", 9999);

        let problems = validate(&[app], &tunnel);
        let (errors, warnings) = counts(&problems);
        assert_eq!(errors, 0, "{problems:?}");
        assert_eq!(warnings, 1, "{problems:?}");
    }

    #[test]
    fn per_app_and_cross_app_problems_can_be_combined() {
        use crate::registry::load_dir;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut good = container_named("portfolio", 8001, 0, "m4marvin.com");
        good.compose_svc = "portfolio".into();
        std::fs::write(
            dir.path().join("portfolio.toml"),
            crate::registry::to_toml(&good),
        )
        .unwrap();

        // A stateful app configured to swap: caught per-app.
        let mut bad = container_named("forgejo", 8003, 1, "git.m4marvin.com");
        bad.writes_state = true;
        bad.strategy = crate::registry::Strategy::Swap;
        std::fs::write(
            dir.path().join("forgejo.toml"),
            crate::registry::to_toml(&bad),
        )
        .unwrap();

        // A third app colliding with the second's front port: caught cross-app.
        let mut collide = container_named("mirror", 8003, 2, "git2.m4marvin.com");
        collide.compose_svc = "mirror".into();
        std::fs::write(
            dir.path().join("mirror.toml"),
            crate::registry::to_toml(&collide),
        )
        .unwrap();

        let loaded = load_dir(dir.path()).unwrap();
        let mut problems = loaded.problems.clone();
        for app in &loaded.apps {
            problems.extend(app.problems());
        }
        problems.extend(validate(&loaded.sorted(), &TunnelRoutes::new()));

        let found = codes(&problems);
        assert!(found.contains(&"swap-with-shared-state"), "{found:?}");
        assert!(found.contains(&"duplicate-listener"), "{found:?}");
    }
}
