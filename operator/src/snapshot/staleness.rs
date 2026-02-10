/// Check whether a node's local slot is stale compared to a reference slot.
///
/// Returns `true` when the node is more than `threshold` slots behind the
/// reference, meaning it would be faster to download a fresh snapshot than
/// to let the node catch up naturally.
pub fn is_stale(local_slot: Option<u64>, reference_slot: u64, threshold: u64) -> bool {
    match local_slot {
        None => true,
        Some(local) if reference_slot > local => (reference_slot - local) > threshold,
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_local_slot_is_stale() {
        assert!(is_stale(None, 100_000, 1000));
    }

    #[test]
    fn far_behind_is_stale() {
        assert!(is_stale(Some(90_000), 100_000, 1000));
    }

    #[test]
    fn within_threshold_is_not_stale() {
        assert!(!is_stale(Some(99_500), 100_000, 1000));
    }

    #[test]
    fn at_threshold_is_not_stale() {
        assert!(!is_stale(Some(99_000), 100_000, 1000));
    }

    #[test]
    fn ahead_of_reference_is_not_stale() {
        assert!(!is_stale(Some(101_000), 100_000, 1000));
    }
}
