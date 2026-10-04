//! Port allocation by slot.
//!
//! Back-end ports are derived from a slot rather than found by scanning for a
//! free port. Scanning races: two concurrent deploys can pick the same port, and
//! a deploy that dies mid-flight leaves an orphan holding one that a later deploy
//! then reuses. Deriving makes collision impossible by construction.
//!
//! Each app owns two back ports for life:
//!
//! ```text
//! pair(slot) = (BACK_BASE + 2*slot, BACK_BASE + 2*slot + 1)
//!
//! slot 0  ->  9000, 9001      slot 1  ->  9002, 9003
//! slot 13 ->  9026, 9027
//! ```
//!
//! A deploy uses whichever port of the pair is not live, so the two are blue and
//! green and alternate forever.

use std::ops::RangeInclusive;

/// First port of the reserved back-end range.
pub const BACK_BASE: u16 = 9000;

/// Number of addressable slots. 128 apps x 2 ports = 9000..9256.
pub const MAX_SLOTS: u8 = 128;

/// The whole back-end range, reserved so front ports can be checked against it.
pub const RESERVED_BACK: RangeInclusive<u16> = BACK_BASE..=(BACK_BASE + 2 * MAX_SLOTS as u16 - 1);

/// Why a slot or port is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("slot {slot} is out of range; the maximum is {}", crate::ports::MAX_SLOTS - 1)]
    SlotOutOfRange { slot: u8 },

    #[error("port {port} is not in the pair for slot {slot}, which is {a} or {b}")]
    NotInSlot { slot: u8, port: u16, a: u16, b: u16 },

    #[error("port {port} is reserved for back ends ({}..={})", crate::ports::RESERVED_BACK.start(), crate::ports::RESERVED_BACK.end())]
    Reserved { port: u16 },
}

/// The blue/green pair of back-end ports for a slot.
pub fn pair(slot: u8) -> Result<(u16, u16), PortError> {
    if slot >= MAX_SLOTS {
        return Err(PortError::SlotOutOfRange { slot });
    }
    let base = BACK_BASE + 2 * slot as u16;
    Ok((base, base + 1))
}

/// The port of `slot`'s pair that is not `current`.
///
/// This is how a deploy picks its target: give it the live port, get the other.
pub fn other(slot: u8, current: u16) -> Result<u16, PortError> {
    let (a, b) = pair(slot)?;
    match current {
        p if p == a => Ok(b),
        p if p == b => Ok(a),
        other => Err(PortError::NotInSlot {
            slot,
            port: other,
            a,
            b,
        }),
    }
}

/// True when `port` belongs to any slot's pair.
pub fn is_reserved(port: u16) -> bool {
    RESERVED_BACK.contains(&port)
}

/// True when `port` belongs to `slot`'s pair specifically.
pub fn belongs_to_slot(port: u16, slot: u8) -> bool {
    match pair(slot) {
        Ok((a, b)) => port == a || port == b,
        Err(_) => false,
    }
}

/// Every reserved port, in order. Used by the validator to report all clashes.
pub fn all_reserved() -> impl Iterator<Item = u16> {
    RESERVED_BACK.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_zero_is_the_first_pair() {
        assert_eq!(pair(0).unwrap(), (9000, 9001));
    }

    #[test]
    fn consecutive_slots_never_overlap() {
        let mut seen = std::collections::HashSet::new();
        for slot in 0..MAX_SLOTS {
            let (a, b) = pair(slot).unwrap();
            assert!(seen.insert(a), "slot {slot} reuses port {a}");
            assert!(seen.insert(b), "slot {slot} reuses port {b}");
            assert_eq!(b, a + 1, "a pair must be adjacent");
        }
        assert_eq!(seen.len(), 2 * MAX_SLOTS as usize, "all ports distinct");
    }

    #[test]
    fn pairs_are_contiguous_across_slots() {
        assert_eq!(pair(1).unwrap(), (9002, 9003));
        assert_eq!(pair(13).unwrap(), (9026, 9027));
        // End of the range: slot 127 -> 9254, 9255.
        assert_eq!(pair(MAX_SLOTS - 1).unwrap(), (9254, 9255));
        assert_eq!(*RESERVED_BACK.end(), 9255);
    }

    #[test]
    fn other_returns_the_free_port_of_the_pair() {
        assert_eq!(other(0, 9000).unwrap(), 9001);
        assert_eq!(other(0, 9001).unwrap(), 9000);
        assert_eq!(other(5, 9010).unwrap(), 9011);
    }

    #[test]
    fn other_is_an_involution() {
        for slot in [0u8, 1, 7, 42, MAX_SLOTS - 1] {
            let (a, b) = pair(slot).unwrap();
            assert_eq!(other(slot, other(slot, a).unwrap()).unwrap(), a);
            assert_eq!(other(slot, other(slot, b).unwrap()).unwrap(), b);
        }
    }

    #[test]
    fn a_slot_beyond_the_max_is_rejected() {
        assert_eq!(
            pair(MAX_SLOTS),
            Err(PortError::SlotOutOfRange { slot: MAX_SLOTS })
        );
        assert_eq!(pair(255), Err(PortError::SlotOutOfRange { slot: 255 }));
        assert_eq!(
            other(200, 9000).unwrap_err(),
            PortError::SlotOutOfRange { slot: 200 }
        );
    }

    #[test]
    fn a_port_outside_the_pair_is_rejected() {
        let err = other(0, 8001).unwrap_err();
        assert_eq!(
            err,
            PortError::NotInSlot {
                slot: 0,
                port: 8001,
                a: 9000,
                b: 9001
            }
        );
    }

    #[test]
    fn reserved_detection_matches_the_pairs() {
        assert!(is_reserved(9000));
        assert!(is_reserved(9255));
        assert!(!is_reserved(8999));
        assert!(!is_reserved(9256));

        for port in [9000, 9001, 9002, 9026, 9027, 9255] {
            let owner = (0..MAX_SLOTS).find(|s| belongs_to_slot(port, *s));
            assert!(owner.is_some(), "{port} must belong to a slot");
            assert!(is_reserved(port));
        }
    }

    #[test]
    fn belongs_to_slot_is_false_for_an_invalid_slot() {
        assert!(!belongs_to_slot(9000, MAX_SLOTS));
    }

    #[test]
    fn front_ports_are_outside_the_reserved_range() {
        // The ports already in use on the host must not collide with back ends.
        for front in [8001u16, 8002, 8003, 8010, 8011, 8012] {
            assert!(!is_reserved(front), "front port {front} must be free");
        }
    }

    #[test]
    fn all_reserved_yields_the_whole_range_in_order() {
        let all: Vec<u16> = all_reserved().collect();
        assert_eq!(all.len(), 2 * MAX_SLOTS as usize);
        assert_eq!(all.first(), Some(&9000));
        assert_eq!(all.last(), Some(&9255));
        assert!(all.windows(2).all(|w| w[0] + 1 == w[1]), "contiguous");
    }

    #[test]
    fn the_reserved_range_leaves_headroom_below_u16_max() {
        // A real invariant rather than a tautology: the range must end far enough
        // below u16::MAX that a future range can be added without overflowing.
        let headroom = u16::MAX - *RESERVED_BACK.end();
        assert!(
            headroom > 30_000,
            "only {headroom} ports of headroom below the reserved range"
        );
        let (a, b) = pair(MAX_SLOTS - 1).unwrap();
        assert_eq!(b, *RESERVED_BACK.end());
        assert!(b > a);
        assert_eq!(
            u32::from(b) - u32::from(BACK_BASE) + 1,
            2 * u32::from(MAX_SLOTS)
        );
    }
}
