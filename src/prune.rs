//! Pruning old release images.
//!
//! The deploy loop deliberately keeps the image for `release` and
//! `old_release`, plus the timestamped `rollback-pre-*` tags. Everything older
//! is dead weight; a busy app can accumulate hundreds of 400 MB images.
//!
//! Selection is a pure function so a mistaken rule fails a test instead of
//! deleting the image a rollback needs. `latest` and any tag that does not look
//! like a release are always kept: the tool only removes what it can prove it
//! created.

use std::collections::BTreeSet;

use crate::registry;

/// What must survive a prune.
#[derive(Debug, Default, Clone)]
pub struct Keep {
    pub current: Option<String>,
    pub old: Option<String>,
}

/// Tags that may be removed, given the local tags and the rollback budget.
///
/// Keeps:
/// * `latest` — a moving tag humans and compose defaults may use;
/// * the recorded `release` and `old_release`;
/// * the newest `keep_rollbacks` tags beginning `rollback-pre-`;
/// * any tag that is not a hex release (the tool did not create it).
pub fn select_removals(tags: &[String], keep: &Keep, keep_rollbacks: usize) -> Vec<String> {
    let mut rollbacks: Vec<&String> = tags
        .iter()
        .filter(|t| t.starts_with("rollback-pre-"))
        .collect();
    rollbacks.sort();
    rollbacks.reverse();
    let keep_rollbacks_set: BTreeSet<&String> =
        rollbacks.into_iter().take(keep_rollbacks).collect();

    let mut removals = Vec::new();
    for tag in tags {
        if tag == "latest"
            || keep.current.as_deref() == Some(tag.as_str())
            || keep.old.as_deref() == Some(tag.as_str())
            || keep_rollbacks_set.contains(tag)
        {
            continue;
        }
        if tag.starts_with("rollback-pre-") || registry::is_valid_release(tag) {
            removals.push(tag.clone());
        }
    }
    removals.sort();
    removals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keeps_current_old_latest_and_the_newest_rollback() {
        let all = tags(&[
            "latest",
            "69ff7b5",
            "b04926f",
            "98accdd",
            "rollback-pre-20260923T221150Z",
            "rollback-pre-20260919T113341Z",
            "dev",
        ]);
        let keep = Keep {
            current: Some("69ff7b5".into()),
            old: Some("b04926f".into()),
        };
        assert_eq!(
            select_removals(&all, &keep, 1),
            vec![
                "98accdd".to_string(),
                "rollback-pre-20260919T113341Z".to_string()
            ]
        );
    }

    #[test]
    fn unknown_tags_are_never_removed() {
        let all = tags(&["latest", "dev", "manual-backup", "69ff7b5"]);
        let keep = Keep {
            current: Some("69ff7b5".into()),
            old: None,
        };
        assert!(select_removals(&all, &keep, 3).is_empty());
    }

    #[test]
    fn a_rollback_budget_of_zero_removes_all_rollback_tags() {
        let all = tags(&[
            "rollback-pre-20260923T221150Z",
            "rollback-pre-20260919T113341Z",
        ]);
        assert_eq!(
            select_removals(&all, &Keep::default(), 0),
            vec![
                "rollback-pre-20260919T113341Z".to_string(),
                "rollback-pre-20260923T221150Z".to_string()
            ]
        );
    }
}
