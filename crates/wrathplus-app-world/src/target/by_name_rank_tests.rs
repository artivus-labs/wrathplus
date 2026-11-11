//! The `Rank::beats` tests, split out of `by_name.rs`.

use super::*;

fn exact(dist2: f32) -> Rank {
    Rank {
        exact: true,
        prefix: 3,
        dist2,
    }
}
fn prefix(prefix: usize, dist2: f32) -> Rank {
    Rank {
        exact: false,
        prefix,
        dist2,
    }
}

#[test]
fn exact_beats_any_prefix_and_nearest_breaks_the_tie() {
    // The deviation: exact ranks first, whatever the reference's walk order.
    assert!(exact(100.0).beats(prefix(3, 1.0)));
    assert!(!prefix(3, 1.0).beats(exact(100.0)));
    // Among same-named exact matches, the nearest wins.
    assert!(exact(4.0).beats(exact(9.0)));
    assert!(!exact(9.0).beats(exact(4.0)));
}

#[test]
fn longer_prefix_beats_nearer_shorter_one() {
    // The reference's `jg` accept: a longer prefix wins at any distance.
    assert!(prefix(5, 900.0).beats(prefix(3, 1.0)));
    assert!(!prefix(3, 1.0).beats(prefix(5, 900.0)));
    // Equal prefix length falls through to strictly-nearest.
    assert!(prefix(3, 4.0).beats(prefix(3, 9.0)));
}

#[test]
fn ties_keep_the_incumbent_and_nan_never_displaces() {
    // The reference's `jp` reject edge makes the accept a strict `<`.
    assert!(!prefix(3, 4.0).beats(prefix(3, 4.0)));
    assert!(!exact(4.0).beats(exact(4.0)));
    assert!(!prefix(3, f32::NAN).beats(prefix(3, 4.0)));
}
