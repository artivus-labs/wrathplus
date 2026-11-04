//! Group messages: invites, leadership, loot method, roster, member stats, minimap pings,
//! subgroups, raid target icons, ready checks and lockouts (vmangos `Server/Packets/Group.cpp`).
//! Every guid here is a full `u64` except the party member stats subject, which is packed.

use crate::types::update_object::power_display_scale;

/// The raid-assistant bit of a member's flags byte; bits 0-2 are the subgroup
/// (`Group/Group.cpp:1382,1394`).
pub const GROUP_MEMBER_ASSISTANT: u8 = 0x80;

/// vmangos `GroupMemberStatus` (`Group/Group.h:102-111`); bit `0x20` is never set.
pub mod member_status {
    pub const OFFLINE: u8 = 0x00;
    pub const ONLINE: u8 = 0x01;
    pub const PVP: u8 = 0x02;
    pub const DEAD: u8 = 0x04;
    pub const GHOST: u8 = 0x08;
    pub const PVP_FFA: u8 = 0x10;
    pub const AFK: u8 = 0x40;
    pub const DND: u8 = 0x80;
}

/// vmangos `PartyOperation` (`WorldSession.h:94-98`): which command a party result answers.
pub mod party_operation {
    pub const INVITE: u32 = 0;
    pub const LEAVE: u32 = 2;
}

/// vmangos `PartyResult` (`WorldSession.h:100-111`), the verdict of `SMSG_PARTY_COMMAND_RESULT`.
pub mod party_result {
    pub const OK: u32 = 0;
    pub const BAD_PLAYER_NAME: u32 = 1;
    pub const TARGET_NOT_IN_GROUP: u32 = 2;
    pub const GROUP_FULL: u32 = 3;
    pub const ALREADY_IN_GROUP: u32 = 4;
    pub const NOT_IN_GROUP: u32 = 5;
    pub const NOT_LEADER: u32 = 6;
    pub const WRONG_FACTION: u32 = 7;
    pub const IGNORING_YOU: u32 = 8;
}

/// One `SMSG_GROUP_LIST` member row (`Group.cpp:161-167`); the recipient has no row, their flags
/// ride the list's own-flags byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMemberEntry {
    pub name: String,
    pub guid: u64,
    /// [`member_status`] bits (`GetGroupMemberStatus`, `Group/Group.cpp:45-63`).
    pub status: u8,
    /// Subgroup in bits 0-2, plus [`GROUP_MEMBER_ASSISTANT`].
    pub flags: u8,
}

impl GroupMemberEntry {
    /// Whether the row's status byte marks the member online for the record write: a non-zero byte,
    /// the client's own test for a party slot (`0x5e6c3b`) and a raid row (`0x4ba951`), where bit 0
    /// alone is not asked. vmangos sends 0 for an absent member and a byte with bit 0 set for a
    /// present one (`Group/Group.cpp:45-63`), so the two tests agree on its lists.
    pub fn listed_online(&self) -> bool {
        self.status != 0
    }
}

/// The loot tail `SMSG_GROUP_LIST` carries only when it lists members (`Group.cpp:170-179`);
/// `threshold` is an item quality, 2 to 4 in practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupLootInfo {
    /// 0 free-for-all, 1 round-robin, 2 master, 3 group, 4 need before greed (`LootMethod`).
    pub method: u8,
    /// The master looter, 0 unless `method` is 2.
    pub master: u64,
    pub threshold: u8,
}

/// vmangos `GROUP_UPDATE_FLAG_*` (`Group.h:124-151`): the member stats that follow, in bit order.
pub mod party_member_mask {
    pub const STATUS: u32 = 0x0000_0001;
    pub const CUR_HP: u32 = 0x0000_0002;
    pub const MAX_HP: u32 = 0x0000_0004;
    pub const POWER_TYPE: u32 = 0x0000_0008;
    pub const CUR_POWER: u32 = 0x0000_0010;
    pub const MAX_POWER: u32 = 0x0000_0020;
    pub const LEVEL: u32 = 0x0000_0040;
    pub const ZONE: u32 = 0x0000_0080;
    pub const POSITION: u32 = 0x0000_0100;
    pub const AURAS: u32 = 0x0000_0200;
    pub const AURAS_NEGATIVE: u32 = 0x0000_0400;
    pub const PET_GUID: u32 = 0x0000_0800;
    pub const PET_NAME: u32 = 0x0000_1000;
    pub const PET_MODEL_ID: u32 = 0x0000_2000;
    pub const PET_CUR_HP: u32 = 0x0000_4000;
    pub const PET_MAX_HP: u32 = 0x0000_8000;
    pub const PET_POWER_TYPE: u32 = 0x0001_0000;
    pub const PET_CUR_POWER: u32 = 0x0002_0000;
    pub const PET_MAX_POWER: u32 = 0x0004_0000;
    pub const PET_AURAS: u32 = 0x0008_0000;
    pub const PET_AURAS_NEGATIVE: u32 = 0x0010_0000;
}

/// One `SMSG_PARTY_MEMBER_STATS` or `_FULL` payload (`GroupHandler.cpp:590-742`): only the masked
/// fields are present. The plain opcode carries what changed; `_FULL` everything the server has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartyMemberStatsInfo {
    pub status: Option<u8>,
    pub cur_hp: Option<u16>,
    pub max_hp: Option<u16>,
    pub power_type: Option<u8>,
    pub cur_power: Option<u16>,
    pub max_power: Option<u16>,
    pub level: Option<u16>,
    pub zone: Option<u16>,
    /// Raw WoW x and y, truncated to `i16` by the server (`GroupHandler.cpp:625`).
    pub position: Option<(i16, i16)>,
    /// Buff slots 0..31 as `(slot, spell id)`, one per set bit of the wire's `u32` slot mask, in
    /// slot order. A delta's mask names the slots that changed, a 0 id clearing one
    /// (`Player::GetAuraUpdateMask`), so the record is patched slot by slot ([`patch_auras`]).
    pub auras: Option<Vec<(u8, u16)>>,
    /// Debuff slots, likewise, from a `u16` mask; the slot is absolute, 32..47.
    pub auras_negative: Option<Vec<(u8, u16)>>,
    pub pet_guid: Option<u64>,
    pub pet_name: Option<String>,
    pub pet_model_id: Option<u16>,
    pub pet_cur_hp: Option<u16>,
    pub pet_max_hp: Option<u16>,
    pub pet_power_type: Option<u8>,
    pub pet_cur_power: Option<u16>,
    pub pet_max_power: Option<u16>,
    /// The pet's buff and debuff slots, as [`Self::auras`] and [`Self::auras_negative`].
    pub pet_auras: Option<Vec<(u8, u16)>>,
    pub pet_auras_negative: Option<Vec<(u8, u16)>>,
}

/// Apply one aura-block field to a record's (`0x5e52d0`-`0x5e52f8`): each named slot takes its
/// id, the other slots keep theirs, and a 0 empties the slot. The record keeps the occupied slots
/// in ascending order, the order `UnitBuff`'s roster walk reads them in (`0x519780`).
pub fn patch_auras(record: &mut Option<Vec<(u8, u16)>>, patch: &[(u8, u16)]) {
    let slots = record.get_or_insert_with(Vec::new);
    for &(slot, spell) in patch {
        match slots.binary_search_by_key(&slot, |&(s, _)| s) {
            Ok(i) if spell == 0 => {
                slots.remove(i);
            }
            Ok(i) => slots[i].1 = spell,
            Err(i) if spell != 0 => slots.insert(i, (slot, spell)),
            Err(_) => {}
        }
    }
}

impl PartyMemberStatsInfo {
    /// The record of a member new to the roster: 1/1 health and power, so an unseen member shows
    /// full bars until stats arrive (`0x4e833d`), and the roster's online bit.
    pub fn placeholder(online: bool) -> Self {
        Self {
            status: Some(u8::from(online)),
            cur_hp: Some(1),
            max_hp: Some(1),
            cur_power: Some(1),
            max_power: Some(1),
            ..Self::default()
        }
    }

    /// The record's online bit (`[rec+8] & 1`), the bit the readers test (`0x4e8227`, `0x4e816b`,
    /// `0x4e8884`, `0x519741`, `UnitIsConnected`'s `0x517dd3`). A record with no status yet reads
    /// as the zeroed byte of a fresh one, offline.
    pub fn is_online(&self) -> bool {
        self.status.unwrap_or(0) & member_status::ONLINE != 0
    }

    /// Set or clear the record's online bit (`[rec+8] & 1`), the other bits of its status byte as
    /// they were: the write `SMSG_GROUP_LIST` makes to a member it lists, from that row's status
    /// (`0x4e8361`-`0x4e837b`, raid `0x4ba947`-`0x4ba960`). A record with no status yet reads as
    /// the zeroed byte of a fresh one.
    pub fn set_online(&mut self, online: bool) {
        let status = self.status.unwrap_or(0);
        self.status = Some(if online {
            status | member_status::ONLINE
        } else {
            status & !member_status::ONLINE
        });
    }

    /// Snapshot a member's live descriptor as the 1.12 client does (`0x5f0880`) when the member's
    /// object leaves view, just before the stats request, so the frame never reads 0/0. Values are
    /// raw; [`Self::shown_power`] divides at the read. AFK and DND are cleared, as in the client.
    /// `pet` is the object of the member's pet guid ([`ObjectFields::unit_pet_guid`]) when held:
    /// its block is copied, and without one the pet block is emptied (`0x5f0a1f`-`0x5f0b72`).
    /// `pet_name` is that pet's cached name (`0x609210`, copied into `+0x88` at `0x5f0a70`); a
    /// pet whose name the cache does not hold yet keeps the name the record had.
    ///
    /// Deviation: zone and position are kept, not set to the viewer's zone and the object's
    /// position, because nothing in WrathPlus reads them.
    ///
    /// [`ObjectFields::unit_pet_guid`]: crate::types::update_object::ObjectFields::unit_pet_guid
    pub fn snapshot_descriptor(
        &mut self,
        fields: &crate::types::update_object::ObjectFields,
        pet: Option<&crate::types::update_object::ObjectFields>,
        pet_name: Option<&str>,
    ) {
        let mut status = member_status::ONLINE;
        if fields.player_is_ghost() {
            status |= member_status::GHOST;
        }
        // `UNIT_FIELD_FLAGS` PvP bit.
        if fields.unit_flags() & 0x1000 != 0 {
            status |= member_status::PVP;
        }
        // `PLAYER_FLAGS` bit 7 (`0x5f08be`).
        if fields.player_flags() & 0x80 != 0 {
            status |= member_status::PVP_FFA;
        }
        // Raw health, not `unit_reads_dead`: a feigning member reads alive (`0x5f08d3`).
        if fields.unit_health().unwrap_or(0) == 0 {
            status |= member_status::DEAD;
        }
        self.status = Some(status);
        // The record holds `u16`s; the 1.12 client keeps each dword's low word (`0x5f08ea`).
        let power_type = fields.unit_power_type();
        self.cur_hp = Some(fields.unit_health().unwrap_or(0) as u16);
        self.max_hp = Some(fields.unit_max_health().unwrap_or(0) as u16);
        self.power_type = Some(power_type);
        self.cur_power = Some(fields.unit_power(power_type).unwrap_or(0) as u16);
        self.max_power = Some(fields.unit_max_power(power_type).unwrap_or(0) as u16);
        self.level = Some(fields.unit_level().unwrap_or(0) as u16);
        // 12340 descriptors carry no aura block; the record's auras stay empty.
        self.auras = Some(Vec::new());
        self.auras_negative = Some(Vec::new());

        let pet_guid = fields.unit_pet_guid();
        match pet_guid.zip(pet) {
            Some((guid, pet)) => {
                let power_type = pet.unit_power_type();
                self.pet_guid = Some(guid);
                if let Some(name) = pet_name {
                    self.pet_name = Some(name.to_string());
                }
                self.pet_model_id = Some(pet.unit_displayid().unwrap_or(0) as u16);
                self.pet_cur_hp = Some(pet.unit_health().unwrap_or(0) as u16);
                self.pet_max_hp = Some(pet.unit_max_health().unwrap_or(0) as u16);
                self.pet_power_type = Some(power_type);
                self.pet_cur_power = Some(pet.unit_power(power_type).unwrap_or(0) as u16);
                self.pet_max_power = Some(pet.unit_max_power(power_type).unwrap_or(0) as u16);
                self.pet_auras = Some(Vec::new());
                self.pet_auras_negative = Some(Vec::new());
            }
            // No pet object: the block is zero-filled (`0x5f0a0f`), its guid included.
            None => {
                self.pet_guid = None;
                self.pet_name = None;
                self.pet_model_id = None;
                self.pet_cur_hp = None;
                self.pet_max_hp = None;
                self.pet_power_type = None;
                self.pet_cur_power = None;
                self.pet_max_power = None;
                self.pet_auras = None;
                self.pet_auras_negative = None;
            }
        }
    }

    /// `UnitPowerType` from the record, 0 when it has none (the binding's miss value).
    pub fn shown_power_type(&self) -> u8 {
        self.power_type.unwrap_or(0)
    }

    /// `UnitMana` from the record (`0x517744`): the stored power divided by
    /// [`power_display_scale`], as for a live unit.
    pub fn shown_power(&self) -> u32 {
        u32::from(self.cur_power.unwrap_or(0))
            / power_display_scale(u32::from(self.shown_power_type()))
    }

    /// `UnitManaMax` from the record (`0x5178af`), the same divide.
    pub fn shown_max_power(&self) -> u32 {
        u32::from(self.max_power.unwrap_or(0))
            / power_display_scale(u32::from(self.shown_power_type()))
    }

    /// `UnitPowerType` from the pet block (`+0xd8`, `0x5179d2`'s pet leg), 0 when it has none.
    pub fn shown_pet_power_type(&self) -> u8 {
        self.pet_power_type.unwrap_or(0)
    }

    /// `UnitMana` from the pet block (`+0xe0`): the stored power divided by
    /// [`power_display_scale`] of the pet's own power type, as for a live unit.
    pub fn shown_pet_power(&self) -> u32 {
        u32::from(self.pet_cur_power.unwrap_or(0))
            / power_display_scale(u32::from(self.shown_pet_power_type()))
    }

    /// `UnitManaMax` from the pet block (`+0xe2`), the same divide.
    pub fn shown_pet_max_power(&self) -> u32 {
        u32::from(self.pet_max_power.unwrap_or(0))
            / power_display_scale(u32::from(self.shown_pet_power_type()))
    }
}

/// A server `MSG_RAID_TARGET_UPDATE` (`Group.cpp:132-147`): mode 0 is one changed icon, mode 1
/// every set icon, an empty list when none is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaidTargetUpdate {
    Delta { icon: u8, guid: u64 },
    List(Vec<(u8, u64)>),
}

/// A server `MSG_RAID_READY_CHECK`: empty when the leader starts a check, else one member's answer,
/// sent to the leader only (`Group.cpp:94-96`, `126-130`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyCheck {
    Started,
    Answer { guid: u64, ready: u8 },
}

/// `CMSG_LOOT_METHOD` body (`Group.cpp:26-31`): `u32` method, the master looter's full guid
/// (ignored unless the method is 2), `u32` quality threshold.
pub fn loot_method(method: u32, loot_master: u64, threshold: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&method.to_le_bytes());
    body.extend_from_slice(&loot_master.to_le_bytes());
    body.extend_from_slice(&threshold.to_le_bytes());
    body
}

/// Our `MSG_MINIMAP_PING` body: x and y, no guid (`Group.cpp:33-37`).
pub fn minimap_ping(x: f32, y: f32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&x.to_le_bytes());
    body.extend_from_slice(&y.to_le_bytes());
    body
}

/// One `SMSG_RAID_INSTANCE_INFO` row, a permanent raid lockout (vmangos `Player::SendRaidInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaidInstanceEntry {
    /// `Map.dbc` id; the name shown is the client's own lookup.
    pub map: u32,
    /// Seconds until the lockout resets.
    pub reset: u32,
    /// The instance id, `GetSavedInstanceInfo`'s second return.
    pub instance: u32,
}
