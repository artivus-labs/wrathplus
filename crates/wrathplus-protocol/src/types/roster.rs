//! Character select: the `SMSG_CHAR_ENUM` [`Character`] entry (vmangos `Player::BuildEnumData`
//! order), the create/delete result codes and the create request.

use crate::wire::Vector3d;

/// `WorldResult::CharCreateSuccess` / `CharCreateNameInUse` (`SMSG_CHAR_CREATE`).
pub const CHAR_CREATE_SUCCESS: u8 = 0x2E;
pub const CHAR_CREATE_NAME_IN_USE: u8 = 0x31;
/// `WorldResult::CharCreateServerLimit`: the account already holds `CharactersPerRealm`
/// characters (vmangos `HandleCharCreateOpcode`; 0x2E + 6 in `SharedDefines.h` ResponseCodes).
pub const CHAR_CREATE_SERVER_LIMIT: u8 = 0x34;
/// `WorldResult::CharDeleteSuccess` (`SMSG_CHAR_DELETE`; vmangos `SharedDefines.h` ResponseCodes).
pub const CHAR_DELETE_SUCCESS: u8 = 0x39;
/// Race/Class/Gender values WrathPlus's char-create sends.
pub const RACE_HUMAN: u8 = 0x1;
pub const CLASS_WARRIOR: u8 = 0x1;
pub const GENDER_MALE: u8 = 0x0;

/// A `CMSG_CHAR_CREATE` request for [`super::char_create`]. The outfit id is always 0 on the wire:
/// the server ignores it and picks the start gear (vmangos `CharacterHandler.cpp:310`). The
/// appearance bytes must be valid `CharSections` indices or `Player::ValidateAppearance` refuses.
#[derive(Debug, Clone)]
pub struct CharCreateReq {
    pub name: String,
    /// `ChrRaces.dbc` id (1 Human … 8 Troll).
    pub race: u8,
    /// `ChrClasses.dbc` id (1 Warrior … 11 Druid).
    pub class: u8,
    /// 0 male, 1 female.
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
}

/// The vmangos `CHARACTER_FLAG_*` bits of [`Character::flags`] that the select screen renders.
pub const CHARACTER_FLAG_HIDE_HELM: u32 = 0x0400;
pub const CHARACTER_FLAG_HIDE_CLOAK: u32 = 0x0800;
pub const CHARACTER_FLAG_GHOST: u32 = 0x2000;
pub const CHARACTER_FLAG_RENAME: u32 = 0x4000;

/// One visible-equipment slot: an `ItemDisplayInfo.dbc` id (not an item id, 0 for empty) and the
/// item's `InventoryType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharEnumItem {
    pub display_id: u32,
    pub inventory_type: u8,
}

/// One `SMSG_CHAR_ENUM` roster entry.
#[derive(Debug, Clone)]
pub struct Character {
    pub guid: u64,
    pub name: String,
    /// `ChrRaces.dbc` id (1 Human … 8 Troll).
    pub race: u8,
    /// `ChrClasses.dbc` id (1 Warrior … 11 Druid).
    pub class: u8,
    /// 0 male, 1 female.
    pub gender: u8,
    /// First of the five appearance bytes, all `CharSections` indices.
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub level: u8,
    /// `AreaTable.dbc` id of the character's zone.
    pub zone: u32,
    pub map: u32,
    pub position: Vector3d,
    /// `CHARACTER_FLAG_*` bits.
    pub flags: u32,
    /// Build 12340's `CHAR_CUSTOMIZE_FLAG_*`: `0x1` customize, `0x10000` faction change,
    /// `0x100000` race change (`GetCharacterInfo` `0x4e3170`); 0 on 1.12.
    pub customize_flags: u32,
    /// Equipment order: head, neck, shoulders, shirt, chest, waist, legs, feet, wrists, hands,
    /// finger x2, trinket x2, back, main hand, off hand, ranged, tabard.
    pub equipment: [CharEnumItem; 19],
    /// The pet's `CreatureDisplayInfo.dbc` id, 0 for none. The server already zeroes it for all but
    /// a living hunter or warlock (`Player::BuildEnumData`), so the client needs no class check.
    pub pet_display_id: u32,
    /// The pet's level; `pet_family` is its `CreatureFamily.dbc` id.
    pub pet_level: u32,
    pub pet_family: u32,
}

impl Character {}
