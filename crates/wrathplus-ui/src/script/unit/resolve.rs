//! What the unit-token resolver reads, which the app pushes as [`UnitGuids`], and a text-only
//! parse of a token's base and `target` hops ([`parse_unit_token`]) for the app's own selection
//! verbs. The VM resolves through build 12340's `0x60abf0` ([`super::token`]), which also takes
//! `pet` suffixes, `none`, `vehicle` and member names. Every compare folds ASCII case
//! (`_strnicmp` through `0x76e780`).

use std::collections::HashMap;

/// What the resolver reads: the guid globals, the group tables and, for each unit a token can
/// reach, whether it is held and whom it targets. A zero guid is nobody.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitGuids {
    /// The local player's guid; 0 without the player object, when `player` and `pet` name nobody
    /// (`0x60ac4f`, `0x60acbf`).
    pub player: u64,
    /// The pet: `UNIT_FIELD_CHARM`, else `UNIT_FIELD_SUMMON` (`0x60acc3`).
    pub pet: u64,
    /// The target global `[0xbd07b0]`.
    pub target: u64,
    /// The mouseover global `[0xbd07a0]`, read only while it is a held unit or ours or a member's
    /// (`0x60aefa`, `0x60af0b`).
    pub mouseover: u64,
    /// The interaction NPC `[0xbd07a8]`.
    pub npc: u64,
    /// `partyN`'s guid, index N-1 (`0x52bc50`, the slots at `0xbd1948`).
    pub party: [u64; 4],
    /// `partypetN`'s guid, index N-1 (`0x52c6e0`).
    pub party_pets: [u64; 4],
    /// `raidN`'s guid, one per roster row; a row past the end is nobody (`0x512a80`).
    pub raid: Vec<u64>,
    /// `raidpetN`'s guid, row for row with [`Self::raid`] (`0x518ce0`).
    pub raid_pets: Vec<u64>,
    /// Wrath's focus unit; 0 for none.
    pub focus: u64,
    /// `arenaN`'s opponent, slot N-1 (`0xbe9f48`); the app's arena feed owns it, not this push.
    pub arena: [u64; 5],
    /// `arenapetN`'s stored pet, slot N-1 (`0xbe9ef8`), the arena feed's too.
    pub arena_pets: [u64; 5],
    /// Whether [`parse_unit_token_in`]'s text parse takes Wrath's bases. The VM sets it; the
    /// app's pushes leave it alone.
    pub wrath: bool,
    /// Each held unit a token can reach, to its `UNIT_FIELD_TARGET` (0 for none): the object
    /// manager's lookup with the unit typemask (`0x4d4db0`, 8) and the descriptor read a `target`
    /// hop makes (`0x60ab16`-`0x60ab24`). A guid missing here is not held.
    pub held: HashMap<u64, u64>,
}

impl UnitGuids {
    /// Every token naming `guid`, in the order build 12340's unit-event fan-out lists them
    /// (`0x60bb70`, which `0x60bf60`/`0x60bf10` walk for each `UNIT_*` event): `player`, `pet`,
    /// the first `partyN`/`partypetN` match, the first `raidN`/`raidpetN` match, `target`, `focus`,
    /// `mouseover`. Not built (labelled): `vehicle`, `arenaN`/`arenapetN` and `bossN`, which these
    /// inputs do not carry. A zero guid names nothing.
    pub fn event_tokens(&self, guid: u64) -> Vec<String> {
        let mut out = Vec::new();
        if guid == 0 {
            return out;
        }
        if self.player == guid {
            out.push("player".into());
        }
        if self.pet == guid {
            out.push("pet".into());
        }
        for i in 0..4 {
            if self.party[i] == guid {
                out.push(format!("party{}", i + 1));
                break;
            }
            if self.party_pets[i] == guid {
                out.push(format!("partypet{}", i + 1));
                break;
            }
        }
        for (i, (&raid, &pet)) in self
            .raid
            .iter()
            .zip(self.raid_pets.iter().chain(std::iter::repeat(&0)))
            .enumerate()
        {
            if raid == guid {
                out.push(format!("raid{}", i + 1));
                break;
            }
            if pet == guid {
                out.push(format!("raidpet{}", i + 1));
                break;
            }
        }
        // `0x60bb70`: the arena slots after the raid rows, over `GetNumArenaOpponents`' count.
        for i in 0..self.arena_count() {
            if self.arena[i] == guid {
                out.push(format!("arena{}", i + 1));
                break;
            }
            if self.arena_pets[i] == guid {
                out.push(format!("arenapet{}", i + 1));
                break;
            }
        }
        for (token, slot) in [
            ("target", self.target),
            ("focus", self.focus),
            ("mouseover", self.mouseover),
        ] {
            if slot == guid {
                out.push(token.into());
            }
        }
        out
    }
}

impl crate::script::UiScript {
    /// [`UnitGuids::event_tokens`] over the resolver inputs the app last pushed.
    pub fn unit_event_tokens(&self, guid: u64) -> Vec<String> {
        self.model_ref().unit_guids.event_tokens(guid)
    }

    /// The arena feed's slots: `arenaN` and `arenapetN`, slot N-1, 0 for none.
    pub fn set_arena_units(&mut self, arena: [u64; 5], pets: [u64; 5]) {
        let mut model = self.model_mut();
        model.unit_guids.arena = arena;
        model.unit_guids.arena_pets = pets;
    }
}

/// A token's base, the resolver's compares in its order (`partypet` before `party`, `raidpet`
/// before `raid`); `npc` is tested apart, as its one full-string compare.
#[derive(Clone, Copy)]
enum Base {
    Player,
    Pet,
    Target,
    PartyPet,
    Party,
    RaidPet,
    Raid,
    Mouseover,
}

/// Each prefix and its base, in the resolver's order; a prefix test compares `strlen(literal)`
/// bytes, so `"playerfoo"` matches `player`.
const PREFIXES: [(&str, Base); 8] = [
    ("player", Base::Player),
    ("pet", Base::Pet),
    ("target", Base::Target),
    ("partypet", Base::PartyPet),
    ("party", Base::Party),
    ("raidpet", Base::RaidPet),
    ("raid", Base::Raid),
    ("mouseover", Base::Mouseover),
];

/// The unit a token starts from, before any `target` hop. A numbered base carries its 0-based
/// row, [`index_of`]'s: a table lookup that fails on the reserved `u32::MAX` of a missing or zero
/// number names nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitBase {
    Player,
    Pet,
    Target,
    Mouseover,
    /// The interaction NPC; an exact compare, so it takes no `target` hop (`0x60af53`).
    Npc,
    Party(u32),
    PartyPet(u32),
    Raid(u32),
    RaidPet(u32),
    /// Wrath's bases.
    Focus,
    Vehicle,
    Boss(u32),
    Arena(u32),
    ArenaPet(u32),
}

/// Wrath's additional bases, after the 1.12 compares; `arenapet` before `arena`.
#[derive(Clone, Copy)]
enum WrathBase {
    Focus,
    Vehicle,
    Boss,
    ArenaPet,
    Arena,
}

const WRATH_PREFIXES: [(&str, WrathBase); 5] = [
    ("focus", WrathBase::Focus),
    ("vehicle", WrathBase::Vehicle),
    ("boss", WrathBase::Boss),
    ("arenapet", WrathBase::ArenaPet),
    ("arena", WrathBase::Arena),
];

/// A token's text as [`parse_unit_token`] reads it, before any state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitTokenParse {
    /// No base matches; 12340 then tries the text as a name (`0x60a630`).
    Unknown,
    /// Recognised, and names nobody whatever the state: the empty token, or text after the base,
    /// and after each hop, that is not `target`.
    Nobody,
    /// A base and the `target` hops after it, each off the current unit's `UNIT_FIELD_TARGET`
    /// (`0x60ab16`); the loop has no depth limit.
    Unit { base: UnitBase, hops: usize },
}

/// `_strnicmp(s, lit, strlen(lit)) == 0`: ASCII fold only, bytes of 0x80 and up exact.
pub(super) fn has_prefix(s: &[u8], lit: &str) -> bool {
    s.len() >= lit.len() && s[..lit.len()].eq_ignore_ascii_case(lit.as_bytes())
}

/// The first prefix the token matches, with its length.
fn base_of(token: &[u8]) -> Option<(Base, usize)> {
    PREFIXES
        .iter()
        .find(|(p, _)| has_prefix(token, p))
        .map(|&(p, b)| (b, p.len()))
}

/// Whether the token's text starts with a base, not whether it names a unit: `"party5"` solo is
/// recognised. The app's verbs that take a token or a name route on it.
pub(crate) fn token_recognised(token: &str, wrath: bool) -> bool {
    token.eq_ignore_ascii_case("npc")
        || (wrath && token.eq_ignore_ascii_case("questnpc"))
        || base_of(token.as_bytes()).is_some()
        || (wrath && wrath_base_of(token.as_bytes()).is_some())
}

fn wrath_base_of(token: &[u8]) -> Option<(WrathBase, usize)> {
    WRATH_PREFIXES
        .iter()
        .find(|(p, _)| has_prefix(token, p))
        .map(|&(p, b)| (b, p.len()))
}

/// The base-10 parse after a numbered base (`0x76f190`), then `-1` to a 0-based index: digits
/// accumulate with 32-bit wrap, and a missing number leaves 0, whose index `0xffffffff` every
/// bound rejects.
pub(super) fn index_of(s: &[u8]) -> (u32, usize) {
    let digits = s.iter().take_while(|b| b.is_ascii_digit()).count();
    let n = s[..digits].iter().fold(0u32, |acc, &b| {
        acc.wrapping_mul(10).wrapping_add(u32::from(b - b'0'))
    });
    (n.wrapping_sub(1), digits)
}

/// Read `token`'s text before any state, the `target`-only subset of `0x60abf0`'s grammar: the
/// base by the compares, a numbered base's row, then `target` hops to the end of the text.
/// `"party1target"` is party1 and one hop, `"targettarget"` the target and one, and `"npctarget"`
/// matches no base, as `npc` is one full-string compare.
pub fn parse_unit_token(token: &str) -> UnitTokenParse {
    parse_unit_token_in(token, false)
}

/// [`parse_unit_token`] in a dialect: `wrath` adds Wrath's bases after the 1.12 compares.
pub fn parse_unit_token_in(token: &str, wrath: bool) -> UnitTokenParse {
    let bytes = token.as_bytes();
    if bytes.is_empty() {
        return UnitTokenParse::Nobody;
    }
    // 2.0's quest frame names the conversation NPC `questnpc` (`QuestFrame.lua`), the unit 1.12
    // calls `npc`.
    if wrath && token.eq_ignore_ascii_case("questnpc") {
        return UnitTokenParse::Unit {
            base: UnitBase::Npc,
            hops: 0,
        };
    }
    if wrath && base_of(bytes).is_none() {
        if let Some((base, len)) = wrath_base_of(bytes) {
            let mut rest = &bytes[len..];
            let mut numbered = |make: fn(u32) -> UnitBase| {
                let (row, digits) = index_of(rest);
                rest = &rest[digits..];
                make(row)
            };
            let base = match base {
                WrathBase::Focus => UnitBase::Focus,
                WrathBase::Vehicle => UnitBase::Vehicle,
                WrathBase::Boss => numbered(UnitBase::Boss),
                WrathBase::ArenaPet => numbered(UnitBase::ArenaPet),
                WrathBase::Arena => numbered(UnitBase::Arena),
            };
            return target_hops(base, rest);
        }
    }
    let Some((base, len)) = base_of(bytes) else {
        return if token.eq_ignore_ascii_case("npc") {
            UnitTokenParse::Unit {
                base: UnitBase::Npc,
                hops: 0,
            }
        } else {
            UnitTokenParse::Unknown
        };
    };
    let mut rest = &bytes[len..];
    let mut numbered = |make: fn(u32) -> UnitBase| {
        let (row, digits) = index_of(rest);
        rest = &rest[digits..];
        make(row)
    };
    let base = match base {
        Base::Player => UnitBase::Player,
        Base::Pet => UnitBase::Pet,
        Base::Target => UnitBase::Target,
        Base::Mouseover => UnitBase::Mouseover,
        Base::PartyPet => numbered(UnitBase::PartyPet),
        Base::Party => numbered(UnitBase::Party),
        Base::RaidPet => numbered(UnitBase::RaidPet),
        Base::Raid => numbered(UnitBase::Raid),
    };
    target_hops(base, rest)
}

/// The `target` hops after a base, to the end of the text.
fn target_hops(base: UnitBase, mut rest: &[u8]) -> UnitTokenParse {
    let mut hops = 0;
    while !rest.is_empty() {
        if !has_prefix(rest, "target") {
            return UnitTokenParse::Nobody;
        }
        rest = &rest[6..];
        hops += 1;
    }
    UnitTokenParse::Unit { base, hops }
}

impl UnitGuids {
    /// The guids a `target` hop can end on: each held unit's target, when it names one. `held`
    /// holds the bases' units and each unit a hop reaches from them, so a token with a hop names
    /// one of these or nobody, and a per-guid feed over them covers the chains a script can spell.
    /// A guid repeats when two units share a target, and the last hop's unit need not be held: the
    /// lookup that finds its object is the reader's.
    pub(crate) fn chain_ends(&self) -> impl Iterator<Item = u64> + '_ {
        self.held.values().copied().filter(|&guid| guid != 0)
    }

    /// `0x549630`: the slots before the first holding neither an opponent nor a pet, which
    /// `GetNumArenaOpponents` (`0x54a2c0`) answers.
    pub(crate) fn arena_count(&self) -> usize {
        (0..5)
            .take_while(|&i| self.arena[i] != 0 || self.arena_pets[i] != 0)
            .count()
    }

    /// `GetNumArenaOpponents()` over the pushed slots.
    pub(crate) fn arena_lua(lua: &mlua::Lua, (): ()) -> mlua::Result<usize> {
        let model = lua
            .app_data_ref::<crate::script::Model>()
            .expect("model app_data");
        Ok(model.unit_guids.arena_count())
    }

    /// `next`, keeping what neither the app's unit push nor its zero default may move: the VM's
    /// dialect flag and the arena feed's slots.
    pub(crate) fn with_kept(&self, next: &UnitGuids) -> UnitGuids {
        UnitGuids {
            wrath: self.wrath,
            arena: self.arena,
            arena_pets: self.arena_pets,
            ..next.clone()
        }
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
