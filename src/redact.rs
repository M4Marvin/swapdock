//! Secret redaction.
//!
//! Everything this tool does is written to an append-only log. That log will
//! contain `docker login` arguments and environment variables, which is exactly
//! where credentials leak. Redaction is applied inside the single subprocess
//! chokepoint *before* anything is handed to the tracer, so a new call site
//! cannot forget it.
//!
//! Three rules, applied in order of confidence:
//!
//! 1. **Registered literals.** Anything the caller explicitly registers is
//!    replaced everywhere, matched exactly. Labelled with a non-reversible
//!    fingerprint so a log still tells you *which* credential was in use.
//! 2. **Known token shapes.** Prefixes for the services actually in play here
//!    (GitHub, GitLab, Cloudflare, Docker Hub) plus AWS keys and JWTs.
//! 3. **Sensitive names.** Environment variables and flags whose name marks
//!    them as secret. Name matching is segment-aware, so `SSH_KEY` matches but
//!    `KEYBOARD` does not.
//!
//! Deliberate non-goal: no secret characters are preserved. A fingerprint is
//! enough to debug "wrong token" and not enough to leak one.

use std::fmt::Write as _;

/// Replaces text that must never reach a log.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    literals: Vec<LiteralSecret>,
}

/// A secret the caller told us about by value.
///
/// `Debug` is written by hand rather than derived: a derived `Debug` would print
/// the secret value, and `Redactor` is `#[derive(Debug)]`. A log line or a test
/// failure containing `{:?}` of a redactor must never leak a credential.
struct LiteralSecret {
    value: String,
    /// Short non-reversible tag, so log lines stay distinguishable.
    fingerprint: String,
}

impl std::fmt::Debug for LiteralSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LiteralSecret(len={}, fp={})",
            self.value.len(),
            self.fingerprint
        )
    }
}

impl Clone for LiteralSecret {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            fingerprint: self.fingerprint.clone(),
        }
    }
}

/// Token prefixes, longest-first at match time so `github_pat_` wins over `ghp_`.
const TOKEN_PREFIXES: &[(&str, &str)] = &[
    ("github_pat_", "github-fine-grained-pat"),
    ("dckr_pat_", "dockerhub-pat"),
    ("cfut_", "cloudflare-token"),
    ("glpat-", "gitlab-pat"),
    ("ghp_", "github-pat"),
    ("gho_", "github-oauth"),
    ("ghu_", "github-user-token"),
    ("ghs_", "github-server-token"),
    ("ghr_", "github-refresh-token"),
    ("sk-ant-", "anthropic-key"),
];

/// Env var name segments that mark the value as a secret.
const SECRET_NAME_SEGMENTS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "CREDENTIAL",
    "CREDENTIALS",
    "AUTH",
    "PAT",
    "SESSION",
    "KEY",
    "KEYS",
    "APIKEY",
    "DSN",
];

/// Flags whose *following* argument is a secret.
///
/// `-p` is intentionally absent: in `docker run -p 8001:80` it is a port, not a
/// password. Masking it would be both wrong and confusing.
const SECRET_FLAGS: &[&str] = &[
    "--password",
    "--token",
    "--secret",
    "--api-key",
    "--auth-token",
    "--client-secret",
];

impl Redactor {
    /// An empty redactor, which still applies rules 2 and 3.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret by value so it is masked wherever it appears.
    ///
    /// Empty and very short values are ignored: masking a one-character string
    /// would redact half the log for no benefit.
    pub fn register(&mut self, secret: &str) {
        if secret.len() < 8 {
            return;
        }
        if self.literals.iter().any(|l| l.value == secret) {
            return;
        }
        self.literals.push(LiteralSecret {
            value: secret.to_string(),
            fingerprint: fingerprint(secret),
        });
    }

    /// Registers every value of an environment variable, if it looks secret.
    pub fn register_env(&mut self, key: &str, value: &str) {
        if name_is_secret(key) {
            self.register(value);
        }
    }

    /// Masks a command line, including the value that follows a secret flag.
    pub fn argv(&self, argv: &[String]) -> Vec<String> {
        let mut out = Vec::with_capacity(argv.len());
        let mut mask_next = false;

        for arg in argv {
            if mask_next {
                out.push(format!("[REDACTED:{}]", fingerprint(arg)));
                mask_next = false;
                continue;
            }
            if SECRET_FLAGS.contains(&arg.as_str()) {
                mask_next = true;
                out.push(arg.clone());
                continue;
            }
            out.push(self.text(arg));
        }
        out
    }

    /// Masks environment variables, by name and by value.
    pub fn env(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        env.iter()
            .map(|(k, v)| {
                if name_is_secret(k) {
                    (k.clone(), format!("[REDACTED:{}]", fingerprint(v)))
                } else {
                    (k.clone(), self.text(v))
                }
            })
            .collect()
    }

    /// Masks known token shapes and any registered literal inside free text.
    ///
    /// This is applied to captured stdout and stderr too, because a tool can
    /// print the environment it was given, and so can a container.
    pub fn text(&self, haystack: &str) -> String {
        let mut out = self.mask_literals(haystack);
        out = mask_token_prefixes(&out);
        out = mask_jwt(&out);
        out = mask_aws(&out);
        mask_bearer(&out)
    }

    fn mask_literals(&self, haystack: &str) -> String {
        let mut out = haystack.to_string();
        for lit in &self.literals {
            if lit.value.is_empty() {
                continue;
            }
            out = out.replace(
                lit.value.as_str(),
                &format!("[REDACTED:literal-{}]", lit.fingerprint),
            );
        }
        out
    }
}

/// True when an env var or flag name marks the value as a secret.
///
/// Splits on `_` and `-` and compares whole segments, so `GITHUB_TOKEN` and
/// `SSH-KEY` match while `KEYBOARD` and `MONKEY` do not.
pub fn name_is_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper
        .split(['_', '-', '.'])
        .any(|seg| SECRET_NAME_SEGMENTS.contains(&seg))
}

/// Short, stable, non-reversible tag for a value.
///
/// FNV-1a over the first bytes. It is not a security primitive and does not need
/// to be: its only job is to let a human tell two credentials apart in a log.
fn fingerprint(value: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for byte in value.as_bytes().iter().take(16) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:08x}")
}

/// Replaces `prefix<token>` with `prefix[REDACTED:<label>]`.
fn mask_token_prefixes(text: &str) -> String {
    let mut sorted: Vec<_> = TOKEN_PREFIXES.iter().collect();
    sorted.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));

    let mut out = text.to_string();
    for (prefix, label) in sorted {
        out = replace_tokens_with_prefix(&out, prefix, label);
    }
    out
}

/// Consumes everything up to a delimiter after `prefix`.
///
/// Delimiters are the characters that terminate a credential in a URL, a header
/// or a shell argument. Without this, one token in a long log line would mask
/// the entire rest of the line.
fn replace_tokens_with_prefix(text: &str, prefix: &str, label: &str) -> String {
    const DELIMITERS: &[char] = &[
        ' ', '"', '\'', '\n', '\r', '\t', ',', ';', ')', '(', ']', '[', '}', '{', '=', '&', '?',
        '#', '\\', '`', '|', '<', '>', ':',
    ];

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(pos) = rest.find(prefix) {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];

        let token: String = rest[prefix.len()..]
            .chars()
            .take_while(|c| !DELIMITERS.contains(c))
            .collect();

        if token.is_empty() {
            // The prefix with nothing after it is not a credential.
            out.push_str(prefix);
            rest = &rest[prefix.len()..];
            continue;
        }

        let _ = write!(out, "{prefix}[REDACTED:{label}]");
        rest = &rest[prefix.len() + token.len()..];
    }
    out.push_str(rest);
    out
}

/// Masks a JSON Web Token: three base64url segments, each at least 10 chars.
fn mask_jwt(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i..].starts_with(b"eyJ") {
            let mut end = i;
            let mut dots = 0;
            while end < bytes.len() && end - i < 4096 {
                let b = bytes[end];
                let is_body = b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
                if b == b'.' {
                    dots += 1;
                } else if !is_body {
                    break;
                }
                end += 1;
            }
            let candidate = &text[i..end];
            let segments: Vec<&str> = candidate.trim_end_matches('.').split('.').collect();
            let looks_like_jwt =
                dots == 2 && segments.len() == 3 && segments.iter().all(|s| s.len() >= 10);

            if looks_like_jwt {
                let _ = write!(out, "[REDACTED:jwt]");
                i = end;
                continue;
            }
        }
        // Advance by one full char so a multi-byte sequence is never split.
        let ch = text[i..].chars().next().expect("in bounds");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Masks an AWS access key id: `AKIA` or `ASIA` plus 16 uppercase alphanumerics.
fn mask_aws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let is_start = text[i..].starts_with("AKIA") || text[i..].starts_with("ASIA");
        if is_start {
            let tail = &text[i + 4..];
            let key: String = tail
                .chars()
                .take(16)
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .collect();
            if key.len() == 16 {
                let _ = write!(out, "[REDACTED:aws-access-key]");
                i += 4 + 16;
                continue;
            }
        }
        let ch = text[i..].chars().next().expect("in bounds");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Masks the value of an `Authorization: Bearer …` or `Bearer …` header.
fn mask_bearer(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(pos) = rest.find("earer ") {
        let value_start = pos + "earer ".len();
        out.push_str(&rest[..value_start]);

        let token: String = rest[value_start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"' && *c != ',')
            .collect();

        // Do not re-mask a placeholder written by an earlier rule, or the
        // label for a JWT would be replaced by the less specific bearer label.
        if token.is_empty() || token.starts_with("[REDACTED") {
            out.push_str(&rest[value_start..]);
            return out;
        }

        let _ = write!(out, "[REDACTED:bearer]");
        rest = &rest[value_start + token.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn masks_github_tokens_by_prefix() {
        let r = Redactor::new();
        let masked = r.text("login with ghp_16CharsOfNonsense0123456789abcdef ok");
        assert!(masked.contains("[REDACTED:github-pat]"), "{masked}");
        assert!(!masked.contains("16CharsOfNonsense"), "{masked}");
        assert!(masked.contains("login with "), "keeps context: {masked}");
    }

    #[test]
    fn masks_the_fine_grained_pat_before_the_short_prefix() {
        let r = Redactor::new();
        let masked = r.text("github_pat_11ABCDEFG0abcdefghijklmnop");
        assert!(
            masked.contains("[REDACTED:github-fine-grained-pat]"),
            "{masked}"
        );
    }

    #[test]
    fn masks_cloudflare_and_gitlab_and_dockerhub_tokens() {
        let r = Redactor::new();
        for (token, label) in [
            ("cfut_abc123DEF456ghi789", "cloudflare-token"),
            ("glpat-ABCdef123456", "gitlab-pat"),
            ("dckr_pat_ABCdef123456", "dockerhub-pat"),
        ] {
            let masked = r.text(token);
            assert!(masked.contains(&format!("[REDACTED:{label}]")), "{masked}");
            assert!(!masked.contains("abc123DEF456"), "{masked}");
        }
    }

    #[test]
    fn masking_stops_at_the_delimiter() {
        let r = Redactor::new();
        let masked = r.text("url=https://x/ghp_abcdefghijklmnop1234&next=1");
        assert_eq!(
            masked, "url=https://x/ghp_[REDACTED:github-pat]&next=1",
            "must not swallow the rest of the line"
        );
    }

    #[test]
    fn masks_a_jwt() {
        let r = Redactor::new();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let masked = r.text(&format!("Authorization: Bearer {jwt}"));
        assert!(masked.contains("[REDACTED:jwt]"), "{masked}");
        assert!(
            !masked.contains("dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "{masked}"
        );

        // The more specific rule must win and must not be overwritten.
        assert!(!masked.contains("[REDACTED:bearer]"), "{masked}");
    }

    #[test]
    fn masks_an_aws_access_key() {
        let r = Redactor::new();
        let masked = r.text("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE rest");
        assert!(masked.contains("[REDACTED:aws-access-key]"), "{masked}");
        assert!(!masked.contains("IOSFODNN7EXAMPLE"), "{masked}");
    }

    #[test]
    fn masks_a_bearer_header_with_an_opaque_value() {
        let r = Redactor::new();
        let masked = r.text("Authorization: Bearer abc.def.ghi and more");
        assert!(masked.contains("[REDACTED:bearer]"), "{masked}");
        assert!(masked.ends_with(" and more"), "keeps the tail: {masked}");
    }

    #[test]
    fn masks_a_registered_literal_and_labels_it_with_a_fingerprint() {
        let mut r = Redactor::new();
        r.register("hunter2-super-secret-value");
        let masked = r.text("the value is hunter2-super-secret-value here");
        assert!(masked.contains("[REDACTED:literal-"), "{masked}");
        assert!(!masked.contains("hunter2"), "{masked}");
    }

    #[test]
    fn the_same_literal_always_gets_the_same_fingerprint() {
        let mut r = Redactor::new();
        r.register("hunter2-super-secret-value");
        let a = r.text("x hunter2-super-secret-value y");
        let b = r.text("z hunter2-super-secret-value w");
        assert!(a.starts_with("x [REDACTED:literal-"), "{a}");
        assert!(a.ends_with("] y"), "{a}");
        assert_eq!(
            a.split("literal-").nth(1).map(|s| &s[..8]),
            b.split("literal-").nth(1).map(|s| &s[..8]),
            "fingerprints must match across calls"
        );
        assert_eq!(
            r.text("hunter2-super-secret-value"),
            r.text("hunter2-super-secret-value")
        );
    }

    #[test]
    fn different_literals_get_different_fingerprints() {
        let mut r = Redactor::new();
        r.register("aaaaaaaa-secret");
        r.register("bbbbbbbb-secret");
        let a = r.text("aaaaaaaa-secret");
        let b = r.text("bbbbbbbb-secret");
        assert_ne!(a, b);
    }

    #[test]
    fn ignores_literals_too_short_to_be_credentials() {
        let mut r = Redactor::new();
        r.register("abc");
        assert_eq!(r.text("abc def"), "abc def", "short values are not masked");
    }

    #[test]
    fn ignores_a_duplicate_registration() {
        let mut r = Redactor::new();
        r.register("hunter2-super-secret-value");
        let once = r.text("hunter2-super-secret-value");
        r.register("hunter2-super-secret-value");
        assert_eq!(r.text("hunter2-super-secret-value"), once);
    }

    #[test]
    fn env_names_are_matched_by_segment_not_substring() {
        for secret in [
            "GITHUB_TOKEN",
            "REGISTRY_PASSWORD",
            "DB_PASSWD",
            "SSH_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "APIKEY",
            "MY.CREDENTIALS",
        ] {
            assert!(name_is_secret(secret), "{secret} must be secret");
        }
        for safe in [
            "KEYBOARD",
            "MONKEY",
            "PORTFOLIO_PORT",
            "PATH",
            "HOME",
            "AUTHORS",
        ] {
            assert!(!name_is_secret(safe), "{safe} must not be secret");
        }
    }

    #[test]
    fn masks_a_secret_env_value_by_name() {
        let r = Redactor::new();
        let env = vec![
            (
                "GITHUB_TOKEN".to_string(),
                "ghp_realsecretvalue".to_string(),
            ),
            ("PORTFOLIO_PORT".to_string(), "9004".to_string()),
        ];
        let masked = r.env(&env);
        assert!(masked[0].1.starts_with("[REDACTED:"), "{:?}", masked);
        assert!(!masked[0].1.contains("realsecret"), "{:?}", masked);
        assert_eq!(masked[1].1, "9004", "safe values pass through: {masked:?}");
    }

    #[test]
    fn masks_the_argument_after_a_secret_flag() {
        let r = Redactor::new();
        let masked = r.argv(&argv_of(&[
            "docker",
            "login",
            "--password",
            "s3cret-value",
            "ghcr.io",
        ]));
        assert_eq!(
            masked[3],
            "[REDACTED:s3cret-value]".replace("s3cret-value", &fingerprint("s3cret-value"))
        );
        assert!(!masked.join(" ").contains("s3cret-value"));
    }

    #[test]
    fn does_not_mask_a_port_passed_to_dash_p() {
        let r = Redactor::new();
        let masked = r.argv(&argv_of(&[
            "docker",
            "run",
            "-p",
            "127.0.0.1:9004:80",
            "img",
        ]));
        assert_eq!(
            masked[3], "127.0.0.1:9004:80",
            "-p is a port, not a password"
        );
    }

    #[test]
    fn leaves_a_trailing_secret_flag_without_a_value() {
        let r = Redactor::new();
        let masked = r.argv(&argv_of(&["docker", "login", "--password"]));
        assert_eq!(masked, vec!["docker", "login", "--password"]);
    }

    #[test]
    fn handles_multi_byte_text_without_panicking() {
        let r = Redactor::new();
        let text = "héllo ghp_abcdefghijklmnop1234 wörld — ünïcode ✅ 日本語";
        let masked = r.text(text);
        assert!(masked.contains("[REDACTED:github-pat]"), "{masked}");
        assert!(
            masked.contains("日本語"),
            "must not eat other scripts: {masked}"
        );
    }

    #[test]
    fn is_idempotent() {
        let r = Redactor::new();
        let once = r.text("ghp_abcdefghijklmnop1234");
        let twice = r.text(&once);
        assert_eq!(once, twice, "redacting twice must not change more");
    }

    #[test]
    fn debug_output_never_contains_a_registered_secret() {
        let mut r = Redactor::new();
        r.register("ghp_topsecretvalue42");
        let rendered = format!("{r:?}");
        assert!(
            !rendered.contains("ghp_topsecretvalue42"),
            "Debug leaked a credential: {rendered}"
        );
        assert!(
            rendered.contains("fp="),
            "should still identify the entry: {rendered}"
        );
    }

    #[test]
    fn leaves_ordinary_deploy_output_untouched() {
        let r = Redactor::new();
        let line = "portfolio-green  Started  0.4s  127.0.0.1:9004->80/tcp  healthy";
        assert_eq!(r.text(line), line);
    }
}
