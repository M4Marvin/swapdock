//! Validation for SSH/SCP destinations.
//!
//! A registry or API caller can supply the deploy/build target. It must never
//! be parsed as an ssh option: `-oProxyCommand=...` is local command execution.
//! The destination is a data value, not an argv option, so it is validated and
//! always placed after `--`.

/// True for a conservative `[user@]host` destination.
///
/// Allows letters, digits, `.`, `-`, `_`, `:` (port or IPv6), `[`/`]` (IPv6)
/// and `%` (IPv6 zone). Rejects an empty value, a leading `-`, whitespace,
/// control characters, extra `@` signs, and anything else that could be an
/// option or shell fragment.
pub fn is_valid_destination(target: &str) -> bool {
    if target.is_empty()
        || target.starts_with('-')
        || target.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    match target.split_once('@') {
        Some((user, host)) => {
            !user.is_empty()
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                && valid_host(host)
        }
        None => valid_host(target),
    }
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '%')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_destinations_are_accepted() {
        for target in [
            "deploy@example.com",
            "example.com",
            "deploy@10.0.0.5",
            "deploy@[::1]",
            "deploy@fe80::1%eth0",
        ] {
            assert!(is_valid_destination(target), "{target}");
        }
    }

    #[test]
    fn ssh_options_and_shell_fragments_are_refused() {
        for target in [
            "-oProxyCommand=sh -c id",
            "deploy@example.com -oProxyCommand=id",
            "deploy@example.com;id",
            "deploy@example.com$(id)",
            "a@b@c",
            "",
            "deploy@",
        ] {
            assert!(!is_valid_destination(target), "{target}");
        }
    }
}
