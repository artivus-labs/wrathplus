use super::movement::ObjectType;

mod npc;
pub use npc::{NpcFlags, NpcService};

mod layout;
pub use layout::{Field, FieldSpan};

// Native index anchors for watchers and fixtures, derived from the 12340 layout.
/// `CORPSE_FIELD_DYNAMIC_FLAGS`; bit 0 is lootable insignia. On a unit, index 36 is
/// `UNIT_FIELD_BYTES_0`, so a field edge carries its object class.
pub const FIELD_CORPSE_DYNAMIC_FLAGS: u16 = Field::CorpseDynamicFlags.index();
// UNIT fields. The client reads FLAGS, COMBATREACH, DYNAMIC_FLAGS and NPC_FLAGS at unit block
// bytes 0xa0, 0x1f0, 0x224 and 0x234.
/// The unit's target; the client turns an idle unit to face it, and no packet carries that facing.
/// The unit this one charms (`UpdateFields_1_12_1.h:41`), descriptor byte 0.
/// The unit this one summoned (`UpdateFields_1_12_1.h:42`); on us, the `"pet"` unit. The pet bar
/// reads its guid off `SMSG_PET_SPELLS`, so the two can disagree briefly around a summon.
/// The charmer, 0 for none. The attack-start check `0x612df0` refuses a swing when it is set and
/// is not us (`ERR_ATTACK_CHARMED`); `0x5ee5a0` and `0x5ff580` prefer it as the owner.
/// The summoner of a pet, guardian or totem; with CREATEDBY, the "owned by me" test of `0x5efea0`.
/// The channel's target, possibly the caster (`UpdateFields_1_12_1.h:48`). Public, so another
/// unit's channel renders from it; `MSG_CHANNEL_START`/`UPDATE` reach only the caster.
pub const FIELD_UNIT_HEALTH: u16 = Field::UnitHealth.index();
/// Five power slots: mana, rage, focus, energy, happiness; `MAXPOWER1..5` follow `MAXHEALTH`.
#[cfg(test)]
const FIELD_UNIT_POWER1: u16 = Field::UnitPower.index();
pub const FIELD_UNIT_MAXHEALTH: u16 = Field::UnitMaxHealth.index();
#[cfg(test)]
const FIELD_UNIT_MAXPOWER1: u16 = Field::UnitMaxPower.index();
pub const FIELD_UNIT_LEVEL: u16 = Field::UnitLevel.index();
pub const FIELD_UNIT_FACTIONTEMPLATE: u16 = Field::UnitFactionTemplate.index();
pub const FIELD_UNIT_BYTES_1: u16 = Field::UnitBytes1.index();
pub const FIELD_UNIT_FLAGS: u16 = Field::UnitFlags.index();
/// Aura-state bits, tested as `1 << (state - 1)` by the usable check (client `[unit+0x110]+0x1dc`).
/// Horizontal bounding radius, in yards.
/// The unshifted appearance, untouched by forms, morphs and polymorph. The client sizes the
/// collision height from it (`[unit+0xd0]+0xf8` in `0x725f50`), so a shapeshift keeps it.
/// The ridden mount's `CreatureDisplayInfo` id, 0 when unmounted; this field, not the aura, is the
/// mounted state (`Unit::IsMounted`; client `[unit+0x110]+0x1fc`).
pub const FIELD_UNIT_MOUNTDISPLAYID: u16 = Field::UnitMountDisplayId.index();
/// Nonzero on a pet or charm; the rank getter `0x605620` then forces rank 0, so an enslaved mob
/// shows no elite dragon, tooltip rank word or boss skull.
/// Unix time of the pet's last rename (`Pet.cpp:285`). The name itself comes from
/// `CMSG_PET_NAME_QUERY`, cached by pet number, so a change here invalidates that cache.
/// With the next field, the `(currXP, nextXP)` pair `GetPetExperience` (`0x4be840`) returns; the
/// client reads both unsigned.
/// Two `u16`s: `GetPetTrainingPoints` (`0x4be790`) returns the high word first, which
/// `PetPaperDollFrame.lua` names `totalPoints`, then the low word as `spent`.
pub const FIELD_UNIT_DYNAMIC_FLAGS: u16 = Field::UnitDynamicFlags.index();
/// The spell being channeled, 0 for none (`UpdateFields_1_12_1.h:89`).
pub const FIELD_UNIT_CHANNEL_SPELL: u16 = Field::UnitChannelSpell.index();
/// The summoning spell, 0 for none; the first gate of the client's feed-pet path `0x6ea1e0`.
pub const FIELD_UNIT_NPC_FLAGS: u16 = Field::UnitNpcFlags.index();

/// Aura slots per unit (vmangos `MAX_AURAS`, `SpellAuraDefines.h:25`).
pub const UNIT_AURA_SLOTS: u8 = 48;
/// Slots below this hold buffs, the rest debuffs (`SpellAuraDefines.h:26`). Passives get no slot,
/// so the client renders the array unfiltered (`SpellAuras.cpp:6715-6736`).
pub const UNIT_AURA_POSITIVE_SLOTS: u8 = 32;

/// `AFLAG_CANCELABLE`: set on a positive aura without `SPELL_ATTR_NO_AURA_CANCEL`, the same test
/// the `CMSG_CANCEL_AURA` handler makes (`SpellAuras.cpp:7467`).
pub const AURA_FLAG_CANCELABLE: u8 = 0x01;
/// `AFLAG_EFF_INDEX_0|1|2`, the client's aura liveness test: a cleared slot can keep a stale spell
/// id, so a live aura is one with any of these bits set.
pub const AURA_FLAG_EFF_INDEX_MASK: u8 = 0x0E;

/// One occupied aura slot, from the four `UNIT_FIELD_AURA*` arrays; duration and caster are not
/// on the descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitAuraSlot {
    /// Also the buff or debuff class, and the key `SMSG_UPDATE_AURA_DURATION` uses.
    pub slot: u8,
    pub spell_id: u32,
    /// The raw `UNIT_FIELD_AURAFLAGS` nibble.
    pub flags: u8,
    /// The caster's level at apply time, the only trace of the caster.
    pub level: u8,
    /// At least 1; the wire byte holds `stack - 1`.
    pub stacks: u8,
}

impl UnitAuraSlot {
    /// A buff: the slot is in the positive half.
    pub fn is_helpful(&self) -> bool {
        self.slot < UNIT_AURA_POSITIVE_SLOTS
    }
    /// Whether the server will honour a `CMSG_CANCEL_AURA` for this aura.
    pub fn is_cancelable(&self) -> bool {
        self.flags & AURA_FLAG_CANCELABLE != 0
    }
}

pub const FIELD_PLAYER_QUEST_LOG_1_1: u16 = Field::PlayerQuestLog.index();

/// The indices a field watch names: the same constants the accessors read, so the two agree.
pub mod field {
    /// The bit a watcher tests on a raw `UNIT_DYNAMIC_FLAGS` edge (feign death).
    pub use super::unit::UNIT_DYNFLAG_DEAD;
    pub use super::{
        FIELD_CORPSE_DYNAMIC_FLAGS, FIELD_PLAYER_FIELD_COINAGE, FIELD_PLAYER_FLAGS,
        FIELD_PLAYER_INV_SLOT_HEAD, FIELD_PLAYER_QUEST_LOG_1_1, FIELD_PLAYER_SKILL_INFO_1_1,
        FIELD_UNIT_BYTES_1, FIELD_UNIT_CHANNEL_SPELL, FIELD_UNIT_DYNAMIC_FLAGS,
        FIELD_UNIT_FACTIONTEMPLATE, FIELD_UNIT_FLAGS, FIELD_UNIT_HEALTH, FIELD_UNIT_LEVEL,
        FIELD_UNIT_MAXHEALTH, FIELD_UNIT_MOUNTDISPLAYID, FIELD_UNIT_NPC_FLAGS,
    };
}

/// `MAX_QUEST_LOG_SIZE`, 25 in build 12340 (AzerothCore `QuestDef.h`).
pub const PLAYER_QUEST_LOG_SLOTS: u8 = 25;

/// One `PLAYER_QUEST_LOG` slot, the durable quest state; `SMSG_QUESTUPDATE_*` are only toasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestLogSlot {
    /// 0 for an empty slot.
    pub quest_id: u32,
    /// Four objective counters: Vanilla packs 6 bits each; Wrath packs four u16s. The
    /// client counts item objectives from the bags itself.
    pub counters: [u16; 4],
    /// Byte 3 of the counter field (`Player.h:1107`): a [`quest_slot_state`] bit, 0 in progress.
    pub state: u8,
    /// The absolute end time of a timed quest, else 0.
    pub timer: u32,
}

/// `QUEST_STATE_*` bits of [`QuestLogSlot::state`] (`Player.h:447-451`).
pub mod quest_slot_state {
    pub const COMPLETE: u8 = 0x01;
    pub const FAIL: u8 = 0x02;
}
// Inventory fields. The PLAYER slot arrays are private, and each slot is a 2-field guid.

pub const FIELD_PLAYER_INV_SLOT_HEAD: u16 = Field::PlayerInvSlotHead.index(); // 23 slots × 2 (equipment 0–18, bags 19–22)
pub const FIELD_PLAYER_FIELD_COINAGE: u16 = Field::PlayerCoinage.index(); // copper
                                                                          // The watched reputation slot, signed: slot 0 is a real faction, so only -1 means none.
                                                                          // The rested pool in base kill-XP units: a kill drains it 1:1 while granting +100%
                                                                          // (`Player::GetXPRestBonus`), so the doubled span on the XP bar is twice this value.
                                                                          // EXPLORED_ZONES_1 is the discovery bitset, 4096 bits indexed by `AreaTable.dbc` exploreFlag. Just
                                                                          // below it sit block, dodge, parry and crit chance (FLOAT, UNIT_END + 0x396..0x399).
/// The bitset's slot count (`Size: 128` in AzerothCore's `UpdateFields.h`).
pub const PLAYER_EXPLORED_ZONES_SLOTS: u16 = 128;
// PLAYER stat block. POSSTAT, NEGSTAT and the resistance buff mods are floats in vmangos but go out
// as INT (`Object::BuildValuesUpdate` narrows them). Read them signed: the server's cast of a
// negative float wraps on x86 and saturates to 0 on aarch64.
// (`Player.cpp:3336`, `Player.cpp:7274`)
pub const FIELD_PLAYER_SKILL_INFO_1_1: u16 = Field::PlayerSkillInfo11.index(); // ×384: 128 skills × 3 dwords

// Unspent talent points and free primary professions, `UnitCharacterPoints("player")`.
// Tracking masks the minimap tests: bit `1 << (n - 1)` for creature type or `LockType.dbc` id `n`,
// one bit per active tracking aura.
// Bit 0x10 is PLAYER_FLAGS_GHOST (`Player.h:319`), held by the ghost aura 8326 until resurrection.
pub const FIELD_PLAYER_FLAGS: u16 = Field::PlayerFlags.index();
// The client's player block base `[player+0xe68]` is field 188: the arbiter at +0x0, PLAYER_FLAGS
// at +0x8 and the duel team at +0x20, the three reads of `UnitReaction`'s duel leg (`0x6061e0`).
// Public, so `GetGuildInfo(unit)` (`0x4c9330`, reading block +0xc and +0x10) answers for any
// visible player; a guild id of 0 is guildless.
// `GetComboPoints` (`0x51a190`) reads the target at block +0x838 and the count byte at +0x1029.
// highest honor rank; not the appearance PLAYER_BYTES

/// `PLAYER_SKILL_INFO` slots on the wire, 3 fields each; vmangos fills only 127 (`Player.h:69`).
pub const PLAYER_SKILL_SLOTS: u8 = 128;

/// One `PLAYER_SKILL_INFO` slot, three dwords (`Player.cpp:90-99`): `id | step << 16`,
/// `value | max << 16`, `temp_bonus | perm_bonus << 16` with signed bonuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSkillSlot {
    /// A `SkillLine.dbc` id, 0 for an unused slot.
    pub skill_id: u16,
    /// The tier step, 0 unless a tier-raising effect is live.
    pub step: u16,
    pub value: u16,
    pub max: u16,
    /// From auras, consumables and enchants; a malus is negative.
    pub temp_bonus: i16,
    /// From talents.
    pub perm_bonus: i16,
}

/// A corpse's own appearance, snapshotted at death: the seven `CORPSE_FIELD_BYTES_1`/`_2` bytes
/// the client's dress `0x5d6260` loads, not the owner's live `PLAYER_BYTES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpseLook {
    pub race: u8,
    pub sex: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
}

/// A sparse update-field set: the wire's block mask and a dense value array. A CREATE carries only
/// nonzero fields (`Object::_SetCreateBits`) into the client's zeroed buffer, so an absent field
/// within the created type's descriptor reads `Some(0)`; in a `Values` delta, or past the
/// descriptor's end, it reads `None`.
#[derive(Debug, Clone, Default)]
pub struct ObjectFields {
    /// The wire's block mask, verbatim (`index = word * 32 + bit`).
    present: Vec<u32>,
    /// Always `present.len() * 32` long; an unset slot holds 0 but must be read through the mask.
    values: Vec<u32>,
    /// The created type's [`Field::descriptor_len`], below which absent reads 0; 0 for a delta or fixture.
    descriptor_end: u16,
}

impl ObjectFields {
    /// Marks a CREATE snapshot of `object_type`, so absent fields in its descriptor read 0.
    pub fn into_created(mut self, object_type: ObjectType) -> Self {
        self.descriptor_end = Field::descriptor_len(object_type);
        self
    }

    /// A fixture from `(index, value)` pairs; [`Self::into_created`] makes it a create.
    pub fn from_pairs(pairs: &[(u16, u32)]) -> Self {
        let mut this = Self::default();
        for &(index, value) in pairs {
            this.insert(index, value);
        }
        this
    }

    /// A fixture from semantic `(field, value)` pairs, each the field's first dword. A field the
    /// layout lacks panics.
    pub fn from_fields(pairs: &[(Field, u32)]) -> Self {
        let slots: Vec<_> = pairs
            .iter()
            .map(|&(field, value)| (field, 0, value))
            .collect();
        Self::from_field_slots(&slots)
    }

    /// A fixture from semantic `(field, dword offset, value)` triples: a GUID's high
    /// half, an array member. An offset past the field's width panics.
    pub fn from_field_slots(slots: &[(Field, u16, u32)]) -> Self {
        let pairs: Vec<_> = slots
            .iter()
            .map(|&(field, offset, value)| {
                let span = field
                    .span()
                    .unwrap_or_else(|| panic!("{field:?} is not in the 12340 layout"));
                assert!(offset < span.width, "{field:?}+{offset} is past its width");
                (span.index + offset, value)
            })
            .collect();
        Self::from_pairs(&pairs)
    }

    #[inline]
    pub fn get(&self, field: Field) -> Option<u32> {
        self.get_at(field, 0)
    }

    #[inline]
    pub fn get_at(&self, field: Field, offset: u16) -> Option<u32> {
        let span = field.span()?;
        (offset < span.width).then(|| self.get_u32(span.index + offset))?
    }

    pub fn get_float(&self, field: Field, offset: u16) -> Option<f32> {
        self.get_at(field, offset).map(f32::from_bits)
    }

    pub fn get_field_guid(&self, field: Field, offset: u16) -> Option<u64> {
        let span = field.span()?;
        let hi = offset.checked_add(1)?;
        if hi >= span.width {
            return None;
        }
        // GUID presence follows its low half; sparse updates may omit an unchanged high half.
        self.get_guid(span.index + offset)
    }

    pub fn field_present(&self, field: Field, offset: u16) -> bool {
        field
            .span()
            .is_some_and(|span| offset < span.width && self.contains(span.index + offset))
    }

    /// A complete native resource snapshot: all seven powers and their maxima, or `None`.
    pub fn wrath_powers(&self) -> Option<[(u32, u32); 7]> {
        let mut result = [(0, 0); 7];
        for (slot, pair) in result.iter_mut().enumerate() {
            *pair = (
                self.unit_power(slot as u8)?,
                self.unit_max_power(slot as u8)?,
            );
        }
        Some(result)
    }

    /// `CORPSE_FIELD_OWNER` (`UpdateFields_1_12_1.h:339`): the dead player, only on a corpse.
    pub fn corpse_owner(&self) -> Option<u64> {
        self.get_field_guid(Field::CorpseOwner, 0)
            .filter(|&g| g != 0)
    }

    /// The body's `CreatureDisplayInfo` id, the owner's native display (`Player.cpp:4809`). A bone
    /// pile keeps it, but `0x5d6700` ignores it there and builds the skeleton from race and sex.
    pub fn corpse_display_id(&self) -> Option<u32> {
        self.get(Field::CorpseDisplayId).filter(|&d| d != 0)
    }

    /// The piece in equipment slot `slot` (0..18) as `(ItemDisplayInfo id, InventoryType)`, packed
    /// `display | type << 24` (`Player.cpp:4822`); unlike `PLAYER_VISIBLE_ITEM`, not an item entry.
    pub fn corpse_item(&self, slot: u8) -> Option<(u32, u8)> {
        let raw = self.get_at(Field::CorpseItem, u16::from(slot))?;
        let display = raw & 0x00ff_ffff;
        (display != 0).then_some((display, (raw >> 24) as u8))
    }

    /// The owner's guild id at death, 0 for none; the client builds the corpse's tabard crest from
    /// it (`0x5d6ec0`).
    pub fn corpse_guild(&self) -> u32 {
        self.get(Field::CorpseGuild).unwrap_or(0)
    }

    /// Race, gender and skin in bytes 1..3; byte 0 is unused (`Corpse.cpp:228`).
    fn corpse_bytes_1(&self) -> Option<u32> {
        self.get(Field::CorpseBytes1)
    }
    /// Face, hair style, hair colour and facial hair (`Corpse.cpp:229`).
    fn corpse_bytes_2(&self) -> Option<u32> {
        self.get(Field::CorpseBytes2)
    }
    /// The dead player's race, `CORPSE_FIELD_BYTES_1` byte 1, the one corpse field the reaction
    /// gate reads (`0x5d7120`); 0, which no `ChrRaces` row names, when absent.
    pub fn corpse_race(&self) -> u8 {
        self.corpse_bytes_1().map_or(0, |b1| (b1 >> 8) as u8)
    }
    /// The corpse's appearance, `None` in a delta that lacks either BYTES word.
    pub fn corpse_look(&self) -> Option<CorpseLook> {
        let (b1, b2) = (self.corpse_bytes_1()?, self.corpse_bytes_2()?);
        Some(CorpseLook {
            race: (b1 >> 8) as u8,
            sex: (b1 >> 16) as u8,
            skin: (b1 >> 24) as u8,
            face: b2 as u8,
            hair_style: (b2 >> 8) as u8,
            hair_color: (b2 >> 16) as u8,
            facial_hair: (b2 >> 24) as u8,
        })
    }

    /// `CORPSE_FIELD_FLAGS`, 0 when absent (client `[[corpse+0x110]+0x74]`).
    pub fn corpse_flags(&self) -> u32 {
        self.get(Field::CorpseFlags).unwrap_or(0)
    }
    /// [`Self::corpse_flags`] keeping absence: untouched flags in a delta are `None`, not all
    /// clear. A reader acting on a change must use this one.
    pub fn corpse_flags_present(&self) -> Option<u32> {
        self.get(Field::CorpseFlags)
    }
    /// `CORPSE_FLAG_BONES`: a bone pile, which `0x5d6260` tests first; it wears nothing and takes
    /// its model from race and sex.
    pub fn corpse_is_bones(&self) -> bool {
        self.corpse_flags() & 0x01 != 0
    }
    /// `CORPSE_FLAG_HIDE_HELM`, the owner's setting at death; the client skips slot 0 (`0x5d6465`).
    pub fn corpse_hides_helm(&self) -> bool {
        self.corpse_flags() & 0x08 != 0
    }
    /// `CORPSE_FLAG_HIDE_CLOAK`: the same for slot 14 (`0x5d6470`).
    pub fn corpse_hides_cloak(&self) -> bool {
        self.corpse_flags() & 0x10 != 0
    }

    /// `CORPSE_DYNFLAG_LOOTABLE` (`Map.cpp:3655`): the bone pile has insignia; `0x5d6e20` gates the
    /// loot highlight and the `CMSG_LOOT` click on it.
    pub fn corpse_lootable(&self) -> bool {
        self.get(Field::CorpseDynamicFlags).unwrap_or(0) & 0x01 != 0
    }

    /// `CORPSE_FLAG_LOOTABLE`: the battleground insignia is takeable, by spell 22027 "Remove
    /// Insignia". The cursor `0x482740` and right-click `0x5d6bf0` test it; the corpse's PvP flag
    /// is bit 2 (`UnitIsPVP`, `0x516460`).
    pub fn corpse_pvp_insignia(&self) -> bool {
        self.corpse_flags() & 0x20 != 0
    }

    /// The ground caster (`UpdateFields_1_12_1.h:325`), at the same index as a corpse's owner.
    pub fn dynamicobject_caster(&self) -> Option<u64> {
        self.get_field_guid(Field::DynamicobjectCaster, 0)
            .filter(|&g| g != 0)
    }
    /// The type byte; vmangos sends 1 (area spell) for every persistent-area cast.
    pub fn dynamicobject_bytes(&self) -> Option<u32> {
        self.get(Field::DynamicobjectBytes)
    }
    /// The anchoring spell, the root of the ground-targeted visual chain.
    pub fn dynamicobject_spell_id(&self) -> Option<u32> {
        self.get(Field::DynamicobjectSpellid).filter(|&s| s != 0)
    }
    /// The area's radius in yards, which the server resolves from `SpellRadius.dbc`.
    pub fn dynamicobject_radius(&self) -> Option<f32> {
        self.get_float(Field::DynamicobjectRadius, 0)
    }

    /// Whether the mask carries the field, ignoring the created store's absent-is-zero.
    fn contains(&self, index: u16) -> bool {
        self.present
            .get(usize::from(index / 32))
            .is_some_and(|w| w & (1u32 << (index % 32)) != 0)
    }

    fn get_raw(&self, index: u16) -> Option<u32> {
        self.contains(index)
            .then(|| self.values[usize::from(index)])
    }

    fn insert(&mut self, index: u16, value: u32) {
        // A `u16` index bounds the store at 64 Ki values, whatever the wire says.
        let word = usize::from(index / 32);
        if word >= self.present.len() {
            self.present.resize(word + 1, 0);
            self.values.resize(self.present.len() * 32, 0);
        }
        self.present[word] |= 1u32 << (index % 32);
        self.values[usize::from(index)] = value;
    }

    fn get_guid(&self, index: u16) -> Option<u64> {
        // Raw on purpose: a guid is present iff its low half is, even on a created store.
        let lo = self.get_raw(index)?;
        let hi = self.get_u32(index + 1).unwrap_or(0);
        Some(u64::from(lo) | (u64::from(hi) << 32))
    }

    fn get_u32(&self, index: u16) -> Option<u32> {
        self.get_raw(index)
            .or((index < self.descriptor_end).then_some(0))
    }

    /// Folds in an update: a `Values` delta overlays, since a field going to 0 is sent as 0; a
    /// re-CREATE replaces, as overlaying it would keep fields that have since dropped to 0.
    pub fn merge(&mut self, delta: ObjectFields) {
        self.merge_diff(delta, |_, _, _| {});
    }

    /// [`Self::merge`], calling `changed(index, old, new)` once per changed dword in ascending
    /// order, as the client's `CMirrorHandler` pass does (`0x465330`): a resend of the same value
    /// reports nothing, and an absent old value reads 0. A first create is seeded, never merged.
    pub fn merge_diff(&mut self, delta: ObjectFields, mut changed: impl FnMut(u16, u32, u32)) {
        if delta.descriptor_end != 0 {
            let words = self.present.len().max(delta.present.len());
            for word in 0..words {
                let mask = self.present.get(word).copied().unwrap_or(0)
                    | delta.present.get(word).copied().unwrap_or(0);
                if mask == 0 {
                    continue;
                }
                for bit in 0..32u16 {
                    if mask & (1u32 << bit) == 0 {
                        continue;
                    }
                    let index = word as u16 * 32 + bit;
                    let (old, new) = (
                        self.get_raw(index).unwrap_or(0),
                        delta.get_raw(index).unwrap_or(0),
                    );
                    if old != new {
                        changed(index, old, new);
                    }
                }
            }
            *self = delta;
        } else {
            for (index, value) in delta.raw_fields() {
                let old = self.get_raw(index).unwrap_or(0);
                if old != value {
                    changed(index, old, value);
                }
                self.insert(index, value);
            }
        }
    }

    /// The type this was created as, from its descriptor length (the eight lengths are distinct).
    /// Unlike [`Self::object_type`] it tells a corpse from an item, as index 36 needs.
    pub fn created_as(&self) -> Option<ObjectType> {
        const ALL: [ObjectType; 8] = [
            ObjectType::Object,
            ObjectType::Item,
            ObjectType::Container,
            ObjectType::Unit,
            ObjectType::Player,
            ObjectType::GameObject,
            ObjectType::DynamicObject,
            ObjectType::Corpse,
        ];
        (self.descriptor_end != 0)
            .then(|| {
                ALL.into_iter()
                    .find(|&t| Field::descriptor_len(t) == self.descriptor_end)
            })
            .flatten()
    }

    /// No fields carried; the codec skips an empty `Values` delta.
    pub fn is_empty(&self) -> bool {
        self.present.iter().all(|&w| w == 0)
    }
    /// Every carried `(index, value)` pair in ascending order, for debugging and probes.
    pub fn raw_fields(&self) -> impl Iterator<Item = (u16, u32)> + '_ {
        self.present.iter().enumerate().flat_map(move |(word, &w)| {
            (0..32u16)
                .filter(move |bit| w & (1u32 << bit) != 0)
                .map(move |bit| {
                    let index = word as u16 * 32 + bit;
                    (index, self.values[usize::from(index)])
                })
        })
    }
}

mod player;
mod unit;

pub use unit::{power_display_scale, OwnerFallback};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod accessor_tests;
