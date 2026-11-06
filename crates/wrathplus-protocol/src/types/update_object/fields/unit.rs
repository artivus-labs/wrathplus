//! Named accessors over [`ObjectFields`] for the object scale and the unit block.

use super::*;

/// `UNIT_DYNAMIC_FLAGS` bit 5 (`SharedDefines.h:1158`): reads as a corpse wherever health is
/// shown; set for feign death and appear-dead spawns, with health left intact.
pub const UNIT_DYNFLAG_DEAD: u32 = 0x20;

/// `UNIT_STAND_STATE_DEAD` (`UnitDefines.h:109`), the third leg of `0x605f90`; vmangos never
/// writes it. Both `[vtbl+0xa4]` overrides (`0x60be50`, `0x5ed570`) read `UNIT_FIELD_BYTES_1`
/// byte 0.
const STAND_STATE_DEAD: u8 = 7;

/// Build 12340's raw-to-shown power divisor (`0x7fde00`, table `0xaf5220` = 1, 10, 1, 1, 1000, 1,
/// 10): 10 for rage and runic power, 1000 for pet happiness, 1 for any other type, `POWER_HEALTH`
/// (-2) included (a negative type answers 1). Every power figure a player reads divides by it:
/// `UnitPower`/`UnitMana` (`0x60ed40`), party power, the energize and periodic logs, the drain
/// formatter and the spell cost (`GetSpellInfo` `0x540bde`); happiness thresholds stay raw.
pub fn power_display_scale(ty: u32) -> u32 {
    match ty {
        1 | 6 => 10, // rage, runic power
        4 => 1000,   // happiness
        _ => 1,
    }
}

/// The field [`ObjectFields::unit_owner`] falls back to when `CHARMEDBY` is clear, the one thing
/// the client's two owner readers do differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerFallback {
    /// `UNIT_FIELD_CREATEDBY`, as the own-pet resolver `0x5ee5a0` reads.
    CreatedBy,
    /// `UNIT_FIELD_SUMMONEDBY`, as the `PET_ATTACK_*` callback `0x5ff580` reads.
    SummonedBy,
}

impl ObjectFields {
    /// `OBJECT_FIELD_SCALE_X`: the per-object size multiplier.
    pub fn object_scale_x(&self) -> Option<f32> {
        self.get_float(Field::ObjectScaleX, 0)
    }
    /// `UNIT_FIELD_DISPLAYID` (units + players).
    pub fn unit_displayid(&self) -> Option<i32> {
        self.get(Field::UnitDisplayId).map(|v| v as i32)
    }
    /// `UNIT_FIELD_NATIVEDISPLAYID`: the appearance no shapeshift touches; the collision prism
    /// comes from it, so a druid in bear form keeps the druid's water lines.
    pub fn unit_native_displayid(&self) -> Option<i32> {
        self.get(Field::UnitNativeDisplayId).map(|v| v as i32)
    }
    /// `UNIT_FIELD_MOUNTDISPLAYID`: the mount's `CreatureDisplayInfo` id, the only mounted signal.
    pub fn unit_mount_display_id(&self) -> u32 {
        self.get(Field::UnitMountDisplayId).unwrap_or(0)
    }
    /// `UNIT_FIELD_TARGET`: the guid this unit is attacking or interacting with.
    pub fn unit_target(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitTarget, 0)
            .filter(|&g| g != 0)
    }
    /// `UNIT_FIELD_CHANNEL_OBJECT`: what a channel is aimed at, possibly the caster itself.
    pub fn unit_channel_object(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitChannelObject, 0)
            .filter(|&g| g != 0)
    }
    /// `UNIT_CHANNEL_SPELL`: the spell being channelled, 0 for none.
    pub fn unit_channel_spell(&self) -> u32 {
        self.get(Field::UnitChannelSpell).unwrap_or(0)
    }
    /// `UNIT_FIELD_SUMMON`: the unit this one summoned; on our own descriptor, the `"pet"` unit.
    pub fn unit_summon(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitSummon, 0)
            .filter(|&g| g != 0)
    }
    /// `UNIT_FIELD_CHARM`: the unit this one charms.
    pub fn unit_charm(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitCharm, 0).filter(|&g| g != 0)
    }
    /// A player's pet as the client reads it off the descriptor: `CHARM` if set, else `SUMMON`
    /// (`partypetN`'s `0x4e8204`-`0x4e821a`, the party snapshot's `0x5f0a25`-`0x5f0a39`).
    pub fn unit_pet_guid(&self) -> Option<u64> {
        self.unit_charm().or_else(|| self.unit_summon())
    }
    /// `UNIT_FIELD_SUMMONEDBY`: the summoner of a pet, guardian or totem.
    pub fn unit_summoned_by(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitSummonedby, 0)
            .filter(|&g| g != 0)
    }
    /// `UNIT_FIELD_CHARMEDBY`: whoever is charming this unit.
    pub fn unit_charmed_by(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitCharmedby, 0)
            .filter(|&g| g != 0)
    }
    /// The owner as the client's owner readers compute it: `CHARMEDBY` if set, else the fallback,
    /// which differs by caller (`0x5ee62f` reads `CREATEDBY`, `0x5ff769` `SUMMONEDBY`).
    pub fn unit_owner(&self, fallback: OwnerFallback) -> Option<u64> {
        self.unit_charmed_by().or_else(|| match fallback {
            OwnerFallback::CreatedBy => self.unit_created_by(),
            OwnerFallback::SummonedBy => self.unit_summoned_by(),
        })
    }
    /// `UNIT_FIELD_CREATEDBY`: the creator of a totem or created object.
    pub fn unit_created_by(&self) -> Option<u64> {
        self.get_field_guid(Field::UnitCreatedby, 0)
            .filter(|&g| g != 0)
    }
    /// `UNIT_CREATED_BY_SPELL`: the summoning spell; `0x6ea1e0` requires it to feed a pet.
    pub fn unit_created_by_spell(&self) -> Option<u32> {
        self.get(Field::UnitCreatedBySpell).filter(|&s| s != 0)
    }
    /// `UNIT_FIELD_PETNUMBER` is nonzero: a pet or charm, which the rank getter `0x605620`
    /// reports as rank 0. vmangos writes 0 for guardians and totems (`CharmInfo::SetPetNumber`
    /// without the stat window), so they keep their template's rank.
    pub fn unit_is_pet_or_charm(&self) -> bool {
        self.unit_pet_number() != 0
    }
    /// `UNIT_FIELD_PETNUMBER`: `GetUnitName` (`0x609210`) keys the pet-name cache on it when
    /// nonzero. A `HIGHGUID_PET` guid's number is no substitute: a companion's is never filed in
    /// its charm info, so no pet-name query answers it.
    pub fn unit_pet_number(&self) -> u32 {
        self.get(Field::UnitPetnumber).unwrap_or(0)
    }
    /// `UNIT_FIELD_PET_NAME_TIMESTAMP`: changes when the pet is renamed, staling the name cache.
    pub fn unit_pet_name_timestamp(&self) -> Option<u32> {
        self.get(Field::UnitPetNameTimestamp)
    }
    /// `UNIT_FIELD_HEALTH`: current hit points, 0 when dead.
    pub fn unit_health(&self) -> Option<u32> {
        self.get(Field::UnitHealth)
    }
    /// `UNIT_FIELD_MAXHEALTH`: full hit points.
    pub fn unit_max_health(&self) -> Option<u32> {
        self.get(Field::UnitMaxHealth)
    }
    /// Really dead: health 0 with a known maximum, since a dead unit's create sends no health.
    /// For how the unit looks, which a feigning hunter shares, use [`Self::unit_reads_dead`].
    pub fn unit_is_dead(&self) -> bool {
        self.unit_max_health().unwrap_or(0) > 0 && self.unit_health().unwrap_or(0) == 0
    }
    /// Reads as dead, the reference's `0x605f90`: health 0, `UNIT_DYNFLAG_DEAD` (feign death,
    /// `0x605f9d`) or stand state 7 (`0x605faa`). Feign death sets only the flag
    /// (`Unit.cpp:9491`) and every viewer, the feigner included, receives it.
    pub fn unit_reads_dead(&self) -> bool {
        self.unit_is_dead()
            || self.unit_dynamic_flags() & UNIT_DYNFLAG_DEAD != 0
            || self.unit_stand_state() == STAND_STATE_DEAD
    }
    /// Health as `UnitHealth` (`0x5174d0`) reads it: 0 under `UNIT_DYNFLAG_DEAD`, else raw.
    /// `UnitHealthMax` (`0x5175b0`) has no such gate, so a feigning unit's bar shows empty.
    pub fn unit_shown_health(&self) -> Option<u32> {
        if self.unit_dynamic_flags() & UNIT_DYNFLAG_DEAD != 0 {
            return Some(0);
        }
        self.unit_health()
    }
    /// Power as `UnitMana` (`0x517670`) reads it: 0 under `UNIT_DYNFLAG_DEAD`, else divided by
    /// [`power_display_scale`]. Its `POWER_HEALTH` leg (`0x5176f7`) compares a zero-extended
    /// byte with -2, so it never matches and is left out.
    pub fn unit_shown_power(&self, ty: u8) -> Option<u32> {
        if self.unit_dynamic_flags() & UNIT_DYNFLAG_DEAD != 0 {
            return (ty < self.unit_power_slots()).then_some(0);
        }
        Some(self.unit_power(ty)? / power_display_scale(u32::from(ty)))
    }
    /// Maximum power as `UnitManaMax` (`0x5177e0`) reads it: divided, with no dead gate.
    pub fn unit_shown_max_power(&self, ty: u8) -> Option<u32> {
        Some(self.unit_max_power(ty)? / power_display_scale(u32::from(ty)))
    }
    /// `UNIT_FIELD_LEVEL`: the unit's level.
    pub fn unit_level(&self) -> Option<u32> {
        self.get(Field::UnitLevel)
    }

    /// `UNIT_FIELD_BYTES_0` byte 3: power type, 0 mana, 1 rage, 2 focus, 3 energy, 4 happiness.
    pub fn unit_power_type(&self) -> u8 {
        (self.unit_bytes_0().unwrap_or(0) >> 24) as u8
    }
    /// `UNIT_FIELD_BYTES_1` byte 0: stand state, 0 stand, 1 sit, 2 chair, 3 sleep, 4-6 low,
    /// medium and high chair, 8 kneel; for a player, the echo of `CMSG_STANDSTATECHANGE`.
    pub fn unit_stand_state(&self) -> u8 {
        (self.get(Field::UnitBytes1).unwrap_or(0) & 0xff) as u8
    }
    /// `UNIT_VIRTUAL_ITEM_SLOT_ID + slot`: a creature's held item in slot 0 (main hand), 1 (off
    /// hand) or 2 (ranged), as an item entry the client names through `Item.dbc`; 0 or absent is
    /// an empty hand.
    pub fn unit_virtual_item_entry(&self, slot: u8) -> Option<u32> {
        (slot < 3)
            .then(|| self.get_at(Field::UnitVirtualItemSlotId, u16::from(slot)))?
            .filter(|&entry| entry != 0)
    }
    /// `UNIT_FIELD_PETEXPERIENCE` and `UNIT_FIELD_PETNEXTLEVELEXP`: `GetPetExperience`'s
    /// `(currXP, nextXP)`; absent reads 0, the binding's own failure value, never nil.
    pub fn unit_pet_experience(&self) -> (u32, u32) {
        (
            self.get(Field::UnitPetexperience).unwrap_or(0),
            self.get(Field::UnitPetnextlevelexp).unwrap_or(0),
        )
    }
    /// `UNIT_FIELD_BYTES_1` byte 3 bit `0x2`, `UNIT_VIS_FLAGS_CREEP` (`SpellAuras.cpp:3610`):
    /// stealthed; read by the usable check (`[+0x110]+0x213 & 2`) and the tracker (`0x5ed210`).
    pub fn unit_is_stealthed(&self) -> bool {
        self.unit_visual_flags() & 0x2 != 0
    }

    /// `UNIT_FIELD_BYTES_1` byte 3 bit `0x4` (vmangos `UNIT_VIS_FLAGS_UNTRACKABLE`): no minimap
    /// dot at all, quest or tracking (`byte [eax+0x213] & 4`, the byte's only such test).
    pub fn unit_is_untrackable(&self) -> bool {
        self.unit_visual_flags() & 0x4 != 0
    }

    // AC UnitDefines.h: visibility flags moved from byte 3 to byte 2; byte 3 is anim tier.
    fn unit_visual_flags(&self) -> u8 {
        let shift = 16;
        (self.get(Field::UnitBytes1).unwrap_or(0) >> shift) as u8
    }

    /// `UNIT_FIELD_AURASTATE`: the aura-state bits (1 defense, 2 health under 20%, ...), which
    /// the usable check tests as `1 << (state - 1)`.
    pub fn unit_aura_state(&self) -> u32 {
        self.get(Field::UnitAuraState).unwrap_or(0)
    }

    /// `UNIT_FIELD_BYTES_1` byte 2: the shapeshift form, 0 none, 17-19 the warrior stances, the
    /// druid forms low; the creature-type resolver `0x605570` checks it first.
    pub fn unit_shapeshift_form(&self) -> u8 {
        (self.get(Field::UnitBytes2).unwrap_or(0) >> 24) as u8
    }
    /// Number of native descriptor resources: five in build 5875, seven in build 12340.
    pub fn unit_power_slots(&self) -> u8 {
        Field::UnitPower.span().unwrap().width as u8
    }
    /// `UNIT_FIELD_POWER1..5`: current power of type `ty`, on the wire's raw scale.
    pub fn unit_power(&self, ty: u8) -> Option<u32> {
        self.get_at(Field::UnitPower, u16::from(ty))
    }
    /// `UNIT_FIELD_MAXPOWER1..5`: maximum power of type `ty`, raw.
    pub fn unit_max_power(&self, ty: u8) -> Option<u32> {
        self.get_at(Field::UnitMaxPower, u16::from(ty))
    }
    /// `UNIT_FIELD_BASE_MANA` (field 162, owner only): what `ManaCostPercentage` costs scale from.
    pub fn unit_base_mana(&self) -> Option<u32> {
        self.get(Field::UnitBaseMana)
    }
    /// `UNIT_FIELD_BASE_HEALTH` (field 163, owner only): the basis for percentage health costs.
    pub fn unit_base_health(&self) -> Option<u32> {
        self.get(Field::UnitBaseHealth)
    }
    /// `UNIT_FIELD_FACTIONTEMPLATE` (public, `+0x74`): the `FactionTemplate.dbc` row reactions use.
    pub fn unit_faction_template(&self) -> Option<u32> {
        self.get(Field::UnitFactionTemplate)
    }
    /// `UNIT_FIELD_FLAGS` (`+0xa0`): unit state flags; bit `0x0400_0000` is skinnable.
    pub fn unit_flags(&self) -> u32 {
        self.get(Field::UnitFlags).unwrap_or(0)
    }
    /// Lossless build-12340 UNIT_FIELD_BYTES_2 byte 1, including unknown bits.
    /// Native attack/assist inspect this byte (0x00729740/0x007293d0).
    /// Vanilla has no equivalent packed state byte: returns None, not a projection
    /// of its separate flags. Missing is None; an explicitly zero byte is Some(0).
    pub fn unit_pvp_state_flags(&self) -> Option<u8> {
        self.get(Field::UnitBytes2).map(|word| (word >> 8) as u8)
    }
    /// Direct Wrath sanctuary flag (byte 1 mask 0x08), not combat eligibility.
    /// AzerothCore Unit.h::IsInSanctuary; Vanilla and absent fields return false.
    pub fn unit_is_in_sanctuary(&self) -> bool {
        self.unit_pvp_state_flags()
            .is_some_and(|flags| flags & 0x08 != 0)
    }
    /// Direct replicated PvP state, not a reaction, attack/assist, or pricing policy.
    /// Build 5875 uses UNIT_FLAGS 0x1000; build 12340 uses UNIT_FIELD_BYTES_2
    /// byte 1 mask 0x01 (AzerothCore Unit.h::IsPvP). Missing fields are false.
    pub fn unit_is_pvp(&self) -> bool {
        self.get(Field::UnitBytes2).unwrap_or(0) & 0x0000_0100 != 0
    }
    /// Direct replicated free-for-all PvP state, not effective combat eligibility.
    /// Build 5875 uses PLAYER_FLAGS 0x80; build 12340 uses UNIT_FIELD_BYTES_2
    /// byte 1 mask 0x04 (AzerothCore Unit.h::IsFFAPvP). Missing fields are false;
    /// controlled-unit inheritance and reaction policy belong to callers.
    pub fn unit_is_ffa_pvp(&self) -> bool {
        self.get(Field::UnitBytes2).unwrap_or(0) & 0x0000_0400 != 0
    }
    /// Pet rename permission flag. Build 5875 uses UNIT_FLAGS 0x10;
    /// build 12340 uses UNIT_FIELD_BYTES_2 byte 2 mask 0x01.
    /// Missing fields are false; ownership and action policy are caller concerns.
    pub fn unit_pet_can_be_renamed(&self) -> bool {
        self.get(Field::UnitBytes2).unwrap_or(0) & 0x0001_0000 != 0
    }
    /// Pet abandon permission flag. Build 5875 uses UNIT_FLAGS 0x20;
    /// build 12340 uses UNIT_FIELD_BYTES_2 byte 2 mask 0x02.
    /// Missing fields are false; ownership and action policy are caller concerns.
    pub fn unit_pet_can_be_abandoned(&self) -> bool {
        self.get(Field::UnitBytes2).unwrap_or(0) & 0x0002_0000 != 0
    }
    /// `UNIT_FIELD_COMBATREACH` (`+0x1f0`): melee reach, summed as `rA + rB + 1.333`, floor 5.0,
    /// by the attack range gate (`0x7ff480`); 1.5, the vanilla default, before it streams.
    pub fn unit_combat_reach(&self) -> f32 {
        self.get_float(Field::UnitCombatreach, 0).unwrap_or(1.5)
    }
    /// `UNIT_FIELD_BOUNDINGRADIUS`: horizontal radius in yards; the water-foam depth gate takes
    /// `max(2 * radius, 1.0)` (`0x5fa760`), so absent reads 0.
    pub fn unit_bounding_radius(&self) -> f32 {
        self.get_float(Field::UnitBoundingradius, 0).unwrap_or(0.0)
    }
    /// `UNIT_DYNAMIC_FLAGS` (`+0x224`), per viewer: `0x1` lootable, `0x2` tracked, `0x20` dead.
    pub fn unit_dynamic_flags(&self) -> u32 {
        self.get(Field::UnitDynamicFlags).unwrap_or(0)
    }
    /// `UNIT_DYNAMIC_FLAGS` bit `0x20` alone: `SetSelection`'s last-enemy stamp gates on
    /// `HEALTH > 0 || dynflag 0x20` (`0x49372f`), not on [`Self::unit_reads_dead`].
    pub fn unit_dynflag_dead(&self) -> bool {
        self.unit_dynamic_flags() & UNIT_DYNFLAG_DEAD != 0
    }
    /// `UNIT_DYNAMIC_FLAGS` bit `0x4`: tapped, the only test in `UnitIsTapped` (`0x519c90`).
    pub fn unit_tapped(&self) -> bool {
        self.unit_dynamic_flags() & 0x4 != 0
    }
    /// `UNIT_DYNAMIC_FLAGS` bit `0x8`: tapped by me (`UnitIsTappedByPlayer`, `0x519d00`), which
    /// the server sets per viewer; tapped but not by me is the grey unit frame.
    pub fn unit_tapped_by_player(&self) -> bool {
        self.unit_dynamic_flags() & 0x8 != 0
    }
    /// `UNIT_DYNAMIC_FLAGS` bit `0x1`: lootable by me, stripped per viewer by the server; gates
    /// the loot cursor, right-click looting and the corpse sparkle.
    pub fn unit_lootable(&self) -> bool {
        self.unit_dynamic_flags() & 0x1 != 0
    }
    /// `UNIT_NPC_FLAGS` (`+0x234`, `UnitDefines.h`): gossip `0x1`, questgiver `0x2`, vendor
    /// `0x4`, flight master `0x8`, trainer `0x10`, ..., repair `0x4000`; cursor: first bit wins.
    pub fn unit_npc_flags(&self) -> u32 {
        self.get(Field::UnitNpcFlags).unwrap_or(0)
    }
    /// `UNIT_NPC_EMOTESTATE`: the looping `Emotes.dbc` state, which the server keeps while moving.
    pub fn unit_emote_state(&self) -> u32 {
        self.get(Field::UnitNpcEmotestate).unwrap_or(0)
    }
    /// `UNIT_FIELD_BYTES_0`: race, class, gender and power type, bytes 0 to 3.
    pub fn unit_bytes_0(&self) -> Option<u32> {
        self.get(Field::UnitBytes0)
    }
    /// `UNIT_FIELD_BYTES_2` byte 0: sheath state, 0 stowed, 1 melee drawn, 2 ranged drawn; for a
    /// player, whatever they last sent in `CMSG_SETSHEATHED`.
    pub fn unit_sheath_state(&self) -> Option<u8> {
        self.get(Field::UnitBytes2).map(|v| (v & 0xff) as u8)
    }
    /// `UNIT_FIELD_STAT0 + i`: primary stat `i` after buffs, 0 Str, 1 Agi, 2 Sta, 3 Int, 4 Spi.
    pub fn unit_stat(&self, i: u8) -> Option<u32> {
        (i < 5).then(|| {
            self.get(
                [
                    Field::UnitStat0,
                    Field::UnitStat1,
                    Field::UnitStat2,
                    Field::UnitStat3,
                    Field::UnitStat4,
                ][usize::from(i)],
            )
        })?
    }
    /// `UNIT_FIELD_RESISTANCES + i`: resistance `i`, signed, 0 armor, 1-6 the magic schools.
    pub fn unit_resistance(&self, i: u8) -> Option<i32> {
        (i < 7).then(|| {
            self.get_at(Field::UnitResistances, u16::from(i))
                .map(|v| v as i32)
        })?
    }
    /// `UNIT_FIELD_ATTACK_POWER`: melee attack power before the split pos/neg mods.
    pub fn unit_attack_power(&self) -> Option<i32> {
        self.get(Field::UnitAttackPower).map(|v| v as i32)
    }
    /// `UNIT_FIELD_ATTACK_POWER_MODS`: flat `(positive, negative)` mods, the low and high i16.
    pub fn unit_attack_power_mods(&self) -> (i16, i16) {
        let (lo, hi) = self
            .get(Field::UnitAttackPowerMods)
            .map(|v| (v as u16, (v >> 16) as u16))
            .unwrap_or((0, 0));
        (lo as i16, hi as i16)
    }
    /// `UNIT_FIELD_ATTACK_POWER_MULTIPLIER`: the percent bonus minus 1.0, so 0.0 is no bonus.
    pub fn unit_attack_power_multiplier(&self) -> Option<f32> {
        self.get_float(Field::UnitAttackPowerMultiplier, 0)
    }
    /// `UNIT_FIELD_RANGED_ATTACK_POWER`: ranged attack power before the split pos/neg mods.
    pub fn unit_ranged_attack_power(&self) -> Option<i32> {
        self.get(Field::UnitRangedAttackPower).map(|v| v as i32)
    }
    /// `UNIT_FIELD_RANGED_ATTACK_POWER_MODS`: the ranged `(positive, negative)` flat mods.
    pub fn unit_ranged_attack_power_mods(&self) -> (i16, i16) {
        let (lo, hi) = self
            .get(Field::UnitRangedAttackPowerMods)
            .map(|v| (v as u16, (v >> 16) as u16))
            .unwrap_or((0, 0));
        (lo as i16, hi as i16)
    }
    /// `UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER`: the ranged percent bonus minus 1.0.
    pub fn unit_ranged_attack_power_multiplier(&self) -> Option<f32> {
        self.get_float(Field::UnitRangedAttackPowerMultiplier, 0)
    }
    /// `UNIT_FIELD_MINDAMAGE`: the mainhand weapon's minimum damage.
    pub fn unit_min_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMindamage, 0)
    }
    /// `UNIT_FIELD_MAXDAMAGE`: the mainhand weapon's maximum damage.
    pub fn unit_max_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMaxdamage, 0)
    }
    /// `UNIT_FIELD_MINOFFHANDDAMAGE`: the offhand minimum damage, sent even with no offhand worn.
    pub fn unit_min_offhand_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMinoffhanddamage, 0)
    }
    /// `UNIT_FIELD_MAXOFFHANDDAMAGE`: the offhand weapon's maximum damage.
    pub fn unit_max_offhand_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMaxoffhanddamage, 0)
    }
    /// `UNIT_FIELD_MINRANGEDDAMAGE`: the ranged weapon's minimum damage.
    pub fn unit_min_ranged_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMinrangeddamage, 0)
    }
    /// `UNIT_FIELD_MAXRANGEDDAMAGE`: the ranged weapon's maximum damage.
    pub fn unit_max_ranged_damage(&self) -> Option<f32> {
        self.get_float(Field::UnitMaxrangeddamage, 0)
    }
    /// `UNIT_FIELD_BASEATTACKTIME + i`: attack speed in ms, 0 mainhand, 1 offhand.
    pub fn unit_base_attack_time(&self, i: u8) -> Option<u32> {
        (i < 2).then(|| self.get_at(Field::UnitBaseattacktime, u16::from(i)))?
    }
    /// `UNIT_FIELD_RANGEDATTACKTIME`: ranged weapon attack speed in ms.
    pub fn unit_ranged_attack_time(&self) -> Option<u32> {
        self.get(Field::UnitRangedattacktime)
    }
}
