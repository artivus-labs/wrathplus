//! Player spell press and combat-validation retry share the normal ladder.
use super::*;
impl CastLadder<'_, '_> {
    /// A player's spell press (`UseAction`, `CastSpell`, `CastSpellByName`): the ladder with
    /// TryCast's attack pick, where the cast may stop in [`HeldForPick`].
    pub fn send_spell(&mut self, spell_id: u32, ctx: &cast_target::CastContext, on_self: bool) {
        self.send_spell_at(spell_id, ctx, on_self, None);
    }

    /// A supplied GUID remains attached to the press across the combat-validation hold.
    pub fn send_spell_at(
        &mut self,
        spell_id: u32,
        ctx: &cast_target::CastContext,
        on_self: bool,
        explicit: Option<u64>,
    ) {
        let held = HeldCast {
            spell_id,
            on_self,
            explicit,
        };
        self.send_bound(spell_id, ctx, CastCommit::Spell, None, Some(held));
    }

    /// The press held at the attack pick, if one is.
    pub fn take_held(&mut self) -> Option<HeldCast> {
        self.held_for_pick.0.take()
    }

    /// A held press after its pick, at the unit the pick left selected. The rungs above the pick
    /// run again; none reads the selection, so each passes as it did.
    pub fn resume_after_pick(&mut self, held: HeldCast, ctx: &cast_target::CastContext) {
        self.send_bound(held.spell_id, ctx, CastCommit::Spell, None, None);
    }
}
