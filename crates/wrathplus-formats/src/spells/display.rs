//! `SpellDisplay`: one spell's `Spell.dbc` row and the client predicates over it.

use super::*;

/// A spell's `SPELL_EFFECT_OPEN_LOCK` effect: the `LockType` it opens and the slot it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenLock {
    /// `EffectMiscValue`: the `LockType.dbc` id, such as pick lock 1, herbalism 2, mining 3.
    pub lock_type: u32,
    /// Which of `Effect[0..3]` is the OPEN_LOCK one.
    pub effect: usize,
}

/// One spell's `Spell.dbc` row, as the display, tooltip and cast gates read it.
pub struct SpellDisplay {
    /// `ID` (column 0), which `GetEffectPoints` hands the level lookup `0x6e3130` (`0x6e384b`).
    pub id: u32,
    pub name: String,
    /// `SpellNameSubtext` enUS (column 129): "Rank N", even "Rank 1", or a word such as "Racial".
    pub rank: Option<String>,
    /// The icon's MPQ path, without extension; `None` for icon 0 or an id not in `SpellIcon.dbc`.
    pub icon: Option<String>,
    /// `SpellVisual.dbc` id (column 115); 0 is a silent cast.
    pub visual: u32,
    /// Projectile speed (column 37) in world units per second; 0 is an instant impact.
    pub speed: f32,
    /// `Attributes` (column 6).
    pub attributes: u32,
    /// `School` (column 1): an index; immunity tests `1 << School` against a school mask.
    pub school: u32,
    /// `Mechanic` (column 5): the whole spell's `SpellMechanic.dbc` id.
    pub mechanic: u32,
    /// `AttributesEx` (column 7).
    pub attributes_ex: u32,
    /// `AttributesEx2` (column 8).
    pub attributes_ex2: u32,
    /// `AttributesEx3` (column 9). Bits `0x400` and `0x1000000` limit the equipped-item search to
    /// the main or the off hand (`0x5f0c50`).
    pub attributes_ex3: u32,
    /// `AttributesEx4` (column 10, `+0x28`).
    pub attributes_ex4: u32,
    /// Build 12340's `AttributesEx5` (`+0x24`): `0x8` usable while stunned, `0x20000` while
    /// feared, `0x40000` while confused (AzerothCore `SPELL_ATTR5_USABLE_WHILE_*`).
    pub attributes_ex5: u32,
    /// Build 12340's `AttributesEx7` (`+0x2c`): `0x800` interrupts only non-players, `0x1000`
    /// silences only non-players, `0x2000` the cast cannot be shown as uninterruptible.
    pub attributes_ex7: u32,
    /// Build 12340's `AttributesEx6` (`+0x28`): `0x1` hides the tooltip cooldown cell,
    /// `0x80000000` keeps op 11 off the category recovery.
    pub attributes_ex6: u32,
    /// Build 12340's `SchoolMask` (`+0x284`), one bit per school.
    pub school_mask: u32,
    /// `SpellFamilyName` (column 160): `GetSpellModifiers` (`0x6e6b30`) applies nothing unless it
    /// is nonzero and equals the local player's class family (`[0xcecaac]`, `0x6e6b46`).
    pub spell_family: u32,
    /// `SpellFamilyFlags` (columns 161-162, low dword first): the modifier rows that apply, one
    /// summed cell per set bit over all 64; Cleanse 4987 sets bits 12 and 33.
    pub spell_family_flags: u64,
    /// `SpellFamilyFlags[2]` (3.3.5 column 211): family bits 64-95, the third word of build
    /// 12340's 96-bit mask; see [`Self::has_family_bit`].
    pub spell_family_flags_2: u32,
    /// `EffectSpellClassMask[3]` (3.3.5 columns 122-130): per effect, the 96 family bits (three
    /// words, low first) its spell-modifier aura applies to.
    pub effect_spell_class_mask: [[u32; 3]; 3],
    /// `PreventionType` (column 165): 1 silence, 2 pacify, 0 neither; the crowd control that
    /// refuses the spell locally (`0x6094f0`, from `TryCast` `0x6e4b60`). A stun refuses all.
    pub prevention_type: u32,
    /// `modalNextSpell` (column 38): cast at no target once this spell's `SMSG_CAST_RESULT` lands
    /// (`0x6e74aa`, `0x6e7330`), a running auto-repeat only re-armed; hunter shots name 75.
    pub modal_next_spell: u32,
    /// `Attributes & 0x40`, `SPELL_ATTR_PASSIVE`: the spellbook grays it.
    pub passive: bool,
    /// `castUI` (column 3): nonzero keeps the spell out of the spellbook; 0 on ordinary spells.
    pub cast_ui: u32,
    /// `Effect[3]` (columns 61-63); the trainer's icon scans all three (`0x4d8fed`).
    pub effects: [u32; 3],
    /// The `SPELL_EFFECT_OPEN_LOCK` effect the GameObject interact-cast matches against a lock.
    pub open_lock: Option<OpenLock>,
    /// `baseLevel` (column 28): the level effect values are quoted at (`0x6e3854`, `0x6e3340`).
    pub base_level: u32,
    /// `maxLevel` (column 27): caps an opener's skill at `maxLevel × 5`, unless 0 (`0x5ea6e3`).
    pub max_level: u32,
    /// `spellLevel` (column 29), read only by the Beast Training rank comparator.
    pub spell_level: u32,
    /// `Dispel` (column 4): the `SpellDispelType.dbc` id, as `UnitDebuff`'s third return.
    pub dispel: u32,
    /// `Category` (column 2): the shared-cooldown category; 0 is none.
    pub category: u32,
    /// The category's `SpellCategory.dbc` flags carry `0x2`, matching every cooldown query
    /// (`GetCooldownInfo` `0x6e13e0`); true only for wand Shoot's 351, whose swing sweeps the bar.
    pub category_wildcard: bool,
    /// The category's `SpellCategory.dbc` flags carry `0x1`: its recovery scales by the main-hand
    /// weapon's delay, per 1000 ms (`007fef60`).
    pub category_weapon_speed: bool,
    /// `RecoveryTime` (column 19): the spell's own cooldown in ms.
    pub recovery_ms: u32,
    /// `InterruptFlags` (column 21): what breaks the cast; `0xf` on timed casts, `0x1` movement.
    pub interrupt_flags: u32,
    /// `AuraInterruptFlags` (column 22): what breaks the applied aura, such as food's sit-still
    /// bits. The cast-start moving gate reads its `0x18`, moving or turning (`0x609de3`).
    pub aura_interrupt_flags: u32,
    /// `ChannelInterruptFlags` (column 23): what breaks the channel, in aura-interrupt bits.
    pub channel_interrupt_flags: u32,
    /// `CategoryRecoveryTime` (column 20): the category's shared cooldown in ms.
    pub category_recovery_ms: u32,
    /// `StartRecoveryCategory` (column 157): the global cooldown's category, usually 133.
    pub start_recovery_category: u32,
    /// `StartRecoveryTime` (column 158): the global cooldown in ms, usually 1500; 0 for none.
    pub start_recovery_ms: u32,
    /// `powerType` (column 31): 0 mana, 1 rage, 3 energy; health is -2, stored as `0xFFFFFFFE`
    /// (Life Tap, Health Funnel), and the reference reads negative, or 5 and up, as health.
    pub power_type: u32,
    /// `manaCost` (column 32): the flat cost in `power_type`'s unit.
    pub mana_cost: u32,
    /// `ManaCostPercentage` (column 156): percent of base mana added to the flat cost.
    pub mana_cost_pct: u32,
    /// `manaCostPerlevel` (column 33): the per-level cost term of `GetPowerCost` (`0x6e31b0`).
    pub mana_cost_per_level: u32,
    /// `manaPerSecond` (column 34): the tooltip's "plus N per sec"; column 35 is 0 on every row.
    pub mana_per_second: u32,
    /// `RuneCostID`: the `SpellRuneCost.dbc` row a rune-cost spell spends
    /// ([`SpellCatalog::rune_cost`]).
    pub rune_cost_id: u32,
    /// `PowerDisplayId`: the `PowerDisplay.dbc` row naming a vehicle power
    /// ([`SpellCatalog::power_display`]); 0 for none.
    pub power_display_id: u32,
    /// `rangeIndex` (column 36): the `SpellRange.dbc` row.
    pub range_index: u32,
    /// `Targets` (column 13): the `TARGET_FLAG_*` seed of the cast arm's targeting word.
    pub targets: u32,
    /// `TargetCreatureType` (column 14): the creature types the spell may target, bit `type - 1`
    /// each; 0 is any. Hibernate takes Beast and Dragonkin, Banish Demon and Elemental.
    pub target_creature_type: u32,
    /// `EffectImplicitTargetA[0]` (column 82): the cast arm's switch adjusts the targeting word
    /// by it, and the usable walk's target-aura-state leg forks on it (6 enemy, 21 friend).
    pub implicit_target_a1: u32,
    /// `EffectImplicitTargetA[3]` (columns 82-84), then `EffectImplicitTargetB[3]` (85-87).
    pub effect_implicit_target_a: [u32; 3],
    pub effect_implicit_target_b: [u32; 3],
    /// `Stances` (column 11): forms the spell may be cast in, `1 << (form - 1)` each; 0 is none.
    pub stances: u32,
    /// `StancesNot` (column 12): forms the spell may not be cast in.
    pub stances_not: u32,
    /// `CasterAuraState` (column 16): state n needs bit `1 << (n - 1)` of the caster's
    /// `UNIT_FIELD_AURASTATE`; 0 is none.
    pub caster_aura_state: u32,
    /// `TargetAuraState` (column 17): the current target's required aura state (`0x6e3f58`).
    pub target_aura_state: u32,
    /// `Totem[2]` (columns 40-41): tools that must be carried, not consumed, such as a hammer.
    pub totems: [u32; 2],
    /// `RequiredTotemCategoryID[2]`: `TotemCategory.dbc` tool kinds, served by any carried item
    /// of a compatible category (`TotemCategoryCatalog::serves`).
    pub totem_categories: [u32; 2],
    /// `Reagent[8]` and `ReagentCount[8]` (columns 42-49, 50-57): consumed items and counts.
    pub reagents: [(u32, u32); 8],
    /// `EquippedItemClass` (column 58, signed): the item class a worn item must be; -1 is none.
    pub equipped_item_class: i32,
    /// `EquippedItemSubClassMask` (column 59): the allowed subclasses, `1 << subclass` each.
    pub equipped_item_subclass_mask: u32,
    /// `EquippedItemInventoryTypeMask` (column 60): `1 << InventoryType` each; the item-target
    /// gate's last leg (`0x495d60`), whose mismatch is the local "Invalid target" (`0x0a`).
    pub equipped_item_inventory_type_mask: u32,
    /// `RequiresSpellFocus` (column 15): the `SpellFocusObject.dbc` object needed nearby.
    pub requires_spell_focus: u32,
    /// The `SpellShapeshiftForm.dbc` form: the first `SPELL_AURA_MOD_SHAPESHIFT` effect's
    /// `EffectMiscValue`, which the stance bar keys on (`0x4b2810`, `0x4b475c`).
    pub shapeshift_form: Option<u32>,
    /// `StanceBarOrder` (column 166, signed): the stance bar's sort key, negative last.
    pub stance_bar_order: i32,
    /// `ActiveIconID`'s texture, shown on the stance button while the form is active.
    pub active_icon: Option<String>,
    /// `ActiveIconID` (column 118) as stored: nonzero makes a second press cancel the spell's
    /// live aura. The client tests the id, not the texture (`0x4e55f0`, `0x4b36f0`).
    pub active_icon_id: u32,
    /// `Description` enUS (column 138): the tooltip body, its `$` tokens unsubstituted.
    pub description: Option<String>,
    /// `AuraDescription` enUS (column 147): the buff or debuff icon's tooltip.
    pub aura_description: Option<String>,
    /// `SpellDescriptionVariables.dbc`'s row text: the `$<name>` definitions (`0x579d60`).
    pub description_variables: Option<String>,
    /// `DurationIndex` (column 30): the `SpellDuration.dbc` row; 0 for no duration.
    pub duration_index: u32,
    /// `CastingTimeIndex` (column 18): the `SpellCastTimes.dbc` row; row 1 is instant.
    pub casting_time_index: u32,
    /// `ProcFlags` (column 24, `SpellRec+0x60`): bit 0 makes the description's `$t` a fixed five
    /// seconds.
    pub proc_flags: u32,
    /// `ProcChance` (column 25): percent; vmangos reads 101 as always, with no roll.
    pub proc_chance: u32,
    /// `ProcCharges` (column 26): the description expander's `$n` value before op 4.
    pub proc_charges: u32,
    /// `StackAmount` (column 39): the description expander's `$u` (`507ef1`).
    pub stack_amount: u32,
    /// `MaxTargetLevel` (column 159): the description expander's `$v` (`507f4f`).
    pub max_target_level: u32,
    /// `MaxAffectedTargets` (column 163): the description expander's `$i` (`507fc3`).
    pub max_affected_targets: u32,
    /// `EffectBasePoints[3]` (columns 76-78, signed): each roll's floor; -1 on weapon damage.
    pub effect_base_points: [i32; 3],
    /// `EffectDieSides[3]` (columns 64-66, signed): n dice run base + n to base + sides × n.
    pub effect_die_sides: [i32; 3],
    /// `EffectBaseDice[3]` (columns 67-69).
    pub effect_base_dice: [i32; 3],
    /// `EffectAmplitude[3]` (columns 94-96): a periodic effect's tick in ms; 0 is not periodic.
    pub effect_amplitude: [u32; 3],
    /// `EffectApplyAuraName[3]` (columns 91-93): each effect's aura type; 0 is not an aura.
    pub effect_apply_aura: [u32; 3],
    /// `EffectMechanic[3]` (columns 79-81): each effect's `SpellMechanic.dbc` id. Immunity
    /// matches it or [`Self::mechanic`]; the `0x8d` "Can't do that while %s" line prefers it.
    pub effect_mechanic: [u32; 3],
    /// `EffectRadiusIndex[3]` (columns 88-90): each effect's `SpellRadius.dbc` row; 0 is none.
    pub effect_radius_index: [u32; 3],
    /// `EffectChainTarget[3]` (columns 100-102): extra chain targets beyond the first.
    pub effect_chain_targets: [u32; 3],
    /// `EffectMultipleValue[3]` (columns 97-99, float): the chain or multi-target falloff.
    pub effect_multiple_value: [f32; 3],
    /// `DmgMultiplier[3]` (columns 167-169, float): `$f`/`$F` print it scaled (`507ff7`, `508025`).
    pub damage_multiplier: [f32; 3],
    /// `EffectPointsPerComboPoint[3]` (columns 112-114, float): `$b` prints it chopped (`507ed4`).
    pub effect_points_per_combo_point: [f32; 3],
    /// `EffectBonusMultiplier[3]` (3.3.5 columns 229-231): `$bc1` (`0x5782d0`, op 24).
    pub effect_bonus_multiplier: [f32; 3],
    /// `EffectTriggerSpell[3]` (columns 109-111): each effect's triggered spell, 0 for none.
    pub effect_trigger_spell: [u32; 3],
    /// `EffectItemType[3]` (columns 103-105): the item a `SPELL_EFFECT_CREATE_ITEM` effect makes.
    pub effect_item_type: [u32; 3],
    /// `EffectMiscValue[3]` (columns 106-108, signed).
    pub effect_misc_value: [i32; 3],
    /// `EffectMiscValueB[3]`: a SUMMON effect's `SummonProperties.dbc` row (Wrath data only).
    pub effect_misc_value_b: [i32; 3],
    /// `RequiredAuraVision` (file column 221): one more than the viewer's aura-vision bit.
    pub required_aura_vision: u32,
    /// `EffectDicePerLevel[3]` (columns 70-72): the integer per-level term (`0x6e3871`).
    pub effect_dice_per_level: [i32; 3],
    /// `EffectRealPointsPerLevel[3]` (columns 73-75): the float per-level term (`0x6e3889`).
    pub effect_real_points_per_level: [f32; 3],
}

impl SpellDisplay {
    /// Whether family bit `bit` (0-95) is set: bits 0-63 in [`Self::spell_family_flags`], 64-95
    /// in [`Self::spell_family_flags_2`].
    pub fn has_family_bit(&self, bit: u32) -> bool {
        match bit {
            0..64 => self.spell_family_flags >> bit & 1 == 1,
            64..96 => self.spell_family_flags_2 >> (bit - 64) & 1 == 1,
            _ => false,
        }
    }

    /// The level the effect values, cast time, cost and duration scale by for a caster whose
    /// skill in this spell's line is `skill_value` (`0x6e3130`): the skill capped at
    /// `maxLevel × 5` unless `maxLevel` is 0 (`0x5ea6dc`-`0x5ea6ea`), over 5 (`0x6e3195`).
    pub fn skill_level(&self, skill_value: u32) -> u32 {
        let capped = if self.max_level > 0 {
            skill_value.min(self.max_level.saturating_mul(5))
        } else {
            skill_value
        };
        capped / 5
    }

    /// The `LockType` this spell opens, if any.
    pub fn open_lock_type(&self) -> Option<u32> {
        self.open_lock.map(|o| o.lock_type)
    }

    /// The lock skill this spell provides (`0x5f850f`), from the player's skill in its line:
    /// the effect value (`0x6e3800`) with its caller's rounding (`0x6e3760`). Its level term is
    /// that skill with its bonuses (`0x5ea690`), not the caster's level; a skill of 0 leaves only
    /// the flat terms.
    pub fn open_lock_skill(&self, skill_value: u32) -> Option<i32> {
        let e = self.open_lock?.effect;
        // Less baseLevel floored at 0 (read at `0x6e3854`, subtracted at `0x6e385b`-`0x6e385f`).
        let delta = self
            .skill_level(skill_value)
            .saturating_sub(self.base_level) as f32;
        let v = self.effect_base_points[e] as f32
            + self.effect_base_dice[e] as f32
            + self.effect_dice_per_level[e] as f32 * delta
            + self.effect_real_points_per_level[e] * delta;
        // `0x6e3760`'s rounding verbatim: double, bias away from zero by a half, round-to-nearest
        // (x87 `fistp`), then arithmetic-halve.
        let doubled = if v >= 0.0 {
            v * 2.0 - 0.5
        } else {
            v * 2.0 + 0.5
        };
        Some((doubled.round_ties_even() as i32) >> 1)
    }

    /// Build 12340's helpful/harmful classifier `0x7fe1b0`: 1 helpful, 2 harmful, 0 neither.
    /// `Targets & 0x100` (ally) is helpful and then `& 0x80` (enemy) harmful (`0x7fe1ba`); else
    /// any implicit target, A then B per effect, in the enemy set is harmful (tables `0x7fe278`,
    /// `0x7fe2dc`); else one in the friend set, or `TARGET_UNIT_CASTER` (1) on an effect whose
    /// aura is not `DUMMY` (4), is helpful (`0x7fe340`, `0x7fe38c`, `0x7fe23d`).
    pub fn benefit(&self) -> u8 {
        const ENEMY: [u32; 9] = [2, 6, 15, 16, 24, 28, 53, 54, 93];
        const FRIEND: [u32; 19] = [
            3, 4, 5, 20, 21, 27, 29, 30, 31, 33, 34, 35, 45, 56, 57, 58, 59, 61, 62,
        ];
        if self.targets & 0x100 != 0 {
            return 1;
        }
        if self.targets & 0x80 != 0 {
            return 2;
        }
        let (a, b) = (self.effect_implicit_target_a, self.effect_implicit_target_b);
        if (0..3).any(|i| ENEMY.contains(&a[i]) || ENEMY.contains(&b[i])) {
            return 2;
        }
        let friend =
            |t: u32, i: usize| FRIEND.contains(&t) || (t == 1 && self.effect_apply_aura[i] != 4);
        u8::from((0..3).any(|i| friend(a[i], i) || friend(b[i], i)))
    }

    /// `IsHelpfulSpell` (`0x541893`): the classifier answers helpful. It also picks the friendly
    /// range pair when no target is given (`0x801692`).
    pub fn is_helpful(&self) -> bool {
        self.benefit() == 1
    }

    /// `IsHarmfulSpell` (`0x541983`): the classifier answers harmful. It also gates the victim's
    /// flinch at a spell impact: the per-hit loop `0x801f10` hands `== 2` (`0x80200c`) to the kit
    /// play `0x745230`, whose unit call `0x73b140` then plays the wound `0x736640` (`0x73b413`).
    pub fn is_harmful(&self) -> bool {
        self.benefit() == 2
    }

    /// The ranged-stance gate, `AttributesEx2 & 0x20` or `Attributes & 0x2`, as tested at
    /// `0x6e78b6`/`0x6e78f3` (`SMSG_SPELL_START`) and `0x6e5930` (the cast send).
    pub fn ranged_attack(&self) -> bool {
        self.attributes_ex2 & ATTR_EX2_AUTO_REPEAT != 0 || self.attributes & ATTR_RANGED != 0
    }

    /// `Attributes & 0x200`: the cast aims itself at the equipped main hand, so a weapon imbue
    /// binds on the press with no item cursor (`ArmCast` `0x6e5250`, for a player caster). Only
    /// the shaman weapon imbues carry it.
    pub fn targets_main_hand_item(&self) -> bool {
        self.attributes & ATTR_TARGET_MAIN_HAND_ITEM != 0
    }

    /// `AttributesEx2 & 0x20` alone: the cast send sets the client's auto-repeat state from it
    /// (`0x6e593b`), which drives the shooter's Load/Hold idle. Throw is ranged, not auto-repeat.
    pub fn auto_repeat(&self) -> bool {
        self.attributes_ex2 & ATTR_EX2_AUTO_REPEAT != 0
    }

    /// `AttributesEx4 & 0x20` (`0x4bd355`): a pet-bar spell press takes the generic cast entry
    /// `0x6e4b60` and returns, sending no pet action and starting no GCD of its own.
    pub fn allows_client_targeting(&self) -> bool {
        self.attributes_ex4 & ATTR_EX4_ALLOW_CLIENT_TARGETING != 0
    }

    /// `AttributesEx3 & 0x400000`: a running auto-repeat with this bit ends when any new cast
    /// begins (`0x60959e`). Only wand Shoot 5019 has it; Auto Shot survives, so hunters weave.
    pub fn casting_cancels_autorepeat(&self) -> bool {
        self.attributes_ex3 & 0x0040_0000 != 0
    }

    /// `Attributes & 0x2` alone: a ranged-slot spell with no `SpellVisual` borrows the ranged
    /// weapon's `ItemDisplayInfo` visual (`0x60d46a`), which is how Throw and the shots animate.
    pub fn ranged_slot(&self) -> bool {
        self.attributes & ATTR_RANGED != 0
    }

    /// `AttributesEx3 & 0x8000`: damage floats melee white, not spell gold (`0x6128b0`); set on
    /// the basic ranged shots (Auto Shot, Shoot Bow, Throw, wand Shoot).
    pub fn melee_white_damage(&self) -> bool {
        self.attributes_ex3 & ATTR_EX3_NORMAL_RANGED_ATTACK != 0
    }

    /// `AttributesEx3 & 0x4`: `SPELLCAST_START` gets an empty name, a blank cast bar label
    /// (`0x6e7a2d`, the one test of this bit); the channel bar never reads it.
    pub fn no_casting_bar_text(&self) -> bool {
        self.attributes_ex3 & ATTR_EX3_NO_CASTING_BAR_TEXT != 0
    }

    /// `AttributesEx3 & 0x2000`: no channel bar; the handler `0x6e7550` returns at `0x6e7595`
    /// before `SPELLCAST_CHANNEL_START`. Only Blood Siphon, 24322 and 24323, carries it.
    pub fn no_channel_bar(&self) -> bool {
        self.attributes_ex3 & ATTR_EX3_NO_CHANNEL_BAR != 0
    }

    /// `AttributesEx & 0x80000`: the caster is never a bind candidate, not in `BindTarget`
    /// (`0x6e5b40` at `6e5bf7`), not for `SpellCanTargetUnit`'s unit leg (`0x6e6460` at `6e6507`)
    /// and not for the world pick (`0x6e61a0` at `6e61cf`).
    pub fn excludes_caster(&self) -> bool {
        self.attributes_ex & ATTR_EX_EXCLUDE_CASTER != 0
    }

    /// `AttributesEx2 & 0x1` (vmangos `SPELL_ATTR_EX2_ALLOW_DEAD_TARGET`, `SpellDefines.h:868`):
    /// a dead unit stays a bind candidate (`BindTarget` at `6e5c85`).
    pub fn allows_dead_target(&self) -> bool {
        self.attributes_ex2 & ATTR_EX2_ALLOW_DEAD_TARGET != 0
    }

    /// The `TargetCreatureType` gate of `BindTarget` (`6e5c23`-`6e5c53`) and its mirror
    /// (`6e6544`-`6e656c`): no mask admits any unit, else a unit of type 0 is refused and type `t`
    /// needs bit `t - 1`. The shift wraps at 32 as the hardware's does.
    pub fn admits_creature_type(&self, creature_type: u32) -> bool {
        self.target_creature_type == 0
            || (creature_type != 0
                && self.target_creature_type & 1u32.wrapping_shl(creature_type - 1) != 0)
    }

    /// `AttributesEx & 0x2000_0000` (`0x6e759a`): the channel bar shows the spell's own name
    /// (`0x6e75a9`), else `CHANNELING` (`0x6e75bc`); nine channels set it, Fishing among them.
    pub fn channel_bar_own_name(&self) -> bool {
        self.attributes_ex & ATTR_EX_CHANNEL_BAR_OWN_NAME != 0
    }

    /// `Attributes & 0x2` without `AttributesEx2 & 0x20000`: the caster's `SMSG_SPELL_GO` adds
    /// `UNIT_FIELD_RANGEDATTACKTIME` to its category cooldown locally (`0x6e2b60`).
    pub fn ranged_speed_cooldown(&self) -> bool {
        self.attributes & ATTR_RANGED != 0
            && self.attributes_ex2 & ATTR_EX2_DO_NOT_RESET_COMBAT_TIMERS == 0
    }

    /// The spellbook's add gate (`AddSpell` `0x5e9c20`, `0x4b25b0`): a spell stays out for
    /// `Attributes` `0x80` or `0x20`, or a nonzero `castUI`; passive alone does not hide it.
    pub fn in_spellbook(&self) -> bool {
        self.attributes & (ATTR_DO_NOT_DISPLAY | SPELL_ATTR_IS_TRADESKILL) == 0 && self.cast_ui == 0
    }

    /// The pet book's add gate (`0x4b2f90`) tests only `Attributes & 0x80`, as the sign of its
    /// low byte (`0x4b2fad`); it has none of [`Self::in_spellbook`]'s other two legs.
    pub fn in_pet_book(&self) -> bool {
        self.attributes & ATTR_DO_NOT_DISPLAY == 0
    }

    /// `Effect[0] == SPELL_EFFECT_ATTACK` (78): the client's trigger for showing the main-hand
    /// weapon's icon instead (`0x4b3f8a`, `0x4e59de`). In 1.12 only 6603 "Attack" carries it.
    pub fn is_melee_auto_attack(&self) -> bool {
        self.effects[0] == SPELL_EFFECT_ATTACK
    }

    /// The tooltip skips its cast-time and cooldown line for a passive or `Effect[0]` 47 or 78
    /// (`0x52eb15`-`0x52eb45`); [`Self::passive`] must keep reading the attribute alone.
    pub fn tooltip_omits_cast_line(&self) -> bool {
        self.passive
            || matches!(
                self.effects[0],
                SPELL_EFFECT_TRADE_SKILL | SPELL_EFFECT_ATTACK
            )
    }

    /// No tooltip range cell: tested before `GetMinMaxRange` (`0x6e3480`), an on-next-swing spell
    /// (`0x52e9a5`) or `AttributesEx3` bit 30 (`0x52e9b2`); after it, a max of 0 (`0x52e9ed`).
    pub fn tooltip_omits_range_line(&self) -> bool {
        self.on_next_swing() || self.attributes_ex3 & 0x4000_0000 != 0
    }

    /// `GetCooldownInfo` (`0x6e13e0`) reports no cooldown for `Effect[0]` 78 (attack) or 47
    /// (trade skill): those buttons never show one and are never refused by one.
    pub fn cooldown_query_excluded(&self) -> bool {
        matches!(
            self.effects[0],
            SPELL_EFFECT_TRADE_SKILL | SPELL_EFFECT_ATTACK
        )
    }

    /// The ranged icon substitution needs `Attributes & 0x2` and `AttributesEx2 & 0x20` both
    /// (`0x4b3f99`, `0x4e5a2e`): Auto Shot and wand Shoot show the weapon's icon, Throw its own.
    pub fn ranged_icon_substitution(&self) -> bool {
        self.attributes & ATTR_RANGED != 0 && self.attributes_ex2 & ATTR_EX2_AUTO_REPEAT != 0
    }

    /// `SPELL_ATTR_COOLDOWN_ON_EVENT`: the cooldown is held until `SMSG_COOLDOWN_EVENT`.
    pub fn cooldown_on_event(&self) -> bool {
        self.attributes & ATTR_COOLDOWN_ON_EVENT != 0
    }

    /// A finisher (`AttributesEx` bit 20 or 22): unusable with no combo points (`0x6e3e7a`).
    pub fn needs_combo_points(&self) -> bool {
        self.attributes_ex & ATTR_EX_FINISHING_MOVE != 0
    }

    /// `Attributes & 0x404`: queued on the server's melee slot for the next swing, not cast.
    pub fn on_next_swing(&self) -> bool {
        self.attributes & ATTR_ON_NEXT_SWING != 0
    }

    /// The tooltip's "Attack speed" arm: `Attributes & 0x2` alone (`0x52ec1c`), so Throw too.
    pub fn tooltip_on_next_ranged(&self) -> bool {
        self.attributes & ATTR_RANGED != 0
    }

    /// The tooltip cast cell's "Channeled" arm: `AttributesEx & 0x44` (`0x52ec27`).
    pub fn tooltip_channeled(&self) -> bool {
        self.attributes_ex & ATTR_EX_CHANNELED != 0
    }

    /// Predicate `0x6e5200`: an on-next-swing, `INITIATES_COMBAT` or `INITIATE_COMBAT_POST_CAST`
    /// spell. TryCast runs the attack validator's target pick for it (`0x6e4edf`), so a press
    /// with no hostile selection acquires one, as the Attack button does.
    pub fn initiates_combat(&self) -> bool {
        self.attributes & ATTR_ON_NEXT_SWING != 0
            || self.attributes_ex & ATTR_EX_INITIATES_COMBAT != 0
            || self.attributes_ex2 & ATTR_EX2_INITIATE_COMBAT_POST_CAST != 0
    }

    /// Casting this starts melee auto-attack at the send unless one runs (`TryCast` tail
    /// `0x6e51b5`): predicate `0x6e5200` with `AttributesEx2` bit 20 clear, so an on-next-swing
    /// or `INITIATES_COMBAT` spell. Every cast path shares the tail; there is no macro opt-out.
    pub fn initiates_auto_attack(&self) -> bool {
        self.initiates_combat() && !self.initiates_auto_attack_at_go()
    }

    /// This spell's own `SMSG_SPELL_GO` starts melee auto-attack (`0x6131a0`) at its first hit
    /// target: `AttributesEx2` bit 20, the complement of [`Self::initiates_auto_attack`]
    /// (`0x6e83c0`).
    ///
    /// Only this leg is built: the handler's other, `0x6e5200` with `InterruptFlags` bit 3
    /// (`0x6e83d4`), never fires, as its rows already attack from the send (`0x6e83e7`). Built
    /// here, it would send a second `CMSG_ATTACKSWING`: our engaged state is the server's echo.
    pub fn initiates_auto_attack_at_go(&self) -> bool {
        self.attributes_ex2 & ATTR_EX2_INITIATE_COMBAT_POST_CAST != 0
    }

    /// Hidden from the buff bar (`0x4e4170`, `0x4e42b6`-`0x4e42c8`): `Attributes` read as a byte,
    /// so `0x80` and not the dword's sign, or `AttributesEx & 0x10000000`. The aura stays live;
    /// `GetPlayerBuff` never returns it: warrior stances, Defensive State 5302.
    pub fn hidden_from_aura_bar(&self) -> bool {
        self.attributes & ATTR_DO_NOT_DISPLAY != 0 || self.attributes_ex & ATTR_EX_NO_AURA_ICON != 0
    }

    /// A tracking spell (`EffectApplyAuraName` 44, 45 or 151): never in an aura display
    /// (`0x519860`); the player's last one feeds `GetTrackingTexture`, the minimap icon.
    pub fn tracking_aura(&self) -> bool {
        self.effect_apply_aura
            .iter()
            .any(|a| TRACKING_AURA_TYPES.contains(a))
    }

    /// `AttributesEx2` bit 19: `Stances` lists forms the spell may also be cast in, not forms it
    /// needs. The form gate `0x612480` and the tooltip's form line (`0x52f115`) both test it; no
    /// spell that truly needs a form (stances, druid forms, stealth openers) carries it.
    pub fn form_mask_is_permissive(&self) -> bool {
        self.attributes_ex2 & ATTR_EX2_ALLOW_WHILE_NOT_SHAPESHIFTED != 0
    }

    /// The form gate as a boolean, the usable walk's form leg (`0x6e3ec6`).
    pub fn usable_in_form(&self, form: u8, form_is_stance: bool) -> bool {
        self.form_refusal(form, form_is_stance).is_none()
    }

    /// The shapeshift-form gate `0x612480` (usable walk `0x6e3ec6`, cast check `0x6094f0`):
    /// `StancesNot`, then `Stances`, then `Attributes` bit 16 and `AttributesEx2` bit 19. A form
    /// flagged as a stance (warrior stances, stealth) does not count as shapeshifted, and the
    /// refusal reasons split as in vmangos's `SpellEntry::GetErrorAtShapeshiftedCast`.
    pub fn form_refusal(&self, form: u8, form_is_stance: bool) -> Option<FormRefusal> {
        // A form past 32 names no bit of the 32-bit masks.
        let stance_bit = match form {
            0 => 0,
            f => 1u32.checked_shl(u32::from(f) - 1).unwrap_or(0),
        };
        if self.stances_not & stance_bit != 0 {
            return Some(FormRefusal::NotShapeshift);
        }
        if self.stances & stance_bit != 0 {
            return None;
        }
        if form != 0 && !form_is_stance {
            // A true shapeshift, not a stance.
            if self.attributes & ATTR_NOT_SHAPESHIFT != 0 {
                Some(FormRefusal::NotShapeshift)
            } else if self.stances != 0 {
                Some(FormRefusal::OnlyShapeshift)
            } else {
                None
            }
        } else if self.stances != 0 && !self.form_mask_is_permissive() {
            // Unshifted or in a stance: a form requirement holds unless bit 19 waives it.
            Some(FormRefusal::OnlyShapeshift)
        } else {
            None
        }
    }
    /// The name and rank as the client composes them, `"%s (%s)"` (`0x8468b0`), or the bare name
    /// when the rank is empty; the trainer (`0x4d96e0`) and the learn line (`0x4b2982`) use it.
    pub fn ranked_name(&self) -> String {
        match self.rank.as_deref() {
            Some(rank) if !rank.is_empty() => format!("{} ({})", self.name, rank),
            _ => self.name.clone(),
        }
    }

    /// Whether removing this spell prints "You have unlearned %s." (`0x14a`) with the bare name:
    /// `RemoveSpell` (`0x5e9fe0`) is silent for `IS_TRADESKILL` (`0x5ea170`), a nonzero `castUI`
    /// (`0x5ea17a`) or `DO_NOT_DISPLAY` (`0x5ea29b`). The learn line ignores `castUI`. The
    /// caller's suppress flag is clear for `SMSG_REMOVED_SPELL` (`0x5e43e3`), set on a rank-up
    /// (`0x5e6392`).
    pub fn announces_unlearn(&self) -> bool {
        self.attributes & SPELL_ATTR_IS_TRADESKILL == 0
            && self.cast_ui == 0
            && self.attributes & ATTR_DO_NOT_DISPLAY == 0
    }

    /// The chat line a learn prints, off `Attributes` alone (`0x4b2909`): none for `0x80`, the
    /// recipe line for `0x20`, else ability for `0x10` or spell; passive is not read. The block
    /// runs only on a live learn: the `SMSG_INITIAL_SPELLS` replay leaves its flag clear
    /// (`0x5deaa4`); `SMSG_LEARNED_SPELL` (`0x5e61c0`) and a rank-up (`0x4b2f61`) set it.
    pub fn learn_announcement(&self) -> Option<LearnAnnouncement> {
        if self.attributes & ATTR_DO_NOT_DISPLAY != 0 {
            return None;
        }
        if self.attributes & SPELL_ATTR_IS_TRADESKILL != 0 {
            return Some(LearnAnnouncement::Recipe);
        }
        Some(if self.attributes & ATTR_ABILITY != 0 {
            LearnAnnouncement::Ability
        } else {
            LearnAnnouncement::Spell
        })
    }
}

/// Which `ERR_LEARN_*` line a learn prints; all three are system chat lines, not UI errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearnAnnouncement {
    /// `0x37` `ERR_LEARN_SPELL_S`, "You have learned a new spell: %s."
    Spell,
    /// `0x38` `ERR_LEARN_ABILITY_S`, "You have learned a new ability: %s.", for `0x10`.
    Ability,
    /// `0x39` `ERR_LEARN_RECIPE_S`, "You have learned how to create a new item: %s.", for `0x20`;
    /// the name is bare, never ranked.
    Recipe,
}

/// A [`SpellDisplay::form_refusal`] verdict: the cast-fail reason `0x612480` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormRefusal {
    /// `SPELL_FAILED_NOT_SHAPESHIFT` (0x44): a `StancesNot` hit, or a no-shapeshift spell in one.
    NotShapeshift,
    /// `SPELL_FAILED_ONLY_SHAPESHIFT` (0x5e, "Must be in %s"): not in a required form.
    OnlyShapeshift,
}

impl FormRefusal {
    /// The `SPELL_FAILED_*` reason byte, build 12340's numbering.
    pub fn reason(self) -> u8 {
        match self {
            FormRefusal::NotShapeshift => 0x44,
            FormRefusal::OnlyShapeshift => 0x5e,
        }
    }
}

#[path = "display_impls.rs"]
mod impls;

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
