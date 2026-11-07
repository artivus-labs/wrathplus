// Context changes without an aura packet must refresh cached lists.
use super::*;

    /// The actual scheduled feed must reconsider book metadata even though neither the live
    /// member's packet nor the out-of-range roster block receives an aura update.
    #[test]
    fn stable_native_packet_and_roster_refresh_raid_after_learning_and_unlearning() {
        use crate::app_core::state::{PlayerActions, Spells};
        use wrathplus_formats::{SpellCatalog, SpellDisplay};
        for live in [false, true] {
            let mut app = party_app();
            app.insert_resource(Spells {
                catalog: SpellCatalog::from_displays(std::collections::HashMap::from([
                    (
                        123,
                        SpellDisplay {
                            dispel: 1,
                            ..Default::default()
                        },
                    ),
                    (
                        456,
                        SpellDisplay {
                            effects: [38, 0, 0],
                            effect_misc_value: [1, 0, 0],
                            effect_implicit_target_a: [21, 0, 0],
                            ..Default::default()
                        },
                    ),
                    (
                        789,
                        SpellDisplay {
                            effect_implicit_target_a: [21, 0, 0],
                            ..Default::default()
                        },
                    ),
                ])),
                ..Spells::empty_for_tests()
            });
            app.init_resource::<PlayerActions>();
            if live {
                stream(&mut app, MEMBER, &unit_fields(0x19, 0));
                packet(&mut app, MEMBER, &[(0, 789, true), (32, 123, false)]);
            } else {
                use wrathplus_protocol::wrath_group::{Aura, Member, Roster, Stats};
                let mut group = app.world_mut().resource_mut::<GroupState>();
                group.apply_wrath_list(
                    Roster {
                        group_type: 0,
                        subgroup: 0,
                        flags: 0,
                        roles: 0,
                        lfg: None,
                        group_guid: 42,
                        counter: 1,
                        leader: ME,
                        loot: None,
                        members: vec![Member {
                            name: "Ally".into(),
                            guid: MEMBER,
                            status: 1,
                            subgroup: 0,
                            flags: 0,
                            roles: 0,
                        }],
                    },
                    Some(ME),
                );
                group.apply_wrath_stats(Stats {
                    guid: MEMBER,
                    status: Some(1),
                    auras: Some(vec![
                        Aura {
                            slot: 0,
                            spell: 789,
                            flags: 0x10,
                        },
                        Aura {
                            slot: 32,
                            spell: 123,
                            flags: 0x80,
                        },
                    ]),
                    ..Default::default()
                });
            }
            app.update();
            let eligible = |app: &mut App| {
                app.world_mut()
                    .non_send_resource::<UiScript>()
                    .eval::<(bool, bool)>(
                        r#"return UnitDebuff("party1", 1, "RAID") ~= nil,
                    UnitBuff("party1", 1, "RAID") ~= nil"#,
                    )
                    .unwrap()
            };
            assert_eq!(eligible(&mut app), (false, false));
            let _ = seen(&mut app);
            app.world_mut()
                .resource_mut::<PlayerActions>()
                .spells
                .extend([456, 789]);
            app.update();
            assert_eq!(eligible(&mut app), (true, true));
            assert_eq!(seen(&mut app), ["party1"]);
            app.update();
            assert!(
                seen(&mut app).is_empty(),
                "unchanged context is not republished"
            );
            app.world_mut()
                .resource_mut::<PlayerActions>()
                .spells
                .clear();
            app.update();
            assert_eq!(eligible(&mut app), (false, false));
            assert_eq!(seen(&mut app), ["party1"]);
            app.world_mut()
                .resource_mut::<PlayerActions>()
                .spells
                .insert(456);
            app.update();
            assert_eq!(eligible(&mut app), (true, false));
            let _ = seen(&mut app);
            app.world_mut().remove_resource::<PlayerActions>();
            app.update();
            assert_eq!(eligible(&mut app), (false, false));
            assert_eq!(seen(&mut app), ["party1"]);
        }
    }

    #[test]
    fn stable_native_packet_refreshes_player_filter_and_caster_when_caster_streams_or_changes_owner(
    ) {
        use crate::net::wrath_auras::{PacketAuras, StampedAura};
        use wrathplus_protocol::wrath_auras::NativeAura;
        let mut app = party_app();
        stream(&mut app, MEMBER, &unit_fields(0x19, 0));
        app.insert_resource(PacketAuras {
            by_guid: std::collections::HashMap::from([(
                MEMBER,
                vec![StampedAura {
                    received_at: 0.0,
                    aura: NativeAura {
                        slot: 0,
                        spell_id: 123,
                        flags: 0x10,
                        level: 80,
                        stacks: 1,
                        caster_guid: Some(PET),
                        max_duration_ms: None,
                        remaining_ms: None,
                    },
                }],
            )]),
        });
        let query = |app: &mut App| {
            app.world_mut()
                .non_send_resource::<UiScript>()
                .eval::<(bool, Option<String>)>(
                    r#"
                local _, _, _, _, _, _, _, caster = UnitBuff("party1", 1)
                return UnitBuff("party1", 1, "PLAYER") ~= nil, caster
            "#,
                )
                .unwrap()
        };
        app.update();
        assert_eq!(query(&mut app), (false, None));
        let _ = seen(&mut app);
        let links = |owner: u64, charm: u64| {
            let mut fields = unit_fields(0x9, 0);
            for (field, guid) in [(Field::UnitCreatedby, owner), (Field::UnitCharmedby, charm)] {
                fields.extend([
                    (field.index(), guid as u32),
                    (field.index() + 1, (guid >> 32) as u32),
                ]);
            }
            fields
        };
        stream(&mut app, PET, &links(ME, 0));
        app.update();
        assert_eq!(query(&mut app), (true, Some("player".into())));
        assert_eq!(seen(&mut app), ["party1"]);
        app.update();
        assert!(seen(&mut app).is_empty());
        let caster = app.world().resource::<GuidIndex>().0[&PET];
        *app.world_mut().get_mut::<ObjectStore>(caster).unwrap() =
            ObjectStore(ObjectFields::from_pairs(&links(ME, MEMBER)));
        app.update();
        assert_eq!(query(&mut app), (false, Some("party1".into())));
        assert_eq!(seen(&mut app), ["party1"]);
        *app.world_mut().get_mut::<ObjectStore>(caster).unwrap() =
            ObjectStore(ObjectFields::from_pairs(&links(MEMBER, 0)));
        app.update();
        assert_eq!(query(&mut app), (false, Some("party1".into())));
        assert!(
            seen(&mut app).is_empty(),
            "same resolved links do not invalidate"
        );
        app.world_mut().resource_mut::<GuidIndex>().0.remove(&PET);
        app.world_mut().despawn(caster);
        app.update();
        assert_eq!(query(&mut app), (false, None));
        assert_eq!(seen(&mut app), ["party1"]);
    }
