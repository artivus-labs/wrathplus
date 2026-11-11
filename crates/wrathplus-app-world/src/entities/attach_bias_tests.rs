//! The attach-point Z-bias table's test, split out of `entities.rs`.

use super::*;

/// The table at `0x862708`, verbatim; animation events reach it by ids 17 and 1.
#[test]
fn the_z_bias_table_is_the_reference_row_for_ever_id() {
    assert_eq!(ATTACH_Z_BIAS.len(), 0x25, "ids 0..=0x24");
    assert_eq!(ATTACH_Z_BIAS[17], 2.0, "$CSD's fallback lift");
    assert_eq!(ATTACH_Z_BIAS[1], 1.0, "a whiffed $CSS's");
    // Values other than 1.0 and 1.5, where a transcription slip shows.
    assert_eq!(ATTACH_Z_BIAS[19], 0.0);
    assert_eq!(ATTACH_Z_BIAS[29], 3.0);
    assert_eq!(ATTACH_Z_BIAS[18], 2.5);
    // Out of range reads as no lift, matching the reference's `0 ≤ id < 0x25` guard.
    assert_eq!(ATTACH_Z_BIAS.get(0x25), None);
}
