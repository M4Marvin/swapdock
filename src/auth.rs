//! API authentication.
//!
//! swapdock's HTTP API can deploy, roll back and run host commands. It must
//! never be reachable without a credential. This module is intentionally small:
//! one bearer token, compared in constant time, plus a short-lived allowance for
//! local development. The console and CLI both use the same token today; OIDC
//! and per-user sessions are a later layer, not a replacement for this check.
//!
//! The token lives in `SWAPDOCK_API_TOKEN`, not in argv, so it does not appear in
//! `ps`. `--allow-unauthenticated` is an explicit development escape hatch and
//! logs a warning at startup.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, Uri, header};

/// Refuse to start with a token shorter than this. A one-character token is a
/// false sense of security; 16 bytes is still short, but it is not a guess.
const MIN_TOKEN_LEN: usize = 16;

/// How long a console session stays valid. Restarting the API clears all of
/// them: the store is deliberately in memory, not a second credential database.
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);

const SESSION_COOKIE: &str = "swapdock_session";

/// Login throttle window and cap. This is a small in-memory guard, not a WAF:
/// a restart clears it, and the key is a best-effort client address from a
/// trusted proxy header.
const LOGIN_WINDOW: Duration = Duration::from_secs(5 * 60);
const LOGIN_MAX_FAILURES: u32 = 5;
const LOGIN_BLOCK: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
struct Session {
    csrf: String,
    expires: Instant,
}

type Sessions = Arc<Mutex<HashMap<String, Session>>>;

#[derive(Debug, Clone)]
struct LoginWindow {
    failures: u32,
    first: Instant,
    blocked_until: Option<Instant>,
}

type LoginFailures = Arc<Mutex<HashMap<String, LoginWindow>>>;

/// Authentication and browser-origin policy for the API.
#[derive(Clone, Default)]
pub struct AuthConfig {
    token: Option<String>,
    allow_unauthenticated: bool,
    allowed_origins: Vec<String>,
    /// Optional `Host` allowlist (`SWAPDOCK_ALLOWED_HOSTS`), without ports.
    allowed_hosts: Vec<String>,
    sessions: Sessions,
    login_failures: LoginFailures,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("allow_unauthenticated", &self.allow_unauthenticated)
            .field("allowed_origins", &self.allowed_origins)
            .field(
                "sessions",
                &self.sessions.lock().map(|s| s.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl AuthConfig {
    /// Reads the token and allowed origins from the environment.
    ///
    /// Fails closed: with no token, the server does not start unless the caller
    /// explicitly opted into an unauthenticated local run.
    pub fn try_from_env(allow_unauthenticated: bool) -> Result<Self, String> {
        let token = std::env::var("SWAPDOCK_API_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(token) = &token
            && token.len() < MIN_TOKEN_LEN
        {
            return Err(format!(
                "SWAPDOCK_API_TOKEN must be at least {MIN_TOKEN_LEN} characters; \
                 generate one with `openssl rand -hex 32`"
            ));
        }
        if let Some(token) = &token
            && !token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        {
            return Err(
                "SWAPDOCK_API_TOKEN must be URL-safe (letters, digits, -._~) so it can be \
                 used in the EventSource query parameter; generate one with `openssl rand -hex 32`"
                    .to_string(),
            );
        }
        if token.is_none() && !allow_unauthenticated {
            return Err(
                "SWAPDOCK_API_TOKEN is not set; set it, or pass --allow-unauthenticated \
                 for a loopback-only development server"
                    .to_string(),
            );
        }
        let allowed_origins = std::env::var("SWAPDOCK_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect();
        let allowed_hosts = std::env::var("SWAPDOCK_ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
            .collect();
        Ok(Self {
            token,
            allow_unauthenticated,
            allowed_origins,
            allowed_hosts,
            sessions: Sessions::default(),
            login_failures: LoginFailures::default(),
        })
    }

    /// A deterministic config for unit tests and local router tests.
    #[cfg(test)]
    pub fn for_tests(token: &str) -> Self {
        Self {
            token: Some(token.to_string()),
            allow_unauthenticated: false,
            allowed_origins: vec![],
            allowed_hosts: vec![],
            sessions: Sessions::default(),
            login_failures: LoginFailures::default(),
        }
    }

    /// A local-only config used by existing API tests that are not testing auth.
    #[cfg(test)]
    pub fn unauthenticated_for_tests() -> Self {
        Self {
            token: None,
            allow_unauthenticated: true,
            allowed_origins: vec![],
            allowed_hosts: vec![],
            sessions: Sessions::default(),
            login_failures: LoginFailures::default(),
        }
    }

    /// True when requests must carry a token.
    pub fn is_required(&self) -> bool {
        !self.allow_unauthenticated
    }

    /// True when an explicit development bypass is active.
    pub fn allows_unauthenticated(&self) -> bool {
        self.allow_unauthenticated
    }

    /// CORS origins to allow. Empty means no CORS layer, which is correct for a
    /// same-origin console.
    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    /// True when `--allow-unauthenticated` is used on a loopback literal.
    ///
    /// The bypass is a development convenience, not a way to expose an
    /// unauthenticated control plane on a LAN or public interface.
    pub fn loopback_bind(bind: &str) -> bool {
        if let Ok(addr) = bind.parse::<std::net::SocketAddr>() {
            return addr.ip().is_loopback();
        }
        let host = bind
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(bind)
            .trim_matches(['[', ']']);
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    }

    /// Host/Origin policy. Returns false for a request that would be a
    /// cross-site form post from an origin we did not allow.
    ///
    /// `SWAPDOCK_ALLOWED_HOSTS` is the DNS-rebinding guard: when set, the Host
    /// header must name one of those hosts. Origin is checked against the same
    /// host (same-origin) or the explicit `SWAPDOCK_ALLOWED_ORIGINS` list.
    pub fn host_origin_ok(&self, headers: &HeaderMap) -> bool {
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_end_matches('.').to_ascii_lowercase());
        if !self.allowed_hosts.is_empty() {
            let Some(host) = host.as_deref() else {
                return false;
            };
            let host_no_port = strip_port(host);
            if !self
                .allowed_hosts
                .iter()
                .any(|allowed| allowed == host_no_port || allowed == host)
            {
                return false;
            }
        }
        let Some(origin) = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
        else {
            return true;
        };
        if origin == "null" || origin.trim().is_empty() {
            return false;
        }
        let Some(origin_host) = origin_host(origin) else {
            return false;
        };
        if self.allowed_origins.iter().any(|allowed| allowed == origin) {
            return true;
        }
        host.as_deref()
            .is_some_and(|host| strip_port(host) == origin_host)
    }

    /// True when this client may attempt a login now.
    pub fn login_allowed(&self, key: &str) -> bool {
        let now = Instant::now();
        let Ok(mut failures) = self.login_failures.lock() else {
            return true;
        };
        failures.retain(|_, window| {
            window.blocked_until.is_some_and(|until| until > now)
                || now.duration_since(window.first) < LOGIN_WINDOW
        });
        match failures.get(key) {
            Some(window) => window.blocked_until.is_none_or(|until| until <= now),
            None => true,
        }
    }

    /// Records one failed login and blocks the key after repeated failures.
    pub fn login_failed(&self, key: &str) {
        let now = Instant::now();
        let Ok(mut failures) = self.login_failures.lock() else {
            return;
        };
        let window = failures.entry(key.to_string()).or_insert(LoginWindow {
            failures: 0,
            first: now,
            blocked_until: None,
        });
        if now.duration_since(window.first) >= LOGIN_WINDOW {
            window.failures = 0;
            window.first = now;
            window.blocked_until = None;
        }
        window.failures += 1;
        if window.failures >= LOGIN_MAX_FAILURES {
            window.blocked_until = Some(now + LOGIN_BLOCK);
        }
    }

    /// A successful login clears the failure window for this key.
    pub fn login_succeeded(&self, key: &str) {
        if let Ok(mut failures) = self.login_failures.lock() {
            failures.remove(key);
        }
    }

    /// True when a session-backed console is available.
    pub fn sessions_enabled(&self) -> bool {
        self.token.is_some() && !self.allow_unauthenticated
    }

    /// Verifies the shared credential and starts a short-lived console session.
    ///
    /// This is deliberately the same secret as the CLI bearer token: swapdock
    /// has one operator credential in v1, and the session exists to keep that
    /// credential out of JavaScript and to add CSRF protection, not to model
    /// multiple users. Returns `(session_id, csrf_token)`.
    pub fn login(&self, password: &str) -> Option<(String, String)> {
        if !self.sessions_enabled() {
            return None;
        }
        let expected = self.token.as_deref()?;
        if !constant_time_eq(password.as_bytes(), expected.as_bytes()) {
            return None;
        }
        let id = random_hex()?;
        let csrf = random_hex()?;
        let now = Instant::now();
        let mut sessions = self.sessions.lock().ok()?;
        sessions.retain(|_, session| session.expires > now);
        sessions.insert(
            id.clone(),
            Session {
                csrf: csrf.clone(),
                expires: now + SESSION_TTL,
            },
        );
        Some((id, csrf))
    }

    /// True when the request carries a syntactically bearer header.
    pub fn has_bearer(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
    }

    /// True only when the bearer value actually matches the operator token.
    /// A syntactically valid but wrong header must not be a CSRF bypass on a
    /// cookie-authenticated request.
    pub fn bearer_valid(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = self.token.as_deref() else {
            return false;
        };
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .is_some_and(|(scheme, value)| {
                scheme.eq_ignore_ascii_case("bearer")
                    && constant_time_eq(value.trim().as_bytes(), expected.as_bytes())
            })
    }

    /// The CSRF token for the request's session, if the cookie is valid.
    pub fn session_csrf(&self, headers: &HeaderMap) -> Option<String> {
        let id = cookie_session(headers)?;
        let now = Instant::now();
        let mut sessions = self.sessions.lock().ok()?;
        match sessions.get(&id) {
            Some(session) if session.expires > now => Some(session.csrf.clone()),
            Some(_) => {
                sessions.remove(&id);
                None
            }
            None => None,
        }
    }

    /// True when a cookie session supplied the right CSRF header.
    pub fn csrf_valid(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = self.session_csrf(headers) else {
            return false;
        };
        headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| constant_time_eq(value.as_bytes(), expected.as_bytes()))
    }

    /// Ends the session named by the cookie, if any.
    pub fn logout(&self, headers: &HeaderMap) {
        let Some(id) = cookie_session(headers) else {
            return;
        };
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&id);
        }
    }

    /// A `Set-Cookie` value for a new session.
    pub fn session_cookie(id: &str, secure: bool) -> String {
        let secure = if secure { "; Secure" } else { "" };
        format!(
            "{SESSION_COOKIE}={id}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{secure}",
            SESSION_TTL.as_secs()
        )
    }

    /// A `Set-Cookie` value that expires the session cookie.
    pub fn clear_cookie(secure: bool) -> String {
        let secure = if secure { "; Secure" } else { "" };
        format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{secure}")
    }

    /// Checks a bearer header or a `?token=` query value.
    ///
    /// The query form exists for `EventSource`, which cannot set an
    /// `Authorization` header. It is only as safe as the transport; TLS and
    /// log redaction are required before using the console over a network.
    pub fn authorized(&self, headers: &HeaderMap, uri: &Uri) -> bool {
        if self.allow_unauthenticated {
            return true;
        }
        let Some(expected) = self.token.as_deref() else {
            return false;
        };
        if let Some(value) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            && let Some((scheme, token)) = value.split_once(' ')
            && scheme.eq_ignore_ascii_case("bearer")
            && constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
        {
            return true;
        }
        // A cookie session is valid for every route; the middleware separately
        // requires CSRF for state-changing calls.
        if self.session_csrf(headers).is_some() {
            return true;
        }
        // The query form exists only for EventSource. Accepting it on every
        // route would spread the token into access logs and browser history
        // for no reason; a normal API call must send the Authorization header.
        if !uri.path().ends_with("/events") {
            return false;
        }
        let Some(query) = uri.query() else {
            return false;
        };
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key == "token" && constant_time_eq(value.as_bytes(), expected.as_bytes()) {
                return true;
            }
        }
        false
    }
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[')
        && let Some(end) = host.find(']')
    {
        return &host[..=end];
    }
    host.rsplit_once(':').map(|(host, _)| host).unwrap_or(host)
}

fn origin_host(origin: &str) -> Option<String> {
    let rest = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;
    let host = rest
        .split('/')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    Some(strip_port(&host).to_string())
}

fn cookie_session(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| part.trim().strip_prefix("swapdock_session="))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// A URL-safe random session/CSRF value from the kernel CSPRNG.
fn random_hex() -> Option<String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    let mut out = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Some(out)
}

/// Length-independent comparison. The length check is intentionally early (a
/// length leak does not reveal the secret), and the byte loop accumulates XOR
/// differences so a mismatch cannot return before the end.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Uri;

    fn uri(query: &str) -> Uri {
        format!("http://localhost/api/apps{query}").parse().unwrap()
    }

    fn events_uri(query: &str) -> Uri {
        format!("http://localhost/api/events{query}")
            .parse()
            .unwrap()
    }

    #[test]
    fn accepts_bearer_and_query_tokens() {
        let config = AuthConfig::for_tests("0123456789abcdef");
        let mut headers = HeaderMap::new();
        assert!(!config.authorized(&headers, &uri("")));
        headers.insert(
            header::AUTHORIZATION,
            "Bearer 0123456789abcdef".parse().unwrap(),
        );
        assert!(config.authorized(&headers, &uri("")));
        headers.insert(
            header::AUTHORIZATION,
            "bearer 0123456789abcdef".parse().unwrap(),
        );
        assert!(config.authorized(&headers, &uri("")));
        assert!(config.authorized(
            &HeaderMap::new(),
            &events_uri("?run=r1&token=0123456789abcdef")
        ));
        assert!(!config.authorized(&HeaderMap::new(), &events_uri("?token=wrong")));
        assert!(!config.authorized(&HeaderMap::new(), &events_uri("?other=0123456789abcdef")));
        // The query token is SSE-only; it is not a general API credential.
        assert!(!config.authorized(&HeaderMap::new(), &uri("?token=0123456789abcdef")));
    }

    #[test]
    fn a_login_session_authorizes_and_requires_csrf() {
        let config = AuthConfig::for_tests("0123456789abcdef");
        assert!(config.login("wrong-password").is_none());
        let (id, csrf) = config.login("0123456789abcdef").unwrap();
        assert_eq!(id.len(), 64);
        assert_eq!(csrf.len(), 64);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("other=1; swapdock_session={id}").parse().unwrap(),
        );
        assert!(config.authorized(&headers, &uri("")));
        assert!(!config.csrf_valid(&headers));
        headers.insert("x-csrf-token", csrf.parse().unwrap());
        assert!(config.csrf_valid(&headers));

        let cookie = AuthConfig::session_cookie(&id, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Secure"));
        assert!(AuthConfig::clear_cookie(false).contains("Max-Age=0"));

        config.logout(&headers);
        assert!(!config.authorized(&headers, &uri("")));
    }

    #[test]
    fn host_and_origin_policy_rejects_cross_site_and_unknown_hosts() {
        let mut config = AuthConfig::for_tests("0123456789abcdef");
        config.allowed_hosts = vec!["console.example.com".into()];
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "console.example.com:443".parse().unwrap());
        assert!(config.host_origin_ok(&headers));

        headers.insert(
            header::ORIGIN,
            "https://console.example.com".parse().unwrap(),
        );
        assert!(config.host_origin_ok(&headers));

        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!config.host_origin_ok(&headers));

        headers.insert(header::ORIGIN, "null".parse().unwrap());
        assert!(!config.host_origin_ok(&headers));

        headers.remove(header::ORIGIN);
        headers.insert(header::HOST, "evil.example".parse().unwrap());
        assert!(!config.host_origin_ok(&headers));

        // An explicit allowed origin can name a split console host.
        let mut split = AuthConfig::for_tests("0123456789abcdef");
        split.allowed_origins = vec!["https://console.example.com".into()];
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "api.example.com".parse().unwrap());
        headers.insert(
            header::ORIGIN,
            "https://console.example.com".parse().unwrap(),
        );
        assert!(split.host_origin_ok(&headers));
    }

    #[test]
    fn repeated_login_failures_are_throttled_and_success_clears_them() {
        let config = AuthConfig::for_tests("0123456789abcdef");
        assert!(config.login_allowed("client"));
        for _ in 0..5 {
            config.login_failed("client");
        }
        assert!(!config.login_allowed("client"));
        assert!(config.login_allowed("other-client"));
        config.login_succeeded("client");
        assert!(config.login_allowed("client"));
    }

    #[test]
    fn development_bypass_requires_a_loopback_bind() {
        assert!(AuthConfig::loopback_bind("127.0.0.1:8088"));
        assert!(AuthConfig::loopback_bind("[::1]:8088"));
        assert!(AuthConfig::loopback_bind("::1:8088"));
        assert!(!AuthConfig::loopback_bind("0.0.0.0:8088"));
        assert!(!AuthConfig::loopback_bind("[::]:8088"));
        assert!(!AuthConfig::loopback_bind("100.80.96.4:8088"));
        assert!(!AuthConfig::loopback_bind("localhost:8088"));
    }

    #[test]
    fn unauthenticated_mode_is_explicit() {
        let config = AuthConfig::unauthenticated_for_tests();
        assert!(!config.is_required());
        assert!(config.authorized(&HeaderMap::new(), &uri("")));
        assert_eq!(config.allowed_origins(), &[] as &[String]);
    }

    #[test]
    fn debug_never_prints_the_token() {
        let config = AuthConfig::for_tests("0123456789abcdef");
        let debug = format!("{config:?}");
        assert!(debug.contains("REDACTED"), "{debug}");
        assert!(!debug.contains("0123456789abcdef"), "{debug}");
    }

    #[test]
    fn constant_time_compare_is_exact() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"same", b"same-longer"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
