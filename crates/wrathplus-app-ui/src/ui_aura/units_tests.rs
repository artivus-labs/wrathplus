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

    mod context_tests {
        include!("context_tests.rs");
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
