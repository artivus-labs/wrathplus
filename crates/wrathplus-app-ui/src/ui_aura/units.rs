//! Every unit's aura list but the player's, one per guid, and the unit-token resolver's inputs
//! (`0x60abf0`, [`UnitGuids`]) that `UnitBuff 0x614ca0` and `UnitDebuff 0x614cf0` read them through:
//! the guid globals, the group tables, and each held unit a `target` chain can reach. A list is the
//! live unit's slots, gated as any unit's; a group member or pet with no live object diverts to its
//! roster record's block (1.12 `0x519592`-`0x5195a4` to `0x519735`, `0x519b25`). Lists are built
//! only for guids some token can name, once per guid whatever names it, and rebuilt only when what
//! they are built from moves: a 40-man raid names up to 80 units, and their targets.

use std::collections::hash_map::{Entry, HashMap};

use wrathplus_formats::SpellCatalog;
use wrathplus_ui::script::{AuraState, UiScript, UnitGuids};

use super::{aura_list, other_unit_inputs, record_inputs, AuraInput};
use crate::app_core::group::GroupState;
use crate::app_core::raid_rows::raid_row_guids;
use crate::app_core::session::ObjectStore;
use crate::app_core::state::PARTY_PET_TOKENS;
use crate::ui_party::{PARTY_TOKENS, RAID_PET_TOKENS, RAID_TOKENS};

/// The tokens `UNIT_AURA` fires for besides `"player"`, in firing order: [`EVENT_UNITS`], then the
/// group's, each resolved by [`event_guid`].
const EVENT_UNITS: [&str; 3] = ["pet", "target", "targettarget"];

const EVENT_TOKEN_COUNT: usize = EVENT_UNITS.len()
    + PARTY_TOKENS.len()
    + PARTY_PET_TOKENS.len()
    + RAID_TOKENS.len()
    + RAID_PET_TOKENS.len();

fn event_tokens() -> impl Iterator<Item = &'static str> {
    EVENT_UNITS
        .into_iter()
        .chain(PARTY_TOKENS)
        .chain(PARTY_PET_TOKENS)
        .chain(RAID_TOKENS)
        .chain(RAID_PET_TOKENS)
}

/// The guid [`event_tokens`]' `i`-th names, 0 for nobody.
fn event_guid(g: &UnitGuids, i: usize) -> u64 {
    let at = |t: &[u64], i: usize| t.get(i).copied().unwrap_or(0);
    match i {
        0 => g.pet,
        1 => g.target,
        // One hop, as the resolver's `target` loop makes it: off a held target only.
        2 => g.held.get(&g.target).copied().unwrap_or(0),
        3..=6 => g.party[i - 3],
        7..=10 => g.party_pets[i - 7],
        11..=50 => at(&g.raid, i - 11),
        _ => at(&g.raid_pets, i - 51),
    }
}

/// The group tables of [`UnitGuids`]: `party1..4` are the own subgroup's slots and `partypetN`
/// their pets (`0x4e81d0`); `raidN` is `GetRaidRosterInfo` row N, as every other raid-token reader
/// resolves it, and `raidpetN` that row's pet (`0x491960`).
fn fill_group<'a>(
    g: &mut UnitGuids,
    group: Option<&GroupState>,
    me: u64,
    held: impl Fn(u64) -> Option<&'a ObjectStore>,
) {
    g.party = [0; 4];
    g.party_pets = [0; 4];
    g.raid.clear();
    g.raid_pets.clear();
    let Some(group) = group else {
        return;
    };
    let live = |m: u64| held(m).map(|s| &s.0);
    for (i, m) in group.party_slots().take(4).enumerate() {
        g.party[i] = m.guid;
        g.party_pets[i] = group.party_pet_guid(m.guid, live(m.guid)).unwrap_or(0);
    }
    g.raid = raid_row_guids(group, Some(me));
    g.raid_pets.extend(
        g.raid
            .iter()
            .map(|&m| group.raid_pet_guid(m, live(m)).unwrap_or(0)),
    );
}

/// What [`UnitAuras::feed`] reads this frame.
pub(super) struct Inputs<'g, 'a, H, V, N> {
    pub(super) group: Option<&'g GroupState>,
    /// Our guid, and whether the player's list moved this frame: a token naming us reads the
    /// player's list, which the caller pushes.
    pub(super) me: (u64, bool),
    /// `pet`, `target`, `mouseover` and `npc`'s guids, 0 for none.
    pub(super) pet: u64,
    pub(super) target: u64,
    pub(super) mouseover: u64,
    pub(super) npc: u64,
    /// The focus guid, 0 for none.
    pub(super) focus: u64,
    /// The object manager's lookup (`0x468460`).
    pub(super) held: H,
    /// `UnitBuff`'s unit gate on a live unit, by guid.
    pub(super) buffs_visible: V,
    pub(super) native_visible: N,
    pub(super) ctx: &'g super::native::ListCtx<'a>,
    /// The spell catalog was replaced, so every list is rebuilt.
    pub(super) catalog_moved: bool,
    pub(super) packets: Option<&'g crate::net::wrath_auras::PacketAuras>,
    pub(super) clocks: (f64, f64),
}

/// One guid's list, shared by every token naming it.
struct GuidList {
    /// What the list was built from; the list is rebuilt only when these move.
    inputs: Vec<AuraInput>,
    list: Vec<AuraState>,
    native: Option<Vec<crate::net::wrath_auras::StampedAura>>,
    native_all_visible: bool,
    /// Taken from [`UnitAuras::versions`] each time the list changes.
    version: u64,
    /// The frame that last read it; an entry no token can name this frame is dropped.
    frame: u64,
}

/// The per-guid lists and the resolver's inputs, for one VM.
#[derive(Default)]
pub(super) struct UnitAuras {
    /// The resolver's inputs as last built.
    guids: UnitGuids,
    by_guid: HashMap<u64, GuidList>,
    /// Each list version the VM holds, by guid.
    pushed: HashMap<u64, u64>,
    /// Each event token's last `(guid, version)`, by [`event_tokens`] index.
    fired: Vec<Option<(u64, u64)>>,
    /// The last version handed out; every list change takes the next, so none repeats.
    versions: u64,
    /// The player's list version, for a token naming us.
    player_version: u64,
    catalog_present: bool,
    frame: u64,
    /// Kept to save the allocations: the inputs being read, and the walk over the guids a token
    /// can name, each with whether its roster record may stand in for it.
    scratch: Vec<AuraInput>,
    walk: Vec<(u64, bool)>,
}

impl UnitAuras {
    /// Build the resolver's inputs and every nameable guid's list, push what moved, and return
    /// the tokens `UNIT_AURA` fires for: every event token naming a unit whose list moved
    /// (`0x515e50`), or that names a new unit. A token clearing fires nothing.
    pub(super) fn feed<'a, H, V, N>(
        &mut self,
        script: &mut UiScript,
        inputs: Inputs<'_, 'a, H, V, N>,
    ) -> Vec<&'static str>
    where
        H: Fn(u64) -> Option<&'a ObjectStore>,
        V: Fn(u64, &ObjectStore) -> bool,
        N: Fn(u64, Option<&ObjectStore>, &[crate::net::wrath_auras::StampedAura]) -> bool,
    {
        let Inputs {
            group,
            me: (me, player_moved),
            pet,
            target,
            mouseover,
            npc,
            focus,
            held,
            buffs_visible,
            native_visible,
            ctx,
            catalog_moved,
            packets,
            clocks,
        } = inputs;
        self.frame += 1;
        if catalog_moved || ctx.catalog.is_some() != self.catalog_present {
            self.catalog_present = ctx.catalog.is_some();
            self.by_guid.clear();
        }
        if player_moved || self.player_version == 0 {
            self.versions += 1;
            self.player_version = self.versions;
        }

        // The resolver's inputs and the lists, in one walk over every guid a token can name: the
        // bases, then each held unit their `target` chains reach. A group token's guid lists off
        // its roster record while out of view; any other lists only while held.
        let mut g = std::mem::take(&mut self.guids);
        let mut walk = std::mem::take(&mut self.walk);
        (g.player, g.pet, g.target, g.mouseover, g.npc) = (me, pet, target, mouseover, npc);
        g.focus = focus;
        fill_group(&mut g, group, me, &held);
        g.held.clear();
        walk.clear();
        walk.extend([me, pet, target, mouseover, npc, focus].map(|g| (g, false)));
        walk.extend(g.party.iter().chain(&g.party_pets).map(|&g| (g, true)));
        walk.extend(g.raid.iter().chain(&g.raid_pets).map(|&g| (g, true)));
        while let Some((guid, record_ok)) = walk.pop() {
            if guid == 0 || g.held.contains_key(&guid) {
                continue;
            }
            let store = held(guid);
            if let Some(unit) = store.filter(|s| s.is_unit()) {
                let next = unit.0.unit_target().unwrap_or(0);
                g.held.insert(guid, next);
                walk.push((next, false));
            }
            if guid != me {
                if store.is_none() && record_ok {
                    if let Some(native) = group.and_then(|g| g.wrath_aura_slots(guid)) {
                        let stamps = native
                            .into_iter()
                            .map(|(slot, spell_id, _, flags)| {
                                crate::net::wrath_auras::StampedAura {
                                    aura: wrathplus_protocol::wrath_auras::NativeAura {
                                        slot,
                                        spell_id,
                                        flags,
                                        level: 0,
                                        stacks: 1,
                                        caster_guid: Some(0),
                                        max_duration_ms: None,
                                        remaining_ms: None,
                                    },
                                    received_at: 0.0,
                                }
                            })
                            .collect::<Vec<_>>();
                        self.refresh_native(
                            guid,
                            &stamps,
                            native_visible(guid, store, &stamps),
                            ctx,
                            clocks,
                        );
                        continue;
                    }
                }
                if let Some(auras) = packets.and_then(|p| p.by_guid.get(&guid)) {
                    self.refresh_native(
                        guid,
                        auras,
                        native_visible(guid, store, auras),
                        ctx,
                        clocks,
                    );
                } else {
                    self.refresh(guid, store, record_ok, group, &buffs_visible, ctx.catalog);
                }
            }
        }
        script.set_unit_guids(&g);
        self.guids = g;
        self.walk = walk;
        let frame = self.frame;
        self.by_guid.retain(|_, e| e.frame == frame);

        for (&guid, e) in &self.by_guid {
            if self.pushed.insert(guid, e.version) != Some(e.version) {
                script.set_unit_auras(guid, Some(e.list.clone()));
            }
        }
        let by_guid = &self.by_guid;
        self.pushed.retain(|guid, _| {
            let keep = by_guid.contains_key(guid);
            if !keep {
                script.set_unit_auras(*guid, None);
            }
            keep
        });

        self.fired.resize(EVENT_TOKEN_COUNT, None);
        let mut fire = Vec::new();
        for (i, token) in event_tokens().enumerate() {
            let guid = event_guid(&self.guids, i);
            let cur = match guid {
                0 => None,
                g if g == me => Some((g, self.player_version)),
                g => self.by_guid.get(&g).map(|e| (g, e.version)),
            };
            if cur != self.fired[i] && cur.is_some() {
                fire.push(token);
            }
            self.fired[i] = cur;
        }
        fire
    }

    fn refresh_native(
        &mut self,
        guid: u64,
        auras: &[crate::net::wrath_auras::StampedAura],
        all_visible: bool,
        ctx: &super::native::ListCtx,
        clocks: (f64, f64),
    ) {
        if let Some(e) = self.by_guid.get_mut(&guid) {
            e.frame = self.frame;
            if e.native.as_deref() == Some(auras) && e.native_all_visible == all_visible {
                // The packet and gate cache the expensive row construction, not the book or
                // object manager: a caster may stream in, despawn or change owners independently.
                let mut changed = false;
                for state in &mut e.list {
                    changed |= super::native::refresh_context(ctx, state);
                }
                if changed {
                    self.versions += 1;
                    e.version = self.versions;
                }
                return;
            }
        }
        self.versions += 1;
        self.by_guid.insert(
            guid,
            GuidList {
                inputs: Vec::new(),
                native: Some(auras.to_vec()),
                native_all_visible: all_visible,
                list: super::native::list(ctx, guid, auras, all_visible, clocks.0, clocks.1),
                version: self.versions,
                frame: self.frame,
            },
        );
    }

    /// `guid`'s list as last built, `None` for a unit with none.
    pub(super) fn list(&self, guid: u64) -> Option<&[AuraState]> {
        self.by_guid.get(&guid).map(|e| e.list.as_slice())
    }

    /// Rebuild `guid`'s list this frame if what it is built from moved: the live unit's gated
    /// slots (`store`, the object manager's answer), else, for a guid a group token names, the
    /// roster record's block. A guid neither held nor a group token's gets no list, as
    /// `UnitBuff`'s roster walk finds nothing for it.
    fn refresh(
        &mut self,
        guid: u64,
        store: Option<&ObjectStore>,
        record_ok: bool,
        group: Option<&GroupState>,
        buffs_visible: &impl Fn(u64, &ObjectStore) -> bool,
        catalog: Option<&SpellCatalog>,
    ) {
        if self
            .by_guid
            .get(&guid)
            .is_some_and(|e| e.frame == self.frame)
        {
            return;
        }
        self.scratch.clear();
        match store {
            Some(store) => self.scratch.extend(other_unit_inputs(
                store,
                catalog,
                buffs_visible(guid, store),
            )),
            None if record_ok => {
                if let Some(group) = group {
                    if let Some(native) = group.wrath_aura_slots(guid) {
                        self.scratch.extend(native);
                    } else {
                        self.scratch
                            .extend(record_inputs(group.roster_aura_slots(guid), catalog));
                    }
                }
            }
            None => return,
        }
        match self.by_guid.entry(guid) {
            Entry::Occupied(mut e) => {
                let e = e.get_mut();
                e.frame = self.frame;
                if e.native.take().is_some() || e.inputs != self.scratch {
                    std::mem::swap(&mut e.inputs, &mut self.scratch);
                    let list = aura_list(catalog, &e.inputs);
                    if list != e.list {
                        e.list = list;
                        self.versions += 1;
                        e.version = self.versions;
                    }
                }
            }
            Entry::Vacant(e) => {
                self.versions += 1;
                let inputs = self.scratch.clone();
                let list = aura_list(catalog, &inputs);
                e.insert(GuidList {
                    inputs,
                    native: None,
                    native_all_visible: true,
                    list,
                    version: self.versions,
                    frame: self.frame,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    include!("units_tests.rs");
}
