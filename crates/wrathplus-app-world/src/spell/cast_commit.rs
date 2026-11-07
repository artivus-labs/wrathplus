//! Cast commit identity shared by the send ladder and targeting cursor.
use super::cast_fail_keys::reason;

/// What the commit sends: `SendCast 0x6e54f0` branches on whether the pending-cast guid
/// (`0xceac48`, the item's when TryCast got one, else the caster's) is the caster's; the item arm
/// sends `CMSG_USE_ITEM` (`0x6e57d8 push 0xab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastCommit {
    /// `CMSG_CAST_SPELL` (0x12e).
    Spell,
    /// Build 12340's `CMSG_PET_CAST_SPELL` (0x1f0): our pet's, charm's or possessed unit's spell,
    /// raised to the cursor by its bar ([`super::cast_target::pet_cursor_word`]).
    Pet { caster: u64 },
    /// `CMSG_USE_ITEM` (0xab): the item's wire position, plus a caller-bound
    /// GameObject.
    Item {
        bag_index: u8,
        slot: u8,
        /// The item's template entry: the not-ready rung queries the `(use_spell, entry)` record.
        entry: u32,
        /// The item's GUID: build 12340's `CMSG_USE_ITEM` names it, and AC refuses a mismatch.
        item_guid: u64,
        /// `CGItem::Use`'s own target argument, set only for a key in a lock; it bypasses the
        /// binder, as TryCast's target resolve (`6e4ef4`) takes the pair it was passed.
        on_object: Option<u64>,
    },
}

impl CastCommit {
    /// Whether this commit carries an item: what every forked rung reads.
    pub(crate) fn is_item(self) -> bool {
        matches!(self, CastCommit::Item { .. })
    }

    /// The local not-ready reason for this commit (`0x6094f0`'s forked first rung).
    pub(super) fn not_ready_reason(self) -> u8 {
        if self.is_item() {
            reason::ITEM_NOT_READY
        } else {
            reason::NOT_READY
        }
    }
}
