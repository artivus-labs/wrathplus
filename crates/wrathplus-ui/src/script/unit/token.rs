//! Build 12340's unit-token resolver `0x60abf0`, token to guid, over the model: the base compares
//! in their order, the `target`/`pet` suffix loop `0x60aaa0`, and the name fallback `0x60a630`.
//! It never raises: every failure returns false with the guid 0 (`0x60abd4`, `0x60b021`), and a
//! recognised token that names nobody answers true with [`NOBODY`] (`0x60b00c`).

use super::super::Model;
use super::resolve::{has_prefix, index_of};
use super::UnitState;

/// `0x60b00c`: a recognised, fully parsed token whose guid came out 0. Every verb that reads the
/// guid tests for it (`0x60e7e0`, `0x60e694`, `0x60c44a`); it never equals a real guid, nor the
/// zero leader `UnitIsPartyLeader` compares with.
pub(crate) const NOBODY: u64 = 0xFFFF_FFFF_FFFF_FFFE;
/// The `none` token's guid (`0x60afc9`), all ones: no object holds it.
pub(crate) const NONE: u64 = u64::MAX;

/// `PLAYER_FLAGS` bit 19, which opens the `pet` suffix to any unit (`0x60ab6d`).
const PLAYER_FLAGS_PET_ANY: u32 = 0x8_0000;
/// The name fallback's copy buffer (`0x60a667`, `cmp edi,0x30`).
const NAME_MAX: usize = 0x30;

/// What `0x60abf0` answers: its bool and the guid it leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) ok: bool,
    pub(crate) guid: u64,
}

impl Resolved {
    const FAIL: Resolved = Resolved { ok: false, guid: 0 };

    /// The guid when it can name an object: not 0, [`NOBODY`] or [`NONE`].
    pub(crate) fn unit(self) -> Option<u64> {
        names_unit(self.guid).then_some(self.guid)
    }
}

/// A guid an object can hold.
pub(crate) fn names_unit(guid: u64) -> bool {
    guid != 0 && guid != NOBODY && guid != NONE
}

/// `SStrCmpI` (`0x76ea40`) as an equality: the text up to a NUL, character by character as
/// [`sstr_fold_next`] reads it.
pub(crate) fn sstr_eq_i(a: &[u8], b: &[u8]) -> bool {
    let cut = |s: &[u8]| s.iter().position(|&c| c == 0).unwrap_or(s.len());
    let (mut a, mut b) = (&a[..cut(a)], &b[..cut(b)]);
    while !a.is_empty() || !b.is_empty() {
        let ((ca, ra), (cb, rb)) = (sstr_fold_next(a), sstr_fold_next(b));
        if ca != cb {
            return false;
        }
        (a, b) = (ra, rb);
    }
    true
}

/// `0x76e8d0`: one character, its length from the lead byte (`0xa39618`: 1 below `0xc0`, then 2 to
/// 6), less that length's offset (`0xa395f8`), U+FFFD past U+FFFF, 0 when cut short; then upper
/// case for ASCII, `0xe0`-`0xfe` and œ, and Cyrillic with Ё/ё after Е (`0x76e985`-`0x76ea13`).
fn sstr_fold_next(s: &[u8]) -> (u32, &[u8]) {
    const OFFSET: [u32; 7] = [0, 0, 0x3080, 0xe_2080, 0x3c8_2080, 0xfa08_2080, 0x8208_2080];
    let len = match s.first() {
        None => return (0, s),
        Some(0x00..=0xbf) => 1,
        Some(0xc0..=0xdf) => 2,
        Some(0xe0..=0xef) => 3,
        Some(0xf0..=0xf7) => 4,
        Some(0xf8..=0xfb) => 5,
        Some(_) => 6,
    };
    let Some(bytes) = s.get(..len) else {
        return (0, &s[s.len()..]);
    };
    let c = bytes
        .iter()
        .fold(0u32, |c, &b| (c << 6).wrapping_add(u32::from(b)))
        .wrapping_sub(OFFSET[len]);
    let c = if c > 0xffff { 0xfffd } else { c };
    let folded = match (len, c) {
        (1, 0x61..=0x7a) | (2, 0xe0..=0xfe) => c - 0x20,
        (2, 0x153) => 0x152,
        (2, 0x401 | 0x451) => 0x415,
        (2, 0x410..=0x415) => c - 1,
        (2, 0x430..=0x435) => c - 0x21,
        (2, 0x436..=0x44f) => c - 0x20,
        _ => c,
    };
    (folded, &s[len..])
}

/// `lua_tolstring(L, n, NULL)`: a string, or a number as its string; anything else is NULL, which
/// the ungated verbs hand to the resolver as no token (`0x60ac15`).
pub(crate) use crate::script::binding_abi::optional_string as token_text;

impl Model {
    /// `0x60abf0(token, &guid, 0)`: an empty token is false with the guid untouched, 0 here
    /// (`0x60ac17`-`0x60ac20`, `0x60b04a`).
    pub(crate) fn resolve_token(&self, token: &str) -> Resolved {
        let s = token.as_bytes();
        if s.is_empty() {
            return Resolved::FAIL;
        }
        // A base whose guid is 0 with text after it goes to the names (`0x60ad49`-`0x60ad51`), as
        // does a suffix that does not parse (`0x60ad67`).
        if let Some((guid, rest)) = self.base(s) {
            if guid != 0 || rest.is_empty() {
                if let Some(guid) = self.suffixes(guid, rest) {
                    return done(guid);
                }
            }
        }
        // `0x60afe7`: the original text, its tail through the same suffix loop (`0x60aff9`).
        match self.by_name(s) {
            Some((guid, tail)) => self.suffixes(guid, tail).map_or(Resolved::FAIL, done),
            None => Resolved::FAIL,
        }
    }

    /// The active player's guid (`0x4d3790`): the guid table's, else the party push's.
    pub(crate) fn local_guid(&self) -> u64 {
        match self.unit_guids.player {
            0 => self.party.own_guid,
            me => me,
        }
    }

    /// The guid `token` names when an object can hold it; `None` for nobody.
    pub(crate) fn unit_guid(&self, token: &str) -> Option<u64> {
        self.resolve_token(token).unit()
    }

    /// The snapshot of the unit `guid`: a chain end's, else the push of a token naming it, in the
    /// unit-event order (`0x60bb70`), else any push holding it.
    pub(crate) fn unit_by_guid(&self, guid: u64) -> Option<&UnitState> {
        if !names_unit(guid) {
            return None;
        }
        self.units_by_guid.get(&guid).or_else(|| {
            let tokens = self.unit_guids.event_tokens(guid);
            tokens
                .iter()
                .find_map(|t| self.pushed_unit(t))
                .filter(|u| u.guid == guid)
                .or_else(|| self.units_by_lower.values().find(|u| u.guid == guid))
        })
    }

    /// The base's guid and the text after it, `None` when no base matches: the base compares in
    /// `0x60abf0`'s order, each `_strnicmp` over the literal's length (`0x76e780`) but `npc`,
    /// `questnpc` and `none`, whole-string (count `0x7fffffff`), over the guid table.
    fn base<'t>(&self, s: &'t [u8]) -> Option<(u64, &'t [u8])> {
        let g = &self.unit_guids;
        let at = |table: &[u64], row: u32| {
            usize::try_from(row)
                .ok()
                .and_then(|i| table.get(i))
                .copied()
                .unwrap_or(0)
        };
        let numbered = |len: usize, table: &[u64]| {
            let (row, digits) = index_of(&s[len..]);
            (at(table, row), &s[len + digits..])
        };
        Some(if has_prefix(s, "player") {
            // `[[obj+8]]`, 0 without the player object (`0x60ac4f`).
            (g.player, &s[6..])
        } else if has_prefix(s, "vehicle") {
            (self.vehicle_guid(), &s[7..])
        } else if has_prefix(s, "pet") {
            // `CHARM`, else `SUMMON` (`0x60acc3`-`0x60acd8`); 0 without the player.
            (if g.player == 0 { 0 } else { g.pet }, &s[3..])
        } else if has_prefix(s, "target") {
            (g.target, &s[6..])
        } else if has_prefix(s, "partypet") {
            numbered(8, &g.party_pets)
        } else if has_prefix(s, "party") {
            numbered(5, &g.party)
        } else if has_prefix(s, "raidpet") {
            numbered(7, &g.raid_pets)
        } else if has_prefix(s, "raid") {
            numbered(4, &g.raid)
        } else if has_prefix(s, "boss") {
            // `0x5ed710`'s table (`[0xc24300]`) is the encounter feed's, which is not built: every
            // `bossN` names nobody.
            numbered(4, &[])
        } else if has_prefix(s, "arenapet") {
            numbered(8, &g.arena_pets)
        } else if has_prefix(s, "arena") {
            numbered(5, &g.arena)
        } else if has_prefix(s, "commentator") {
            // `0x56b1e0` on the commentator object (`0xace4a8`), which is not built.
            numbered(11, &[])
        } else if has_prefix(s, "mouseover") {
            // Kept while held (`0x60aefa`) or while `0x512a30` names it ours or a member's.
            let held = g.held.contains_key(&g.mouseover) || self.ours_or_grouped(g.mouseover);
            (if held { g.mouseover } else { 0 }, &s[9..])
        } else if has_prefix(s, "focus") {
            (g.focus, &s[5..])
        } else if s.eq_ignore_ascii_case(b"npc") || s.eq_ignore_ascii_case(b"questnpc") {
            // `questnpc` reads `0x6081b0`'s quest giver, apart from `npc`'s `[0xbd07a8]`; the app
            // pushes the one interaction unit for both.
            (g.npc, &[])
        } else if s.eq_ignore_ascii_case(b"none") {
            (NONE, &[])
        } else {
            return None;
        })
    }

    /// `vehicle` (`0x60ac85`): the player's vehicle while seated on it (`0x4f6250`, link state 3).
    pub(crate) fn vehicle_guid(&self) -> u64 {
        let me = self.unit_guids.player;
        self.wrath_vehicles
            .units
            .get(&me)
            .filter(|v| me != 0 && v.seated)
            .and_then(|v| v.seats.iter().find(|s| s.occupant == Some(me)))
            .map_or(0, |s| s.vehicle)
    }

    /// The suffix loop `0x60aaa0`: one optional `-`, then `target` or `pet`, to the end of the
    /// text; `None` (false, guid zeroed) on anything else or on a `pet` it does not allow.
    fn suffixes(&self, mut guid: u64, mut p: &[u8]) -> Option<u64> {
        let g = &self.unit_guids;
        let me = self.local_guid();
        while !p.is_empty() {
            if p[0] == b'-' {
                p = &p[1..];
            }
            if has_prefix(p, "target") {
                p = &p[6..];
                // The player's target is the target global (`0x60aae3`); another unit's is its
                // `UNIT_FIELD_TARGET`, and a unit not held is 0 (`0x60ab10`), which loops on.
                guid = if guid == me {
                    g.target
                } else {
                    g.held.get(&guid).copied().unwrap_or(0)
                };
            } else if has_prefix(p, "pet") {
                // `0x60ab41`-`0x60ab76`: the player, a group member (`0x512a00`), or any unit
                // while the player's `PLAYER_FLAGS` bit 19 is set.
                let flags = self.pushed_unit("player").map_or(0, |u| u.player_flags);
                let any = me != 0 && flags & PLAYER_FLAGS_PET_ANY != 0;
                if !(guid == me || self.is_member(guid) || any) {
                    return None;
                }
                p = &p[3..];
                guid = self.pet_of(guid);
            } else {
                return None;
            }
        }
        Some(guid)
    }

    /// The player's pet, `CHARM` else `SUMMON` (`0x60acc3`-`0x60acd8`); 0 without the player.
    fn my_pet(&self) -> u64 {
        match self.local_guid() {
            0 => 0,
            _ => self.unit_guids.pet,
        }
    }

    /// A party slot's index (`0xbd1948`).
    fn party_index(&self, guid: u64) -> Option<usize> {
        self.unit_guids.party.iter().position(|&m| m == guid)
    }

    /// A raid row's index (`0xbeb568`).
    fn raid_index(&self, guid: u64) -> Option<usize> {
        self.unit_guids.raid.iter().position(|&m| m == guid)
    }

    /// The pet of party slot `i` or raid row `i`.
    fn member_pet(&self, raid: bool, i: usize) -> u64 {
        let g = &self.unit_guids;
        let table = if raid {
            &g.raid_pets[..]
        } else {
            &g.party_pets[..]
        };
        table.get(i).copied().unwrap_or(0)
    }

    /// `0x512a00`: the player or a party slot (`0x52c680`), or a raid row (`0x5726f0`).
    fn is_member(&self, guid: u64) -> bool {
        guid != 0
            && (guid == self.local_guid()
                || self.party_index(guid).is_some()
                || self.raid_index(guid).is_some())
    }

    /// `0x512a30` = `0x52d310 || 0x573200`: us or our pet; a party member or its pet; a raid
    /// member or its pet.
    pub(crate) fn ours_or_grouped(&self, guid: u64) -> bool {
        names_unit(guid)
            && (self.is_member(guid)
                || guid == self.my_pet()
                || (0..4).any(|i| self.member_pet(false, i) == guid)
                || self.in_raid_or_raid_pet(guid))
    }

    /// `0x573200`: a raid row, or a raid member's pet: the held member's `CHARM`/`SUMMON`
    /// (`0x573255`), else the row's stored pet while the row is online (`[row+0x18] & 1`,
    /// `0x573273`). `raidpetN` (`0x518ce0`) reads the stored pet with no online test.
    pub(crate) fn in_raid_or_raid_pet(&self, guid: u64) -> bool {
        let g = &self.unit_guids;
        let online = |m: u64| self.party.raid.iter().any(|r| r.guid == m && r.online);
        let pet_of_row =
            |(&m, &pet): (&u64, &u64)| pet == guid && (g.held.contains_key(&m) || online(m));
        names_unit(guid)
            && (g.raid.contains(&guid) || g.raid.iter().zip(&g.raid_pets).any(pet_of_row))
    }

    /// A `pet` hop's unit (`0x60ab8e`-`0x60abb1`): the held unit's `CHARM`, else `SUMMON`, 0 when
    /// not held. The app carries it for the player and its group members; any other unit's, which
    /// only the bit-19 arm reaches, reads 0.
    fn pet_of(&self, guid: u64) -> u64 {
        if guid == 0 {
            return 0;
        }
        if guid == self.local_guid() {
            return self.my_pet();
        }
        let held = self.unit_guids.held.contains_key(&guid)
            || self.unit_by_guid(guid).is_some_and(|u| u.has_object);
        if !held {
            return 0;
        }
        let party = self.party_index(guid).map(|i| self.member_pet(false, i));
        let raid = || self.raid_index(guid).map(|i| self.member_pet(true, i));
        party.or_else(raid).unwrap_or(0)
    }

    /// The name fallback `0x60a630`: the text up to `0x30` bytes, cut before a `-` that starts the
    /// whole `-target` or `-pet`, or `-target-` or `-pet-`; any other `-` is kept (a realm). Its
    /// guid and the tail, `None` when no name matches (`0x60aa82`).
    fn by_name<'t>(&self, s: &'t [u8]) -> Option<(u64, &'t [u8])> {
        let mut n = 0;
        while n < s.len() && n < NAME_MAX {
            let r = &s[n..];
            if r[0] == b'-'
                && (r.eq_ignore_ascii_case(b"-target")
                    || r.eq_ignore_ascii_case(b"-pet")
                    || has_prefix(r, "-target-")
                    || has_prefix(r, "-pet-"))
            {
                break;
            }
            n += 1;
        }
        let (name, tail) = s.split_at(n);
        let guid = self.name_guid(name);
        (guid != 0).then_some((guid, tail))
    }

    /// The names `0x60a630` compares ([`sstr_eq_i`], `0x76ea40`), in its order: the player
    /// (`0x60a6f0`), then each raid row if there is a raid, else each party slot, else each arena
    /// slot (`0x60a74c`-`0x60a892`); then, still nobody, our pet (`0x60a8bc`) and the same rows'
    /// pets (`0x60a916`-`0x60aa66`). The members are the roster's rows, each named as `0x60a360`
    /// names it ([`Self::name_of`]).
    /// Single realm: a name carries no `-realm`. 0 for no match.
    fn name_guid(&self, name: &[u8]) -> u64 {
        let (g, party) = (&self.unit_guids, &self.party);
        let is = |n: &str| !n.is_empty() && sstr_eq_i(n.as_bytes(), name);
        let named = |guid: u64| guid != 0 && self.name_of(guid).is_some_and(is);
        let arena = g.arena_count();
        let (raid, slots) = (!party.raid.is_empty(), !party.members.is_empty());
        let rows: Vec<u64> = if raid {
            party.raid.iter().map(|m| m.guid).collect()
        } else if slots {
            party.members.iter().take(4).map(|m| m.guid).collect()
        } else {
            g.arena[..arena].to_vec()
        };
        // The player's name answers its guid and skips the members (`0x60a6f0`-`0x60a739`).
        let me = self.local_guid();
        if me != 0 && (named(me) || is(&self.player_record.name)) {
            return me;
        }
        if let Some(&m) = rows.iter().find(|&&m| named(m)) {
            return m;
        }
        let pet = self.my_pet();
        if named(pet) {
            return pet;
        }
        let pet_of_row = |i: usize| match (raid, slots) {
            (true, _) => self.member_pet(true, i),
            (false, true) => self.member_pet(false, i),
            _ => g.arena_pets.get(i).copied().unwrap_or(0),
        };
        (0..rows.len())
            .map(pet_of_row)
            .find(|&p| named(p))
            .unwrap_or(0)
    }

    /// A unit's name as `0x60a360` reads it, the object's else the name cache's: here a
    /// snapshot's, else the roster row's.
    fn name_of(&self, guid: u64) -> Option<&str> {
        let party = &self.party;
        let rows = party.members.iter().map(|m| (m.guid, m.name.as_str()));
        let mut rows = rows.chain(party.raid.iter().map(|m| (m.guid, m.name.as_str())));
        self.unit_by_guid(guid)
            .and_then(|u| u.name.as_deref())
            .or_else(|| {
                rows.find(|&(g, n)| g == guid && !n.is_empty())
                    .map(|(_, n)| n)
            })
    }
}

/// `0x60b00a`-`0x60b012`: a parse that ends on guid 0 answers true with [`NOBODY`].
fn done(guid: u64) -> Resolved {
    Resolved {
        ok: true,
        guid: if guid == 0 { NOBODY } else { guid },
    }
}
