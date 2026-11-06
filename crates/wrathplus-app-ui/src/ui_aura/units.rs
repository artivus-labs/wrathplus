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
    use bevy::prelude::*;
    use wrathplus_protocol::types::Field;
    use wrathplus_protocol::types::{
        member_status, GroupMemberEntry, ObjectFields, PartyMemberStatsInfo,
    };
    use wrathplus_ui::script::UiScript;

    use crate::app_core::group::GroupState;
    use crate::app_core::session::{ObjectStore, SelfPlayer};
    use crate::app_core::state::ClientState;
    use crate::net::{Guid, GuidIndex, NetCommands, Reputations};

    #[test]
    fn unchanged_native_snapshot_rebuilds_when_visibility_changes() {
        let mut cache = super::UnitAuras::default();
        let aura = crate::net::wrath_auras::StampedAura {
            aura: wrathplus_protocol::wrath_auras::NativeAura {
                slot: 0,
                spell_id: 123,
                flags: 1,
                level: 80,
                stacks: 1,
                caster_guid: None,
                max_duration_ms: None,
                remaining_ms: None,
            },
            received_at: 0.0,
        };
        let (snapshot, bare) = ([aura], super::super::native::bare(None));
        cache.refresh_native(7, &snapshot, true, &bare, (0.0, 0.0));
        assert_eq!(cache.list(7).unwrap().len(), 1);
        let version = cache.versions;
        cache.refresh_native(7, &snapshot, true, &bare, (1.0, 1.0));
        assert_eq!(cache.versions, version);
        cache.refresh_native(7, &snapshot, false, &bare, (1.0, 1.0));
        assert!(cache.list(7).unwrap().is_empty());
        cache.refresh_native(7, &snapshot, true, &bare, (2.0, 2.0));
        assert_eq!(cache.list(7).unwrap().len(), 1);
    }

    #[test]
    fn native_feed_rechecks_catalog_and_context_without_packet_changes() {
        use crate::net::wrath_auras::{PacketAuras, StampedAura};
        let mut cache = super::UnitAuras::default();
        let mut script = UiScript::new().unwrap();
        let packets = PacketAuras {
            by_guid: std::collections::HashMap::from([(
                MEMBER,
                vec![StampedAura {
                    aura: wrathplus_protocol::wrath_auras::NativeAura {
                        slot: 0,
                        spell_id: 123,
                        flags: 1,
                        level: 80,
                        stacks: 1,
                        caster_guid: None,
                        max_duration_ms: None,
                        remaining_ms: None,
                    },
                    received_at: 0.0,
                }],
            )]),
        };
        let catalog =
            wrathplus_formats::SpellCatalog::from_displays(std::collections::HashMap::from([(
                123,
                wrathplus_formats::SpellDisplay {
                    effect_apply_aura: [295, 0, 0],
                    ..Default::default()
                },
            )]));
        let mut feed =
            |catalog: Option<&wrathplus_formats::SpellCatalog>, moved, reaction, local, flags| {
                cache.feed(
                    &mut script,
                    super::Inputs {
                        group: None,
                        me: (ME, false),
                        pet: 0,
                        target: MEMBER,
                        mouseover: 0,
                        npc: 0,
                        focus: 0,
                        held: |_| None,
                        buffs_visible: |_, _: &ObjectStore| true,
                        native_visible: |_, _: Option<&ObjectStore>, auras: &[StampedAura]| {
                            super::super::native::all_visible(
                                super::super::native::has_active_aura_295(catalog, auras),
                                local,
                                reaction,
                                flags,
                            )
                        },
                        ctx: &super::super::native::bare(catalog),
                        catalog_moved: moved,
                        packets: Some(&packets),
                        clocks: (0.0, 0.0),
                    },
                );
                cache.list(MEMBER).unwrap().len()
            };
        assert_eq!(feed(None, false, 3, ME, 0), 1); // availability fallback
        assert_eq!(feed(Some(&catalog), true, 3, ME, 0), 0); // same packet, catalog arrives
        assert_eq!(feed(Some(&catalog), false, 4, ME, 0), 1);
        assert_eq!(feed(Some(&catalog), false, 3, ME, 0), 0);
        assert_eq!(feed(Some(&catalog), false, 3, ME, 0x04000000), 1);
        assert_eq!(feed(Some(&catalog), false, 3, 0, 0), 1);
    }

    const ME: u64 = 0x10;
    const MEMBER: u64 = 0x1234;
    const PET: u64 = 0xF140_0000_0000_0077;
    const SUMMON: u16 = Field::UnitSummon.index();
    /// `UNIT_FIELD_FLAGS`; bit 27 is `UNIT_FLAG_AURAS_VISIBLE`, which opens `UnitBuff`'s gate.
    const FLAGS: u16 = Field::UnitFlags.index();

    /// The feed on a bare app with a VM, us, and a party of one, `MEMBER`, with no record.
    fn party_app() -> App {
        let (tx, _) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(ClientState::InWorld)
            .init_resource::<crate::app_core::state::Selection>()
            .init_resource::<crate::app_core::state::PetBar>()
            .init_resource::<GuidIndex>()
            .init_resource::<Reputations>()
            .init_resource::<GroupState>()
            .insert_resource(NetCommands(tx))
            .add_plugins(super::super::UiAuraPlugin);
        app.insert_non_send_resource(UiScript::new().unwrap());
        app.world_mut()
            .non_send_resource::<UiScript>()
            .run(
                r#"
                SEEN = {}
                local f = CreateFrame("Frame")
                f:RegisterEvent("UNIT_AURA")
                -- The group's tokens only: the player's list fires on the first frame too.
                f:SetScript("OnEvent", function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9)
                    if arg1 ~= "player" then table.insert(SEEN, arg1) end
                end)
            "#,
            )
            .unwrap();
        let me = app
            .world_mut()
            .spawn((ObjectStore(ObjectFields::default()), Guid(ME), SelfPlayer))
            .id();
        app.world_mut().resource_mut::<GuidIndex>().0.insert(ME, me);
        let member = GroupMemberEntry {
            name: "Brisca".into(),
            guid: MEMBER,
            status: member_status::ONLINE,
            flags: 0,
        };
        app.world_mut().resource_mut::<GroupState>().apply_list(
            0,
            0,
            vec![member],
            ME,
            None,
            Some(ME),
        );
        app
    }

    #[test]
    fn packet_snapshots_override_projected_fields_for_player_and_party_and_clear() {
        use crate::net::wrath_auras::{PacketAuras, StampedAura};
        use wrathplus_protocol::wrath_auras::NativeAura;
        let mut app = party_app();
        stream(&mut app, MEMBER, &[]);
        let aura = StampedAura {
            received_at: 0.0,
            aura: NativeAura {
                slot: 63,
                spell_id: 123,
                flags: 0x10,
                level: 80,
                stacks: 2,
                caster_guid: Some(ME),
                max_duration_ms: Some(30000),
                remaining_ms: Some(29000),
            },
        };
        let mut packets = PacketAuras::default();
        packets.by_guid.insert(ME, vec![aura.clone()]);
        packets.by_guid.insert(MEMBER, vec![aura]);
        app.insert_resource(packets);
        app.update();
        assert_eq!(listed(&mut app, "player", false), [123]);
        assert_eq!(listed(&mut app, "party1", false), [123]);
        app.world_mut()
            .resource_mut::<PacketAuras>()
            .by_guid
            .insert(ME, vec![]);
        app.world_mut()
            .resource_mut::<PacketAuras>()
            .by_guid
            .insert(MEMBER, vec![]);
        app.update();
        assert!(listed(&mut app, "player", false).is_empty());
        assert!(listed(&mut app, "party1", false).is_empty());
    }

    /// `guid`'s native aura snapshot, `(slot, spell, helpful)` per aura.
    fn packet(app: &mut App, guid: u64, auras: &[(u8, u32, bool)]) {
        crate::net::wrath_auras::set_signed_for_test(app.world_mut(), guid, auras);
    }

    /// Stream `guid` in with these fields, as the object manager holds it.
    fn stream(app: &mut App, guid: u64, fields: &[(u16, u32)]) {
        let e = app
            .world_mut()
            .spawn(ObjectStore(ObjectFields::from_pairs(fields)))
            .id();
        app.world_mut()
            .resource_mut::<GuidIndex>()
            .0
            .insert(guid, e);
    }

    fn stats(app: &mut App, full: bool, info: PartyMemberStatsInfo) {
        stats_of(app, MEMBER, full, info);
    }

    fn stats_of(app: &mut App, guid: u64, full: bool, info: PartyMemberStatsInfo) {
        app.world_mut()
            .resource_mut::<GroupState>()
            .apply_stats(guid, full, info);
    }

    /// The spell ids of `token`'s pushed debuffs (`harmful`) or buffs, after checking that
    /// `UnitDebuff` (or `UnitBuff`) enumerates exactly that many, index 1 up to the nil.
    fn listed(app: &mut App, token: &str, harmful: bool) -> Vec<i64> {
        let verb = if harmful { "UnitDebuff" } else { "UnitBuff" };
        let s = app.world_mut().non_send_resource::<UiScript>();
        let ids = s.aura_spell_ids(token, !harmful);
        let enumerated = s
            .eval::<usize>(&format!(
                r#"local n = 0
                   for i = 1, 48 do
                       local _, _, _, count = {verb}("{token}", i)
                       if count == nil then break end
                       n = n + 1
                   end
                   return n"#
            ))
            .unwrap();
        assert_eq!(
            enumerated,
            ids.len(),
            "{verb}(\"{token}\", i) enumerates the list"
        );
        ids.into_iter().map(i64::from).collect()
    }

    fn seen(app: &mut App) -> Vec<String> {
        app.world_mut()
            .non_send_resource::<UiScript>()
            .eval::<Vec<String>>("local s = SEEN SEEN = {} return s")
            .unwrap()
    }

    /// A live member reads its own packet auras; `UnitBuff` keeps the unit gate, open here
    /// through `UNIT_FLAG_AURAS_VISIBLE` (`0x5195bc`).
    #[test]
    fn a_live_members_packet_auras_win_over_its_record() {
        let mut app = party_app();
        stream(&mut app, MEMBER, &[(FLAGS, 0x0800_0000)]);
        packet(
            &mut app,
            MEMBER,
            &[(0, 1126, true), (33, 589, false), (34, 11976, false)],
        );
        // A record that disagrees, to show the live object wins.
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                auras_negative: Some(vec![(40, 770)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", true), [589, 11976]);
        assert_eq!(listed(&mut app, "party1", false), [1126]);
        assert!(
            listed(&mut app, "party2", true).is_empty(),
            "no second member"
        );
        assert_eq!(seen(&mut app), ["party1"]);
    }

    /// With no live object, both bindings walk the record's block (`0x519735`, `0x519b25`), with
    /// no unit gate, a stack count of 1 (`0x51980f`), and nil for an offline record (`0x519741`).
    #[test]
    fn an_out_of_range_member_reads_the_stats_aura_block() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                auras: Some(vec![(0, 1126), (3, 21562)]),
                auras_negative: Some(vec![(33, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(
            listed(&mut app, "party1", false),
            [1126, 21562],
            "buffs, ungated: no factions, which would close the live gate"
        );
        assert_eq!(listed(&mut app, "party1", true), [589]);
        let count = app
            .world_mut()
            .non_send_resource::<UiScript>()
            .eval::<i64>(r#"local _, _, _, count = UnitDebuff("party1", 1) return count"#)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(seen(&mut app), ["party1"]);

        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                status: Some(member_status::OFFLINE),
                ..Default::default()
            },
        );
        app.update();
        assert!(listed(&mut app, "party1", false).is_empty());
        assert!(listed(&mut app, "party1", true).is_empty());
    }

    /// The list writes the record's online bit too (`0x4e8361`-`0x4e837b`), so a status-only
    /// `SMSG_GROUP_LIST` for a member out of view takes their last auras: `UnitBuff` and
    /// `UnitDebuff` answer nil behind a clear bit (`0x519741`, `0x519b31`), and an online list brings them back.
    #[test]
    fn a_status_only_list_takes_an_out_of_view_members_auras_and_an_online_one_returns_them() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                auras: Some(vec![(0, 1126)]),
                auras_negative: Some(vec![(33, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", false), [1126]);
        assert_eq!(listed(&mut app, "party1", true), [589]);

        let relist = |app: &mut App, status: u8| {
            let member = GroupMemberEntry {
                name: "Brisca".into(),
                guid: MEMBER,
                status,
                flags: 0,
            };
            app.world_mut().resource_mut::<GroupState>().apply_list(
                0,
                0,
                vec![member],
                ME,
                None,
                Some(ME),
            );
            app.update();
        };
        relist(&mut app, member_status::OFFLINE);
        let nil = app
            .world_mut()
            .non_send_resource::<UiScript>()
            .eval::<bool>(
                r#"return UnitBuff("party1", 1) == nil and UnitDebuff("party1", 1) == nil"#,
            )
            .unwrap();
        assert!(nil, "the record kept the auras, the clear bit hides them");
        assert!(listed(&mut app, "party1", false).is_empty());
        assert!(listed(&mut app, "party1", true).is_empty());

        relist(&mut app, member_status::ONLINE);
        assert_eq!(listed(&mut app, "party1", false), [1126]);
        assert_eq!(listed(&mut app, "party1", true), [589]);
    }

    /// The record is slot-indexed: deltas patch named slots, the halves split at slot 32, and each
    /// walk is ascending slot whatever order the slots filled in.
    #[test]
    fn record_auras_list_in_slot_order_and_split_buffs_from_debuffs() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                auras: Some(vec![(5, 1126)]),
                auras_negative: Some(vec![(40, 770)]),
                ..Default::default()
            },
        );
        app.update();
        let _ = seen(&mut app);
        // A later aura in a lower slot, then a debuff, as two deltas.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras: Some(vec![(2, 21562)]),
                ..Default::default()
            },
        );
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras_negative: Some(vec![(34, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", false), [21562, 1126]);
        assert_eq!(listed(&mut app, "party1", true), [589, 770]);
        assert_eq!(seen(&mut app), ["party1"]);

        // Slot 5 falls off: the delta names it with a 0, and only it goes.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras: Some(vec![(5, 0)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", false), [21562]);
        assert_eq!(listed(&mut app, "party1", true), [589, 770]);
        assert_eq!(seen(&mut app), ["party1"]);

        // A delta that moves no aura fires nothing.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(7),
                ..Default::default()
            },
        );
        app.update();
        assert!(seen(&mut app).is_empty());
    }

    /// `partypetN` names the member's `CHARM`, else `SUMMON`, while the member is held, else the
    /// record's pet guid (`0x4e81d0`); the pet's list is its own descriptor while held, else the
    /// pet record's block (`0x496420`, `+0xe4`).
    #[test]
    fn a_party_pet_reads_its_descriptor_or_the_pet_block() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                pet_guid: Some(PET),
                pet_auras: Some(vec![(1, 1126)]),
                pet_auras_negative: Some(vec![(32, 770)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "partypet1", false), [1126]);
        assert_eq!(listed(&mut app, "partypet1", true), [770]);
        assert!(listed(&mut app, "partypet2", true).is_empty());
        assert_eq!(seen(&mut app), ["party1", "partypet1"]);

        // A pet-aura delta moves the pet's list and fires for its token alone.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                pet_auras_negative: Some(vec![(32, 0), (33, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "partypet1", true), [589]);
        assert_eq!(seen(&mut app), ["partypet1"]);

        // Member and pet in view: the pet guid comes off the member's SUMMON, the list off the
        // pet's own packet.
        stream(
            &mut app,
            MEMBER,
            &[(SUMMON, PET as u32), (SUMMON + 1, (PET >> 32) as u32)],
        );
        stream(&mut app, PET, &[]);
        packet(&mut app, PET, &[(35, 11976, false)]);
        app.update();
        assert_eq!(listed(&mut app, "partypet1", true), [11976]);
        assert!(listed(&mut app, "partypet1", false).is_empty());
        assert!(seen(&mut app).contains(&"partypet1".to_string()));
    }

    // == The raid tokens ==

    /// In our subgroup, so `party1` and `raid2`.
    const MATE: u64 = 0x2001;
    /// Subgroup 2, `raid5` only.
    const FAR: u64 = 0x2004;
    const FAR_PET: u64 = 0xF140_0000_0000_0099;

    /// A raid of five: us (row 1, subgroup 1), `MATE` (subgroup 1), two in subgroup 2 and `FAR`
    /// in subgroup 3, in wire order, none streamed and none with a record.
    fn raid_app() -> App {
        let mut app = party_app();
        relist_raid(&mut app, member_status::ONLINE);
        app.update();
        let _ = seen(&mut app);
        app
    }

    /// The raid's list as `raid_app` sends it, with `MATE`'s status byte as given.
    fn relist_raid(app: &mut App, mate: u8) {
        let member = |name: &str, guid, status, subgroup| GroupMemberEntry {
            name: name.into(),
            guid,
            status,
            flags: subgroup,
        };
        let online = member_status::ONLINE;
        app.world_mut().resource_mut::<GroupState>().apply_list(
            crate::app_core::state::GROUPTYPE_RAID,
            0,
            vec![
                member("Mate", MATE, mate, 0),
                member("Two", 0x2002, online, 1),
                member("Three", 0x2003, online, 1),
                member("Far", FAR, online, 2),
            ],
            ME,
            None,
            Some(ME),
        );
    }

    fn online(auras_negative: Vec<(u8, u16)>) -> PartyMemberStatsInfo {
        PartyMemberStatsInfo {
            status: Some(member_status::ONLINE),
            auras_negative: Some(auras_negative),
            ..Default::default()
        }
    }

    /// A raid member in view reads its own packet auras under `raidN`, whatever its subgroup; the
    /// native list applies its own whole-unit gate, which passes a raid member's buffs.
    #[test]
    fn a_live_raid_members_packet_auras_list_under_its_raid_token() {
        let mut app = raid_app();
        stream(&mut app, FAR, &[]);
        packet(&mut app, FAR, &[(0, 1126, true), (33, 589, false)]);
        app.update();
        assert_eq!(listed(&mut app, "raid5", true), [589]);
        assert_eq!(listed(&mut app, "raid5", false), [1126]);
        assert_eq!(seen(&mut app), ["raid5"]);
        for token in ["party1", "party2", "raid6"] {
            assert!(listed(&mut app, token, true).is_empty(), "{token}");
        }
    }

    /// Out of range, `raidN` walks the member's record, which vmangos keeps current for every raid
    /// member we do not see; `raidpetN` walks the pet block the same record names.
    #[test]
    fn an_out_of_range_raid_member_and_its_pet_read_the_record() {
        let mut app = raid_app();
        stats_of(
            &mut app,
            FAR,
            true,
            PartyMemberStatsInfo {
                pet_guid: Some(FAR_PET),
                pet_auras: Some(vec![(1, 1126)]),
                pet_auras_negative: Some(vec![(33, 770)]),
                ..online(vec![(32, 589), (35, 11976)])
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "raid5", true), [589, 11976]);
        let count = app
            .world_mut()
            .non_send_resource::<UiScript>()
            .eval::<i64>(r#"local _, _, _, count = UnitDebuff("raid5", 1) return count"#)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(listed(&mut app, "raidpet5", false), [1126]);
        assert_eq!(listed(&mut app, "raidpet5", true), [770]);
        assert_eq!(seen(&mut app), ["raid5", "raidpet5"]);
        assert!(
            listed(&mut app, "raidpet4", true).is_empty(),
            "a petless member's raidpet token names nobody"
        );
    }

    /// `UNIT_AURA` fires for every token naming a unit whose list moved (`0x515e50`), and for
    /// nothing else; a delta that moves no aura fires nothing.
    #[test]
    fn unit_aura_fires_for_each_token_naming_the_member_whose_list_moved() {
        let mut app = raid_app();
        stats_of(&mut app, MATE, true, online(vec![(32, 589)]));
        stats_of(&mut app, FAR, true, online(vec![(32, 770)]));
        app.update();
        let mut first = seen(&mut app);
        first.sort();
        assert_eq!(first, ["party1", "raid2", "raid5"]);

        stats_of(&mut app, MATE, false, online(vec![(33, 11976)]));
        app.update();
        assert_eq!(seen(&mut app), ["party1", "raid2"]);
        assert_eq!(listed(&mut app, "raid2", true), [589, 11976]);
        assert_eq!(listed(&mut app, "party1", true), [589, 11976]);

        stats_of(
            &mut app,
            FAR,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(7),
                ..Default::default()
            },
        );
        app.update();
        assert!(seen(&mut app).is_empty());
    }

    /// The raid roster's twin of the list write (`0x4ba947`-`0x4ba960`): a status-only raid list
    /// hides an out-of-view row's auras under `raidN` and its party slot alike, and an online list
    /// returns them.
    #[test]
    fn a_status_only_raid_list_takes_an_out_of_view_rows_auras_and_an_online_one_returns_them() {
        let mut app = raid_app();
        stats_of(&mut app, MATE, true, online(vec![(32, 589)]));
        app.update();
        assert_eq!(listed(&mut app, "raid2", true), [589]);
        assert_eq!(listed(&mut app, "party1", true), [589]);

        relist_raid(&mut app, member_status::OFFLINE);
        app.update();
        assert!(listed(&mut app, "raid2", true).is_empty());
        assert!(listed(&mut app, "party1", true).is_empty());

        relist_raid(&mut app, member_status::ONLINE);
        app.update();
        assert_eq!(listed(&mut app, "raid2", true), [589]);
        assert_eq!(listed(&mut app, "party1", true), [589]);
    }

    /// A raid token past the roster, a raid token outside a raid, and an offline member's pet
    /// all answer nil.
    #[test]
    fn a_raid_token_naming_nobody_answers_nil() {
        let mut app = party_app();
        stats(&mut app, true, online(vec![(32, 589)]));
        app.update();
        assert_eq!(listed(&mut app, "party1", true), [589]);
        for token in ["raid1", "raid2"] {
            assert!(
                listed(&mut app, token, true).is_empty(),
                "a party is no raid: {token}"
            );
        }

        let mut app = raid_app();
        stats_of(
            &mut app,
            FAR,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::OFFLINE),
                pet_guid: Some(FAR_PET),
                pet_auras_negative: Some(vec![(33, 770)]),
                auras_negative: Some(vec![(32, 589)]),
                ..Default::default()
            },
        );
        app.update();
        for token in ["raid5", "raidpet5", "raid6", "raid40", "raidpet40"] {
            assert!(listed(&mut app, token, true).is_empty(), "{token}");
        }
    }

    /// Row 1 is us: the player's own list, as `"player"` answers it.
    #[test]
    fn our_raid_row_mirrors_the_player_list() {
        let mut app = raid_app();
        packet(&mut app, ME, &[(32, 589, false)]);
        app.update();
        assert_eq!(listed(&mut app, "player", true), [589]);
        assert_eq!(listed(&mut app, "raid1", true), [589]);
        assert_eq!(seen(&mut app), ["raid1"]);
    }

    /// A frame where nothing moved builds no list: the feed compares the inputs, not the lists.
    #[test]
    fn an_unchanged_raid_rebuilds_no_list() {
        let mut app = raid_app();
        stats_of(&mut app, FAR, true, online(vec![(32, 589)]));
        app.update();
        let versions = |app: &mut App| {
            app.world_mut()
                .resource_scope(|world, mut mem: Mut<super::super::AuraFeedMemory>| {
                    let script = world.non_send_resource::<UiScript>();
                    mem.vm.get(script).units.versions
                })
        };
        let before = versions(&mut app);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(versions(&mut app), before);
        stats_of(&mut app, FAR, false, online(vec![(33, 770)]));
        app.update();
        assert_eq!(
            versions(&mut app),
            before + 1,
            "one list moved, one rebuilt"
        );
    }

    // == Any token the resolver parses ==

    /// `OBJECT_FIELD_TYPE`; a unit's typemask is `0x9`, a player's `0x19`.
    const TYPE: u16 = Field::ObjectType.index();
    /// `UNIT_FIELD_TARGET`'s low dword; the high half is the next index.
    const TARGET: u16 = Field::UnitTarget.index();
    const MOB: u64 = 0xF130_0000_0000_0001;
    const ADD: u64 = 0xF130_0000_0000_0002;

    /// A unit of typemask `ty` targeting `target`.
    fn unit_fields(ty: u32, target: u64) -> Vec<(u16, u32)> {
        vec![
            (TYPE, ty),
            (TARGET, target as u32),
            (TARGET + 1, (target >> 32) as u32),
        ]
    }

    /// We target `MOB`, which targets party1, who targets `ADD`, which targets us; the mouse is over
    /// `ADD`, and our pet targets `MOB`. Each carries its own debuff: us 6788, `MOB` 589, party1
    /// 11976, `ADD` 770.
    fn chain_app() -> App {
        let mut app = party_app();
        app.init_resource::<crate::target::Hovered>();
        let me = app.world_mut().resource::<GuidIndex>().0[&ME];
        *app.world_mut().get_mut::<ObjectStore>(me).unwrap() =
            ObjectStore(ObjectFields::from_pairs(&unit_fields(0x19, MOB)));
        stream(&mut app, MOB, &unit_fields(0x9, MEMBER));
        stream(&mut app, MEMBER, &unit_fields(0x19, ADD));
        stream(&mut app, ADD, &unit_fields(0x9, ME));
        stream(&mut app, PET, &unit_fields(0x9, MOB));
        for (guid, debuff) in [
            (ME, 6788),
            (MOB, 589),
            (MEMBER, 11976),
            (ADD, 770),
            (PET, 1),
        ] {
            packet(&mut app, guid, &[(32, debuff, false)]);
        }
        app.world_mut()
            .resource_mut::<crate::app_core::state::PetBar>()
            .spells
            .pet_guid = PET;
        let (mob, add) = {
            let index = &app.world().resource::<GuidIndex>().0;
            (index[&MOB], index[&ADD])
        };
        let mut selection = app
            .world_mut()
            .resource_mut::<crate::app_core::state::Selection>();
        (selection.target, selection.guid) = (Some(mob), Some(MOB));
        let mut hovered = app.world_mut().resource_mut::<crate::target::Hovered>();
        (hovered.target, hovered.guid) = (Some(add), Some(ADD));
        app.update();
        app
    }

    /// `UnitDebuff` reads the list of whatever unit the token names, through the resolver
    /// (`0x60abf0` via `0x6110a0`): the mouseover, a member's target, any depth of `target` chain,
    /// in any case, and the player's own list at the end of a chain that comes back to us.
    #[test]
    fn every_token_reads_the_list_of_the_unit_it_names() {
        let mut app = chain_app();
        for (token, want) in [
            ("mouseover", 770),
            ("MouseOver", 770),
            ("party1target", 770),
            ("PARTY1TARGET", 770),
            ("target", 589),
            ("targettarget", 11976),
            ("targettargettarget", 770),
            ("targettargettargettarget", 6788),
            ("pettarget", 589),
            ("playertarget", 589),
            ("mouseovertargettarget", 589),
            ("PLAYER", 6788),
            ("Player", 6788),
        ] {
            assert_eq!(listed(&mut app, token, true), [want], "{token}");
        }
        assert_eq!(
            listed(&mut app, "PLAYER", true),
            listed(&mut app, "player", true)
        );
    }

    /// `UNIT_AURA` fires for the tokens it fired for before, never for a chain or the mouseover,
    /// which the reference's fan-out (`0x515c50`) does not list.
    #[test]
    fn unit_aura_still_fires_for_the_fixed_tokens_alone() {
        let mut app = chain_app();
        let mut first = seen(&mut app);
        first.sort();
        assert_eq!(first, ["party1", "pet", "target", "targettarget"]);

        // `ADD`'s list moves: it is the mouseover, party1's target and three hops off ours, and
        // no token that fires names it.
        packet(&mut app, ADD, &[(32, 770, false), (33, 589, false)]);
        app.update();
        assert!(seen(&mut app).is_empty());
        assert_eq!(listed(&mut app, "mouseover", true), [770, 589]);
        assert_eq!(listed(&mut app, "party1target", true), [770, 589]);
    }

    /// A nearer GameObject wins the pick, and the publisher writes its guid, which the resolver
    /// rejects as a unit (`0x515bd9 je`): the unit behind it is no longer the mouseover.
    #[test]
    fn a_nearer_gameobject_takes_the_mouseover_from_the_unit_behind_it() {
        let mut app = chain_app();
        assert_eq!(listed(&mut app, "mouseover", true), [770]);
        app.world_mut()
            .resource_mut::<crate::target::Hovered>()
            .distance = 10.0;
        let chest = app.world_mut().spawn_empty().id();
        app.insert_resource(crate::target::HoveredObject {
            target: Some(chest),
            guid: Some(0xF110_0000_0000_0004),
            distance: 5.0,
        });
        app.update();
        assert!(listed(&mut app, "mouseover", true).is_empty());
        assert!(listed(&mut app, "mouseovertarget", true).is_empty());
    }

    /// A raid member's target that is out of view but on the roster reads its record, as
    /// `UnitDebuff`'s roster walk finds it (`0x519b25`); a hop off a unit not held is nobody.
    #[test]
    fn a_raid_members_target_reads_whatever_the_guid_holds() {
        let mut app = raid_app();
        stats_of(&mut app, FAR, true, online(vec![(32, 589)]));
        // `raid3` is 0x2002, in view and targeting `FAR`, which is not.
        stream(&mut app, 0x2002, &unit_fields(0x19, FAR));
        packet(&mut app, 0x2002, &[(32, 770, false)]);
        app.update();
        assert_eq!(listed(&mut app, "raid3", true), [770]);
        assert_eq!(listed(&mut app, "raid3target", true), [589]);
        assert_eq!(listed(&mut app, "raid5", true), [589]);
        assert!(listed(&mut app, "raid5target", true).is_empty());
        assert!(listed(&mut app, "raid3targettarget", true).is_empty());
    }
}
