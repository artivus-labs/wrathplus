//! Unit-name resolution. Descriptors carry no names: a player's answers `CMSG_NAME_QUERY` (by
//! guid), a creature's `CMSG_CREATURE_QUERY` (by template entry), a pet's or charm's
//! `CMSG_PET_NAME_QUERY` (by pet number), and which one applies is read off the descriptor as the
//! reference's `GetUnitName` (`0x609210`) reads it. Each key is asked once while in flight, and a
//! negative answer is cached so a bad id never loops; for a player that is an empty name, which the
//! reference evicts instead (`0x55f6f0`), so its next read asks again.
//!
//! Creature templates persist, as the reference's `creaturecache.wdb` does; player and pet names
//! are never written to disk and are cleared at world-session start, as the reference does.

use std::collections::HashMap;

use bevy::prelude::*;

use wrathplus_protocol::guid;

use crate::app_core::session::ObjectStore;
use crate::net::{ClientCommand, NetCommands};
use crate::query_cache::QueryCache;

pub mod type_flags;

/// The name cache: players by guid, creatures by template entry, pets by pet number.
#[derive(Resource, Default)]
pub struct NameCache {
    /// A `None` answer is an empty wire name: the server does not know the guid.
    players: QueryCache<u64, String>,
    /// `(race, class, gender)` from `SMSG_NAME_QUERY_RESPONSE`, the `$`-macro expander's fallback
    /// for an unstreamed subject (`0x506f70`). Present only when the server answered: a name
    /// learned otherwise (our own, seeded at login) has no entry, never a zero triple.
    player_traits: HashMap<u64, (u8, u8, u8)>,
    /// A `None` answer is an entry the server flagged unknown.
    creatures: QueryCache<u32, CreatureRecord>,
    /// No negative entry: Vanilla says nothing for missing pets; Wrath empty replies evict.
    pub pets: QueryCache<u32, String>,
    /// Bumped by every landed answer and the pet-rename eviction, never by an ask: the gated
    /// feeds re-run when it moves.
    generation: u64,
}

/// One cached creature template head.
#[derive(Clone, Debug)]
pub struct CreatureRecord {
    pub name: String,
    pub subname: Option<String>,
    pub creature_type: u32,
    /// `CreatureFamily.dbc` id, 0 for anything not a tameable beast or warlock minion (the table
    /// has no row 0).
    pub pet_family: u32,
    /// Elite rank 0..4 as the template declares it; read it through [`gated_rank`].
    pub rank: u32,
    /// Read bit by bit through [`type_flags`].
    pub type_flags: u32,
    pub civilian: bool,
    /// The tooltip's LEADER line (`0x6125c0`).
    pub racial_leader: bool,
    /// `CreatureDisplayInfo.dbc` id, the first of the template's display ids, 0 when it has none.
    /// The only model for a unit with no world object, such as a stabled pet; an on-screen unit's
    /// own `UNIT_FIELD_DISPLAYID` wins.
    pub display_id: u32,
    /// Wrath's template HealthModifier/PowerModifier, `GetUnitHealthModifier`'s answer for a
    /// non-hunter pet (`0x71b8d0` reads record `+0x34`/`+0x38`); `None` for a 1.12 answer.
    pub modifiers: Option<[f32; 2]>,
}

/// The 1.12 client's creature-rank getter (`0x605620`): 0 before the record is queried and 0 for
/// any unit with a `UNIT_FIELD_PETNUMBER` (a charmed or enslaved unit), else the template rank.
/// `UnitClassification`, the tooltip's ELITE/BOSS word and `UnitLevel`'s world-boss -1 all read
/// through it, so every rank read here goes through this function. No store reads as not-a-pet.
pub fn gated_rank(rec: Option<&CreatureRecord>, store: Option<&ObjectStore>) -> u32 {
    match rec {
        Some(rec) if !store.is_some_and(|s| s.0.unit_is_pet_or_charm()) => rec.rank,
        _ => 0,
    }
}

/// Which cache names a unit and under which key: the branch of the reference's `GetUnitName`
/// (`0x609210`), which the combat log's `GetObjectName` (`0x6264e0`) also calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameKey {
    /// The `OBJECT_FIELD_TYPE` player bit (`0x609232`): the name cache by guid.
    Player(u64),
    /// `UNIT_FIELD_PETNUMBER` non-zero (`0x609295`): the pet-name cache under that number.
    Pet(u32),
    /// `UNIT_FIELD_PETNUMBER` zero (`0x60929d`): the creature record under the descriptor's
    /// `OBJECT_FIELD_ENTRY` (`0x60b160`).
    Creature(u32),
}

impl NameKey {
    /// With a descriptor this is `0x609210` exactly: the pet number and entry are the unit's
    /// fields, not the guid's bits (a vmangos companion pet has a `HIGHGUID_PET` guid but
    /// `PETNUMBER` 0; a charmed creature has a `HIGHGUID_UNIT` guid and a pet number). Without
    /// one, a `HIGHGUID_PET` guid's entry slot is taken as the pet number, right only for a
    /// permanent pet.
    fn of(guid_val: u64, unit: Option<&ObjectStore>) -> Option<Self> {
        if guid::is_player(guid_val) {
            return Some(Self::Player(guid_val));
        }
        if !guid::is_creature_or_pet(guid_val) {
            return None;
        }
        match unit {
            Some(unit) => match unit.0.unit_pet_number() {
                0 => unit
                    .0
                    .object_entry()
                    .filter(|&e| e != 0)
                    .or_else(|| guid::entry(guid_val))
                    .map(Self::Creature),
                n => Some(Self::Pet(n)),
            },
            None => guid::pet_number(guid_val)
                .map(Self::Pet)
                .or_else(|| guid::entry(guid_val).map(Self::Creature)),
        }
    }
}

impl NameCache {
    /// The name for a guid with no descriptor in hand; on a miss, asks once and returns `None`.
    /// A caller holding the unit's [`ObjectStore`] uses [`Self::resolve_unit`]: the guid alone
    /// cannot name a companion pet or a guardian. A game object is `None` without a query.
    pub fn resolve(&self, guid_val: u64, commands: &NetCommands) -> Option<&str> {
        self.resolve_unit(guid_val, None, commands)
    }

    /// The name for a unit keyed off its descriptor, as `0x609210` keys it; asks once on a miss.
    pub fn resolve_unit(
        &self,
        guid_val: u64,
        unit: Option<&ObjectStore>,
        commands: &NetCommands,
    ) -> Option<&str> {
        match NameKey::of(guid_val, unit)? {
            NameKey::Player(guid_val) => {
                if self.players.answered(guid_val) {
                    return self.players.get(guid_val).map(String::as_str);
                }
                let command = ClientCommand::NameQuery { guid: guid_val };
                if !commands.supports(&command) {
                    return None;
                }
                self.players
                    .get_or_ask(guid_val, || {
                        debug!("names: asking player name (guid {guid_val})");
                        let _ = commands.0.send(command);
                    })
                    .map(String::as_str)
            }
            NameKey::Pet(pet_number) => self.resolve_pet(pet_number, guid_val, commands),
            NameKey::Creature(entry) => self.resolve_creature(entry, guid_val, commands),
        }
    }

    /// There is no negative answer: vmangos sends nothing when the guid is not a live pet with
    /// that number (`PetHandler.cpp:190-192`).
    fn resolve_pet(&self, pet_number: u32, guid: u64, commands: &NetCommands) -> Option<&str> {
        if self.pets.answered(pet_number) {
            return self.pets.get(pet_number).map(String::as_str);
        }
        let command = ClientCommand::PetNameQuery { pet_number, guid };
        if !commands.supports(&command) {
            return None;
        }
        self.pets
            .get_or_ask(pet_number, || {
                debug!("names: asking pet name (pet {pet_number})");
                let _ = commands.0.send(command);
            })
            .map(String::as_str)
    }

    /// The name for a creature template `entry`; `guid` is 0 when no spawn is known, since the
    /// server answers by entry alone.
    pub fn resolve_creature(&self, entry: u32, guid: u64, commands: &NetCommands) -> Option<&str> {
        if self.creatures.answered(entry) {
            return self.creatures.get(entry).map(|r| r.name.as_str());
        }
        let command = ClientCommand::CreatureQuery { entry, guid };
        if !commands.supports(&command) {
            return None;
        }
        self.creatures
            .get_or_ask(entry, || {
                debug!("names: asking creature name (entry {entry})");
                let _ = commands.0.send(command);
            })
            .map(|r| r.name.as_str())
    }

    /// A completed player-name query, preserving an empty/unknown answer as `Some("")`.
    /// Callback-driven consumers must distinguish it from `None` (no answer yet), otherwise
    /// one negative answer leaves an entire name-gated roster pending forever. Never asks.
    pub fn answered_player_name(&self, guid: u64) -> Option<&str> {
        self.players
            .answered(guid)
            .then(|| self.players.get(guid).map_or("", String::as_str))
    }

    /// The cached name for `guid`, with no query on a miss.
    pub fn peek(&self, guid_val: u64) -> Option<&str> {
        self.peek_unit(guid_val, None)
    }

    /// The cached name for a unit keyed off its descriptor, with no query on a miss.
    pub fn peek_unit(&self, guid_val: u64, unit: Option<&ObjectStore>) -> Option<&str> {
        match NameKey::of(guid_val, unit)? {
            NameKey::Player(guid_val) => self.players.get(guid_val).map(String::as_str),
            NameKey::Pet(pet_number) => self.pets.get(pet_number).map(String::as_str),
            NameKey::Creature(entry) => self.creatures.get(entry).map(|r| r.name.as_str()),
        }
    }

    /// Record a player-name answer; an empty name is a negative answer. `traits` is `None` for a
    /// name learned without `SMSG_NAME_QUERY_RESPONSE`, and then clears any traits filed for the
    /// guid, since they belonged to the name being replaced.
    pub fn insert_player(&mut self, guid: u64, name: String, traits: Option<(u8, u8, u8)>) {
        match traits.filter(|_| !name.is_empty()) {
            Some(t) => self.player_traits.insert(guid, t),
            None => self.player_traits.remove(&guid),
        };
        self.players
            .insert(guid, (!name.is_empty()).then_some(name));
        self.generation = self.generation.wrapping_add(1);
    }

    /// The reference's world-session wipe (`0x555740`; `0x5557ad call 0x560400` for the player
    /// cache): every `DBCache` whose persistence byte `[cache+0x39]` is 0 is cleared, which is
    /// `'WNAM'` (player names, ctor `0x554cd0`), `'WGLD'`, `'WPNM'` (pet names) and `'WPTN'`.
    /// Called at world entry, before the login seeds our own name.
    pub fn clear_world_session(&mut self) {
        self.players.clear();
        self.player_traits.clear();
        self.pets.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Install the creature templates read off disk, replacing (not merging) the current ones and
    /// touching nothing else: the load can land after the login has seeded our own name, which
    /// must survive it.
    pub fn install_persisted(&mut self, loaded: NameCache) {
        self.creatures = loaded.creatures;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The landed-answer counter.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `(race, class, gender)` for a player guid the name query has answered for.
    pub fn player_traits(&self, guid: u64) -> Option<(u8, u8, u8)> {
        self.player_traits.get(&guid).copied()
    }

    /// Record a pet-name answer, keyed by pet number.
    pub fn insert_pet(&mut self, pet_number: u32, name: String) {
        self.pets.insert(pet_number, Some(name));
        self.generation = self.generation.wrapping_add(1);
    }

    /// Forget a pet's name and its in-flight mark so the next resolve asks again: a rename
    /// arrives only as a bumped `UNIT_FIELD_PET_NAME_TIMESTAMP`, never as the name.
    pub fn forget_pet(&mut self, pet_number: u32) {
        self.pets.evict(pet_number);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Record a creature-name answer; `None` is an unknown entry.
    pub fn insert_creature(&mut self, entry: u32, record: Option<CreatureRecord>) {
        self.creatures.insert(entry, record);
        self.generation = self.generation.wrapping_add(1);
    }

    /// The cached subname, the title line under a creature's name.
    pub fn creature_subname(&self, entry: u32) -> Option<&str> {
        self.creatures.get(entry)?.subname.as_deref()
    }

    /// The cached template record. Key it by the descriptor's `OBJECT_FIELD_ENTRY`, never the
    /// guid's entry slot, as the reference does (`0x60b160`). On `None` each consumer picks its own
    /// default as the reference's getters do: `MORE_AUDIBLE` fails closed (`0x623b70`),
    /// `DO_NOT_PLAY_WOUND_ANIM` fails open (`0x6125f0`).
    pub fn creature_record(&self, entry: u32) -> Option<&CreatureRecord> {
        self.creatures.get(entry)
    }

    /// The cached `CreatureType.dbc` id; the TAB-target scan treats `None` as targetable.
    pub fn creature_type(&self, entry: u32) -> Option<u32> {
        Some(self.creatures.get(entry)?.creature_type)
    }

    /// Forget the in-flight asks a disconnect may have dropped; player and creature names stay,
    /// pet names go with the spawns they named.
    pub fn clear_pending(&mut self) {
        self.players.clear_pending();
        self.creatures.clear_pending();
        self.pets.clear();
    }
}

impl crate::query_cache::AskOnce for NameCache {
    fn clear_pending(&mut self) {
        NameCache::clear_pending(self);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Persistence
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The file's first line; any field that differs on load discards the whole file, the
/// reference's rule for its `.wdb` header (`[FourCC | build 0x16f3 | locale | recordSize |
/// version 1]`, no checksum, no timestamp, no TTL).
const CACHE_MAGIC: &str = "wrathplus-namecache";
/// Our record-layout version; bump it when a column changes. Version 2 holds creature records
/// only.
// 3: creature lines carry the Wrath HealthModifier/PowerModifier columns.
const CACHE_FORMAT: u32 = 3;
/// The client build, the reference's `0x16f3`.
const CACHE_BUILD: u32 = 5875;
/// In the header as in the reference's; WrathPlus reads DBC locale slot 0 only.
const CACHE_LOCALE: &str = "enUS";

impl NameCache {
    /// Serialize as TSV: the header line, then one line per creature record. In-flight asks are
    /// not written.
    pub fn to_tsv(&self, realm: &str) -> String {
        let mut out =
            format!("{CACHE_MAGIC}\t{CACHE_FORMAT}\t{CACHE_BUILD}\t{CACHE_LOCALE}\t{realm}\n");
        for (entry, rec) in self.creatures.iter() {
            match rec {
                Some(r) => out.push_str(&format!(
                    "C\t{entry}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    r.creature_type,
                    r.pet_family,
                    r.rank,
                    r.type_flags,
                    u8::from(r.civilian),
                    u8::from(r.racial_leader),
                    r.display_id,
                    r.name,
                    r.subname.as_deref().unwrap_or(""),
                    // Raw f32 bits, so the round trip is exact; empty is no modifiers.
                    r.modifiers
                        .map(|[h, p]| format!("{}\t{}", h.to_bits(), p.to_bits()))
                        .unwrap_or_default()
                )),
                None => out.push_str(&format!("c\t{entry}\n")),
            }
        }
        out
    }

    /// Rebuild a cache from [`Self::to_tsv`]'s output, `None` when the header differs; a
    /// malformed line is skipped.
    pub fn from_tsv(text: &str, realm: &str) -> Option<Self> {
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next()?.split('\t').collect();
        if header.len() != 5
            || header[0] != CACHE_MAGIC
            || header[1] != CACHE_FORMAT.to_string()
            || header[2] != CACHE_BUILD.to_string()
            || header[3] != CACHE_LOCALE
            || header[4] != realm
        {
            return None;
        }
        let mut cache = NameCache::default();
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            match f.first().copied() {
                Some("C") if f.len() >= 11 => {
                    let Ok(entry) = f[1].parse::<u32>() else {
                        continue;
                    };
                    let (Ok(creature_type), Ok(pet_family), Ok(rank), Ok(type_flags)) =
                        (f[2].parse(), f[3].parse(), f[4].parse(), f[5].parse())
                    else {
                        continue;
                    };
                    let Ok(display_id) = f[8].parse() else {
                        continue;
                    };
                    cache.creatures.insert(
                        entry,
                        Some(CreatureRecord {
                            name: f[9].to_string(),
                            // Empty is no subname, as in the wire decode.
                            subname: (!f[10].is_empty()).then(|| f[10].to_string()),
                            creature_type,
                            pet_family,
                            rank,
                            type_flags,
                            civilian: f[6] == "1",
                            racial_leader: f[7] == "1",
                            display_id,
                            modifiers: match (f.get(11), f.get(12)) {
                                (Some(h), Some(p)) => h
                                    .parse()
                                    .ok()
                                    .zip(p.parse().ok())
                                    .map(|(h, p)| [f32::from_bits(h), f32::from_bits(p)]),
                                _ => None,
                            },
                        }),
                    );
                }
                Some("c") if f.len() >= 2 => {
                    if let Ok(entry) = f[1].parse::<u32>() {
                        cache.creatures.insert(entry, None);
                    }
                }
                _ => {}
            }
        }
        Some(cache)
    }

    /// Drop a player's cached name and traits so the next resolve re-asks: the reference's
    /// remove-by-key for `SMSG_INVALIDATE_PLAYER` (`0x555600` into `0x560170`).
    pub fn invalidate_player(&mut self, guid: u64) {
        self.players.evict(guid);
        self.player_traits.remove(&guid);
        self.generation = self.generation.wrapping_add(1);
    }

    /// How many records the cache holds.
    pub fn len(&self) -> usize {
        self.players.len() + self.creatures.len() + self.pets.len()
    }

    /// No records at all. Clippy's `len_without_is_empty` asks for it once the type is `pub` (the
    /// crate split); only tests call it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The three name-query answers and `SMSG_INVALIDATE_PLAYER`.
pub mod net {
    use bevy::prelude::*;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use super::{CreatureRecord, NameCache};
    use crate::net::NetHandlerApp;

    /// Register the four handlers.
    pub fn register(app: &mut App) {
        use SessionEventKind as K;
        app.add_systems(
            Update,
            drain_native_pet_names
                .after(wrathplus_world::schedule::WorldStage::Net)
                .before(crate::app_core::schedule::UiFeed),
        )
        .net_handler(K::PlayerName, on_player_name)
        .net_handler(K::PetName, on_pet_name)
        .net_handler(K::CreatureName, on_creature_name)
        .net_handler(K::InvalidatePlayer, on_invalidate_player);
    }

    // Base-name display only: the lossless DTO carries timestamp/declensions, but this
    // cache does not model them or infer timestamp-driven refresh. Match native player
    // and creature display conversion (wrath_session::cstring), not transport decoding.
    pub(super) fn drain_native_pet_names(
        packets: Option<ResMut<crate::net::wrath_pet_name_capture::NativePetNamePackets>>,
        mut names: ResMut<NameCache>,
    ) {
        let Some(mut packets) = packets else {
            return;
        };
        for response in packets.0.drain(..) {
            if response.name.is_empty() {
                names.forget_pet(response.pet_number);
            } else {
                names.insert_pet(
                    response.pet_number,
                    String::from_utf8_lossy(&response.name).into_owned(),
                );
            }
        }
    }

    fn on_player_name(In(ev): In<SessionEvent>, mut names: ResMut<NameCache>) {
        if let SessionEvent::PlayerName {
            guid,
            name,
            race,
            class,
            gender,
        } = ev
        {
            player_name(guid, name, race, class, gender, &mut names);
        }
    }

    fn on_pet_name(In(ev): In<SessionEvent>, mut names: ResMut<NameCache>) {
        if let SessionEvent::PetName { pet_number, name } = ev {
            pet_name(pet_number, name, &mut names);
        }
    }

    fn on_creature_name(In(ev): In<SessionEvent>, mut names: ResMut<NameCache>) {
        if let SessionEvent::CreatureName {
            entry,
            name,
            subname,
            creature_type,
            pet_family,
            rank,
            type_flags,
            display_id,
            civilian,
            racial_leader,
            modifiers,
            ..
        } = ev
        {
            creature_name(
                entry,
                name,
                subname,
                creature_type,
                pet_family,
                rank,
                type_flags,
                civilian,
                racial_leader,
                display_id,
                modifiers,
                &mut names,
            );
        }
    }

    fn on_invalidate_player(In(ev): In<SessionEvent>, mut names: ResMut<NameCache>) {
        if let SessionEvent::InvalidatePlayer { guid } = ev {
            invalidate_player(guid, &mut names);
        }
    }

    /// `SMSG_NAME_QUERY_RESPONSE`: the name plus race, class and gender.
    fn player_name(
        guid: u64,
        name: String,
        race: u32,
        class: u32,
        gender: u32,
        names: &mut NameCache,
    ) {
        names.insert_player(guid, name, Some((race as u8, class as u8, gender as u8)));
    }

    /// `SMSG_PET_NAME_QUERY_RESPONSE`, keyed by pet number.
    fn pet_name(pet_number: u32, name: String, names: &mut NameCache) {
        names.insert_pet(pet_number, name);
    }

    /// `SMSG_INVALIDATE_PLAYER`, which vmangos sends after a rename (`CharacterHandler.cpp:839`):
    /// the reference's eviction (`0x555600`) in a name cache with no TTL; its other, an empty-name
    /// answer (`0x55f6f0`), is a cached negative here.
    fn invalidate_player(guid: u64, names: &mut NameCache) {
        debug!("net: invalidating cached name for player {guid:#x}");
        names.invalidate_player(guid);
    }

    /// `SMSG_CREATURE_QUERY_RESPONSE`; a `None` name is the server's "no such entry".
    fn creature_name(
        entry: u32,
        name: Option<String>,
        subname: Option<String>,
        creature_type: Option<u32>,
        pet_family: u32,
        rank: u32,
        type_flags: u32,
        civilian: bool,
        racial_leader: bool,
        display_id: u32,
        modifiers: Option<[u32; 2]>,
        names: &mut NameCache,
    ) {
        names.insert_creature(
            entry,
            name.map(|n| CreatureRecord {
                name: n,
                subname,
                creature_type: creature_type.unwrap_or(0),
                pet_family,
                rank,
                type_flags,
                civilian,
                racial_leader,
                display_id,
                modifiers: modifiers.map(|m| m.map(f32::from_bits)),
            }),
        );
    }
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "names_native_pet_tests.rs"]
pub mod native_pet_tests;
