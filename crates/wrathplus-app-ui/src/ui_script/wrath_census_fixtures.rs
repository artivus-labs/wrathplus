//! Representative state for the arity audit: one row per list the stock 3.3.5 Lua unpacks, so a
//! getter answers a hit rather than its miss shape. Seeded through the public setters, which only
//! store; nothing here fires an event. Values are plausible, not a recording.

use std::collections::HashMap;

use wrathplus_ui::script::*;

/// The player's guid in the fixtures.
const ME: u64 = 0x0100_0000_0000_0001;
/// The target, a creature on whose threat table the player sits.
const MOB: u64 = 0xF130_0000_0000_0002;

fn slot(item_id: u32, name: &str) -> ContainerSlot {
    ContainerSlot {
        texture: Some(r"Interface\Icons\INV_Misc_Bag_08".into()),
        count: 1,
        quality: Some(2),
        item_id,
        link: Some(format!(
            "|cff1eff00|Hitem:{item_id}:0:0:0:0:0:0:0:80|h[{name}]|h|r"
        )),
        durability: Some((40, 50)),
        ..Default::default()
    }
}

fn template(name: &str) -> ItemTemplateView {
    ItemTemplateView {
        name: name.into(),
        quality: 2,
        class: 4,
        subclass: 1,
        inventory_type: 5,
        item_type: Some("Armor".into()),
        item_sub_type: Some("Cloth".into()),
        max_durability: 50,
        stackable: 1,
        sell_price: 100,
        icon: Some(r"Interface\Icons\INV_Chest_Cloth_17".into()),
        ..Default::default()
    }
}

fn reward_spell() -> QuestRewardSpell {
    QuestRewardSpell {
        spell_id: 2050,
        name: Some("Lesser Heal".into()),
        texture: Some(r"Interface\Icons\Spell_Holy_LesserHeal".into()),
        ..Default::default()
    }
}

/// Seeds every list the audit's probes read with one representative row.
pub(super) fn seed(s: &mut UiScript) {
    // Items, bags and the backpack.
    s.set_item_template(6948, template("Hearthstone"));
    s.set_item_template(2589, template("Linen Cloth"));
    for bag in [0i64, 1] {
        let mut slots = HashMap::new();
        slots.insert(1, slot(6948, "Hearthstone"));
        s.set_container(
            bag,
            Some(ContainerState {
                name: Some("Bag".into()),
                num_slots: 16,
                slots,
            }),
        );
    }

    // The player resolves by guid, as the aura readers need.
    s.set_unit_guids(&UnitGuids {
        player: ME,
        target: MOB,
        wrath: true,
        ..Default::default()
    });

    // Mail: build 12340's inbox, a letter with one attachment.
    let header = MailInboxRow {
        package_icon: Some(r"Interface\Icons\INV_Letter_15".into()),
        sender: Some("Benchtwo".into()),
        subject: "Hello".into(),
        money: 100,
        days_left: 29.5,
        item_count: 1,
        can_reply: true,
        body: Some("Body".into()),
        has_body: true,
        text_created: true,
        item_id: 6948,
        item_name: Some("Hearthstone".into()),
        item_texture: Some(r"Interface\Icons\INV_Misc_Rune_01".into()),
        item_quality: Some(1),
        can_delete: true,
        ..Default::default()
    };
    s.set_wrath_mail(Some(WrathMailInbox {
        total_count: 1,
        rows: vec![WrathMailRow {
            header,
            attachments: vec![WrathMailAttachment {
                slot: 0,
                entry: 6948,
                count: 1,
                name: Some("Hearthstone".into()),
                texture: Some(r"Interface\Icons\INV_Misc_Rune_01".into()),
                quality: Some(1),
                ..Default::default()
            }],
            kind: 0,
            sender_guid: 0x0100_0000_0000_0002,
        }],
    }));

    // A trade window with one item each side.
    let traded = TradeSlotItem {
        item_id: 2589,
        name: Some("Linen Cloth".into()),
        texture: Some(r"Interface\Icons\INV_Fabric_Linen_01".into()),
        count: 5,
        quality: Some(1),
        ..Default::default()
    };
    let mut trade = TradeState::default();
    trade.player.slots[0] = Some(traded.clone());
    trade.target.slots[0] = Some(traded);
    s.set_trade(Some(trade));

    // The quest log: a header, then a quest with an objective and a reward spell.
    s.set_quest_log(QuestLogState {
        entries: vec![
            QuestLogEntryView {
                title: "Elwynn Forest".into(),
                is_header: true,
                ..Default::default()
            },
            QuestLogEntryView {
                quest_id: 55,
                title: "A Threat Within".into(),
                level: 1,
                objectives: vec![QuestLogObjectiveView {
                    text: "Kobold Vermin slain: 0/10".into(),
                    kind: "monster".into(),
                    finished: false,
                    ..Default::default()
                }],
                detail: Some(QuestLogDetail {
                    description: "Go".into(),
                    objectives_text: "Kill".into(),
                    reward_spell: Some(reward_spell()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            // Quest id 1, which the probe ladder passes: a POI icon on the displayed map.
            QuestLogEntryView {
                quest_id: 1,
                title: "Report to Goldshire".into(),
                level: 5,
                objectives: vec![QuestLogObjectiveView {
                    text: "Marshal Dughan: 0/1".into(),
                    kind: "monster".into(),
                    finished: false,
                    poi_bit: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    // Friends, who, ignores and the guild roster.
    s.set_social(SocialState {
        friends: vec![FriendInfo {
            name: "Benchtwo".into(),
            level: 80,
            class: "Mage".into(),
            area: "Dalaran".into(),
            connected: true,
            status: String::new(),
            note: Some("Bench partner".into()),
            refer_a_friend: false,
        }],
        who: vec![WhoInfo {
            name: "Benchthree".into(),
            guild: "Bench".into(),
            level: 80,
            race: "Human".into(),
            class: "Priest".into(),
            zone: "Dalaran".into(),
            class_file: "PRIEST".into(),
        }],
        who_total: 1,
        ..Default::default()
    });
    s.set_guild(GuildState {
        in_guild: true,
        name: "Bench".into(),
        rank_name: "Member".into(),
        roster: vec![GuildMemberInfo {
            name: "Benchtwo".into(),
            rank: "Member".into(),
            level: 80,
            class: "Mage".into(),
            zone: "Dalaran".into(),
            online: true,
            class_file: "MAGE".into(),
            ..Default::default()
        }],
        ..Default::default()
    });

    // Macros, spells, forms and auras.
    s.set_macros(MacroState {
        account: vec![MacroView {
            name: "Hello".into(),
            texture: Some(r"Interface\Icons\INV_Misc_QuestionMark".into()),
            body: "/wave".into(),
            id: 1,
        }],
        character: vec![],
    });
    s.set_spell_catalog([SpellInfoView {
        spell_id: 6603,
        name: "Attack".into(),
        rank: Some(String::new()),
        texture: Some(r"Interface\Icons\INV_Sword_04".into()),
        max_range: 5.0,
        ..Default::default()
    }]);
    s.set_spellbook(SpellBookState {
        tabs: vec![SpellTabView {
            name: "General".into(),
            texture: Some(r"Interface\Icons\INV_Misc_Book_09".into()),
            offset: 0,
            num_spells: 1,
        }],
        slots: vec![SpellSlotView {
            spell_id: 6603,
            name: "Attack".into(),
            rank: Some(String::new()),
            texture: Some(r"Interface\Icons\INV_Sword_04".into()),
            ..Default::default()
        }],
    });
    s.set_shapeshift_forms(vec![ShapeshiftFormView {
        spell_id: 71,
        texture: Some(r"Interface\Icons\Ability_Warrior_DefensiveStance".into()),
        name: "Defensive Stance".into(),
        active: true,
        castable: true,
        cooldown: None,
    }]);
    s.set_player_auras(vec![
        AuraState {
            spell_id: 1243,
            name: Some("Power Word: Fortitude".into()),
            icon: Some(r"Interface\Icons\Spell_Holy_WordFortitude".into()),
            count: 1,
            duration: 1800.0,
            expiration_time: 1900.0,
            helpful: true,
            cancelable: true,
            ..Default::default()
        },
        AuraState {
            spell_id: 589,
            name: Some("Shadow Word: Pain".into()),
            icon: Some(r"Interface\Icons\Spell_Shadow_ShadowWordPain".into()),
            count: 1,
            debuff_type: Some("Magic".into()),
            duration: 18.0,
            expiration_time: 118.0,
            ..Default::default()
        },
    ]);

    // Gossip and the questgiver greeting.
    s.set_gossip(Some(GossipMenu {
        greeting: "Greetings".into(),
        quests: vec![],
        options: vec![GossipOptionView {
            label: "Train me".into(),
            icon_type: "trainer".into(),
        }],
    }));

    // The trainer.
    s.set_trainer(Some(TrainerState {
        services: vec![TrainerService {
            spell_id: 100,
            name: Some("Charge".into()),
            subtext: Some("Rank 1".into()),
            texture: Some(r"Interface\Icons\Ability_Warrior_Charge".into()),
            cost: 10,
            level_req: 4,
            skill_req: Some(TrainerSkillReq {
                name: "Arms".into(),
                rank: 1,
                met: true,
            }),
            ability_reqs: vec![TrainerAbilityReq {
                name: "Battle Stance".into(),
                met: true,
            }],
            group_key: 1,
            group_name: "Arms".into(),
            ..Default::default()
        }],
        greeting: "Hello".into(),
        ..Default::default()
    }));

    // The questgiver greeting: one active and one available quest.
    s.set_quest(Some(QuestState {
        panel: QuestPanel::Greeting,
        greeting: "Hello".into(),
        active_titles: vec!["A Threat Within".into()],
        available_titles: vec!["Kobold Camp Cleanup".into()],
        active_info: vec![QuestGreetingInfo::default()],
        available_info: vec![QuestGreetingInfo::default()],
        reward_spell: Some(reward_spell()),
        ..Default::default()
    }));

    s.set_wrath_currency_snapshot(WrathCurrencySnapshot {
        entries: vec![
            WrathCurrencyEntry {
                name: Some("Dungeon and Raid".into()),
                category_id: 22,
                is_header: true,
                is_expanded: true,
                ..Default::default()
            },
            WrathCurrencyEntry {
                name: Some("Emblem of Frost".into()),
                category_id: 22,
                is_watched: true,
                count: 12,
                icon: Some(r"Interface\Icons\INV_Misc_Frostemblem_01".into()),
                item_id: 49426,
                ..Default::default()
            },
        ],
        honor_current: Some(100),
        arena_current: Some(10),
        token_stacks: vec![(49426, 12)],
    });

    // The player's cast and channel.
    let cast = WrathUnitCast {
        spell_id: 133,
        name: "Fireball".into(),
        rank: "Rank 1".into(),
        icon: r"Interface\Icons\Spell_Fire_FlameBolt".into(),
        start_ms: 1000,
        end_ms: 2500,
        ..Default::default()
    };
    s.set_wrath_unit_casts(
        HashMap::from([(ME, cast.clone())]),
        HashMap::from([(ME, cast)]),
    );

    // A loot roll, a companion, a weapon enchant and an action button.
    s.set_loot_rolls(LootRollsState {
        rolls: vec![LootRollEntry {
            roll_id: 1,
            name: Some("Hearthstone".into()),
            texture: Some(r"Interface\Icons\INV_Misc_Rune_01".into()),
            quantity: 1,
            quality: Some(3),
            time_left_ms: 60_000,
            item_id: 6948,
            ..Default::default()
        }],
    });
    s.set_wrath_companions(WrathCompanions {
        lists: [
            vec![CompanionView {
                creature_id: 7389,
                creature_name: Some("Senegal".into()),
                spell_id: 10684,
                icon: Some(r"Interface\Icons\Spell_Nature_ForceOfNature".into()),
                ..Default::default()
            }],
            vec![],
        ],
        critter_guid: 0,
    });
    s.set_weapon_enchants(
        Some(WeaponEnchant {
            remaining_ms: Some(60_000),
            charges: 0,
        }),
        None,
    );
    s.set_action(
        1,
        Some(ActionSlot {
            texture: Some(r"Interface\Icons\INV_Sword_04".into()),
            kind: 0,
            action: 6603,
            count: 0,
            consumable: false,
        }),
    );

    // The dungeon finder: a catalog row, a queue and a proposal.
    s.set_wrath_lfg_catalog(
        std::sync::Arc::new(WrathLfgCatalog {
            dungeons: vec![
                WrathLfgDungeon {
                    id: 1,
                    name: "Ragefire Chasm".into(),
                    dungeon_type: 1,
                    min_level: 15,
                    target_min_level: 15,
                    target_max_level: 21,
                    ..Default::default()
                },
                WrathLfgDungeon {
                    id: 2,
                    name: s.eval("return LFG_TYPE_RANDOM_DUNGEON").unwrap_or_default(),
                    dungeon_type: 6,
                    min_level: 15,
                    target_min_level: 15,
                    target_max_level: 80,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        Some(2),
    );
    s.set_wrath_lfg_lock_snapshot(std::sync::Arc::new(WrathLfgLockSnapshot {
        player: Some(WrathLfgPlayerLocks {
            locks: vec![WrathLfgLock {
                entry: 0x0100_0001,
                reason: 2,
            }],
            // The reward preview sits on the random row, the rows `GetLFGDungeonRewardInfo` reads.
            previews: vec![WrathLfgPreview {
                entry: 0x0600_0002,
                money_base: 100,
                xp_base: 1000,
                items: vec![WrathLfgRewardItem {
                    item: 6948,
                    display: 1,
                    count: 1,
                }],
                ..Default::default()
            }],
        }),
        party: None,
    }));
    s.set_wrath_lfg_queue(std::sync::Arc::new(WrathLfgQueueSnapshot {
        player: Some(WrathLfgUpdate {
            update_type: 5,
            joined: true,
            queued: true,
            dungeons: vec![0x0100_0001],
            ..Default::default()
        }),
        role_check: Some(WrathLfgRoleCheck {
            state: 2,
            is_beginning: false,
            dungeons: vec![0x0100_0001],
            members: vec![WrathLfgRoleCheckMember {
                guid: ME,
                ready: true,
                roles: 2,
                level: 80,
                name: Some("Benchone".into()),
            }],
        }),
        completion: Some(WrathLfgCompletion {
            dungeon: 0x0100_0001,
            strangers: 2,
            money_base: 100,
            xp_base: 1000,
            items: vec![WrathLfgCompletionItem {
                item: 6948,
                display: 1,
                count: 1,
            }],
            ..Default::default()
        }),
        queue: Some(WrathLfgQueueStatus {
            dungeon: 0x0100_0001,
            avg_wait: 300,
            my_wait: 120,
            tank_needs: 1,
            healer_needs: 1,
            dps_needs: 3,
            ..Default::default()
        }),
        proposal: Some(WrathLfgProposal {
            dungeon: 0x0100_0001,
            state: 1,
            id: 7,
            rows: vec![WrathLfgProposalRow {
                roles: 2,
                is_self: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }));

    // A merchant row with an extended cost, and a world-state line.
    s.set_merchant(Some(MerchantState {
        items: vec![MerchantItem {
            name: Some("Linen Cloth".into()),
            texture: Some(r"Interface\Icons\INV_Fabric_Linen_01".into()),
            price: 10,
            quantity: 1,
            num_available: -1,
            item_id: 2589,
            extended_cost: Some(ExtendedCostView {
                honor: 0,
                arena: 0,
                items: vec![CostItemView {
                    item_id: 49426,
                    count: 2,
                    texture: Some(r"Interface\Icons\INV_Misc_Frostemblem_01".into()),
                    link: Some(
                        "|cffa335ee|Hitem:49426:0:0:0:0:0:0:0:80|h[Emblem of Frost]|h|r".into(),
                    ),
                }],
                rating_ok: true,
            }),
            ..Default::default()
        }],
        buyback: vec![],
        can_repair: true,
    }));
    s.set_world_state_ui(vec![WorldStateUiView {
        ui_type: 0,
        ui_state: 1,
        text: "Bases: 2".into(),
        icon: r"Interface\TargetingFrame\UI-PVP-Alliance".into(),
        tooltip: "Bases".into(),
        ..Default::default()
    }]);

    // A worn chest piece, the pet's spellbook and the quest log's item extras.
    let mut worn: InventorySlots = Default::default();
    worn[1] = Some(InvSlotView {
        item_id: 6948,
        icon: Some(r"Interface\Icons\INV_Helmet_01".into()),
        count: 1,
        quality: 2,
        name: Some("Hearthstone".into()),
        ..Default::default()
    });
    worn[5] = Some(InvSlotView {
        item_id: 6948,
        icon: Some(r"Interface\Icons\INV_Chest_Cloth_17".into()),
        count: 1,
        quality: 2,
        name: Some("Hearthstone".into()),
        durability: Some((40, 50)),
        ..Default::default()
    });
    s.set_inventory_slots(worn);
    s.set_pet_book(PetBookState {
        token: Some("PET".into()),
        slots: vec![SpellSlotView {
            spell_id: 3110,
            name: "Firebolt".into(),
            rank: Some("Rank 1".into()),
            texture: Some(r"Interface\Icons\Spell_Fire_FireBolt".into()),
            autocast: Some((true, true)),
            ..Default::default()
        }],
    });
    s.set_wrath_quest_item_drops(HashMap::from([(55, (1, vec![(6948, 1)]))]));
    s.set_wrath_quest_special_items(vec![(55, 0, 1)]);

    // A profession with one recipe and its reagent.
    s.set_trade_skill(Some(TradeSkillState {
        line: 197,
        line_name: "Tailoring".into(),
        rank: 1,
        max_rank: 75,
        recipes: vec![TradeSkillRecipe {
            group: None,
            spell_id: 2387,
            name: "Linen Cloak".into(),
            difficulty: TradeSkillDifficulty::Optimal,
            num_available: 1,
            icon: Some(r"Interface\Icons\INV_Misc_Cape_01".into()),
            min_made: 1,
            max_made: 1,
            cooldown_secs: None,
            product_item: 2570,
            product_inv_type: 16,
            product_item_level: 5,
            reagents: vec![TradeSkillReagent {
                item: 2589,
                name: Some("Linen Cloth".into()),
                icon: Some(r"Interface\Icons\INV_Fabric_Linen_01".into()),
                need: 2,
                have: 5,
            }],
            tools: vec![],
            description: None,
            alt_verb: None,
        }],
        repeat_count: 1,
        linked: None,
        list_link: None,
    }));

    // Achievement 1 in category 1, with one criterion, earned.
    let mut metadata = WrathAchievementMetadata::default();
    metadata.achievements.insert(
        1,
        WrathAchievement {
            id: 1,
            name: "Level 10".into(),
            category_id: 1,
            required_criteria_count: 1,
            required_faction: -1,
            description: "Reach level 10.".into(),
            points: 10,
            icon: r"Interface\Icons\Achievement_Level_10".into(),
            ..Default::default()
        },
    );
    metadata.criteria.insert(
        1,
        WrathAchievementCriterion {
            id: 1,
            achievement_id: 1,
            criteria_type: 5,
            asset_id: 0,
            required_quantity: 10,
            description: "Level 10".into(),
            flags: 0,
        },
    );
    metadata.linked_criteria.insert(1, vec![1]);
    metadata.categories.push(WrathAchievementCategory {
        id: 1,
        parent_id: -1,
        name: "General".into(),
    });
    s.set_wrath_achievement_metadata(metadata);
    s.set_wrath_achievement_progress(WrathAchievementProgress {
        earned: vec![(1, 0)],
        criteria: std::collections::BTreeMap::from([(1, 10)]),
        faction: 1,
    });

    // An LFG party that can backfill.
    s.set_wrath_group(WrathGroup {
        lfg_dungeon_entry: 0x0100_0001,
        lfg_status: 2,
        ..Default::default()
    });

    // The barber's chair, a refundable purchase, a socketed item, a sanctuary and a comparison.
    s.set_wrath_barber_shop(Some(WrathBarberShop {
        base_cost: 100.0,
        hair_styles: vec![WrathBarberStyle {
            id: 1,
            name: "Hair".into(),
            description: "Short".into(),
            cost_modifier: 1.0,
            data: 0,
        }],
        hair_colors: 4,
        hair_customization: "HAIR".into(),
        facial_customization: "BEARD".into(),
        ..Default::default()
    }));
    let refund = |bag: i32| {
        (
            false,
            bag,
            1,
            ItemRefundView {
                money: 100,
                honor: 10,
                arena: 0,
                seconds_remaining: 3600,
                items: vec![ItemRefundCost {
                    texture: Some(r"Interface\Icons\INV_Misc_Frostemblem_01".into()),
                    count: 2,
                    link: None,
                }],
            },
        )
    };
    s.set_item_refunds(vec![refund(0), refund(1)]);
    s.set_wrath_item_gems(vec![
        (0, 1, [Some(40111), None, None]),
        (EQUIPMENT_BAG, 1, [Some(40111), None, None]),
    ]);

    // Equipment sets and companions.
    s.set_equipment_sets(vec![EquipmentSetView {
        name: "Tank".into(),
        icon: r"Interface\Icons\INV_Shield_06".into(),
        ..Default::default()
    }]);

    // The guild bank's logs and the guild event log.
    s.set_wrath_guild_bank(WrathGuildBankView {
        money: 1_000_000,
        tabs: vec![WrathGuildBankTab {
            name: "Tab 1".into(),
            ..Default::default()
        }],
        money_log: vec![WrathGuildBankMoneyLog {
            kind: 4,
            player: Some("Arthas".into()),
            copper: 500,
            seconds_ago: 7_200,
        }],
        tab_logs: vec![vec![WrathGuildBankItemLog {
            kind: 3,
            player: Some("Arthas".into()),
            link: None,
            count: 2,
            other_tab: 2,
            seconds_ago: 90_000,
        }]],
        ..Default::default()
    });
    s.set_wrath_guild_events(vec![WrathGuildEvent {
        kind: 3,
        player1: Some("Arthas".into()),
        player2: Some("Jaina".into()),
        rank: Some("Officer".into()),
        seconds_ago: 3_600,
    }]);

    // The raid browser on the catalog's dungeon: one group of two and one lone player, and an
    // encounter of the dungeon's map.
    let member = |guid: u64, name: &str| WrathRaidBrowserMember {
        guid,
        name: Some(name.into()),
        class_name: Some("Mage".into()),
        class_file: Some("MAGE".into()),
        level: 80,
        area: Some("Dalaran".into()),
        comment: "LF healer".into(),
        roles: 8,
        ..Default::default()
    };
    s.set_wrath_raid_browser(std::sync::Arc::new(WrathRaidBrowserView {
        dungeon: 0x0100_0001,
        groups: vec![WrathRaidBrowserGroup {
            guid: 0x0100_0000_0000_0010,
            comment: "LF healer".into(),
            encounter_mask: 1,
            members: vec![
                member(0x0100_0000_0000_0010, "Jaina"),
                member(0x0100_0000_0000_0011, "Thrall"),
            ],
        }],
        players: vec![member(0x0100_0000_0000_0012, "Sylvanas")],
        total_groups: 1,
        total_players: 1,
    }));
    s.set_wrath_lfg_encounters(std::sync::Arc::new(vec![WrathLfgEncounter {
        map: 0,
        difficulty: 0,
        bit: 0,
        name: "Taragaman the Hungerer".into(),
        icon: None,
    }]));

    // Battlefields, vehicles, glyphs, threat, quest POI icons, inspect arena teams and zone PvP.
    battlefield(s);
    vehicles(s);
    glyphs(s);
    threat(s);
    quest_poi(s);
    inspect_arena(s);
    s.set_zone_texts(ZoneTexts {
        zone: "Elwynn Forest".into(),
        real_zone: "Elwynn Forest".into(),
        minimap: "Elwynn Forest".into(),
        pvp_type: Some("friendly".into()),
        pvp_faction: Some("Alliance".into()),
        ..Default::default()
    });
    // Lua-side state: a saved chat position, the comparison unit and an item on the cursor.
    s.run(
        "SetChatWindowSavedPosition(1, 'TOPLEFT', 10, -10) \
         SetChatWindowSavedDimensions(1, 430, 120) \
         SetAchievementComparisonUnit('player') \
         PickupContainerItem(1, 1)",
    )
    .expect("fixture verbs");
    // The compared player's progress, after the comparison unit is set.
    s.set_wrath_achievement_comparison(
        ME,
        WrathAchievementProgress {
            earned: vec![(1, 0x0a2b_1c00)],
            criteria: std::collections::BTreeMap::new(),
            faction: 1,
        },
    );
}

/// Alterac Valley, listed, with one instance.
fn battlefield(s: &mut UiScript) {
    s.set_battleground_types(vec![BattlegroundTypeView {
        id: 1,
        name: "Alterac Valley".into(),
        instance_type: 3,
        groups_allowed: true,
        max_group_size: 40,
        min_level: 51,
        can_enter: true,
        description: Some("Fight".into()),
        has_map: true,
        ..Default::default()
    }]);
    s.set_battlefield_list(BattlefieldListView {
        bg_type: 1,
        instances: vec![7],
        ..Default::default()
    });
}

/// The player drives a two-seat siege engine on a battlefield, possessing it with a spell.
fn vehicles(s: &mut UiScript) {
    // The getters key by the player unit's own guid, which the census VM leaves at its default.
    let me = s
        .eval::<Option<String>>("return UnitGUID('player')")
        .ok()
        .flatten()
        .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    const ENGINE: u64 = 0xF150_0000_0000_0003;
    let seat = |slot: u8, occupant: Option<(u64, &str)>| VehicleSeatView {
        vehicle: ENGINE,
        slot,
        flags: 0x0000_0800 | 0x0400_0000,
        flags_b: 0x0000_0020,
        root: slot == 0,
        occupant: occupant.map(|o| o.0),
        occupant_name: occupant.map(|o| o.1.into()),
    };
    s.set_wrath_vehicles(WrathVehicles {
        units: HashMap::from([(
            me,
            UnitVehicleView {
                seated: true,
                using: true,
                controlling: true,
                seat: Some((0x0000_0800, 1)),
                seats: vec![
                    seat(0, Some((me, "Benchone"))),
                    seat(1, Some((ME + 1, "Benchtwo"))),
                ],
            },
        )]),
        has_mover: true,
        battlefield: vec![BattlefieldVehicleView {
            uv: Some((0.5, 0.25)),
            name: Some("Siege Engine".into()),
            vehicle_type: Some("Drive".into()),
            facing: 1.5,
            alive: true,
            ..Default::default()
        }],
        possess_spell: Some(605),
        possess: Some(PossessView {
            texture: Some(r"Interface\Icons\Spell_Shadow_ShadowWordDominate".into()),
            name: "Mind Control".into(),
            cancel_texture: Some(r"Interface\Icons\Spell_ChargeNegative".into()),
            cancelable: true,
        }),
        ..Default::default()
    });
    s.set_vehicle_indicators(HashMap::from([(
        1,
        VehicleIndicatorView {
            texture: r"Interface\Vehicles\SeatIndicator\SiegeEngine".into(),
            seats: vec![(1, 0.5, 0.25), (2, 0.5, 0.75)],
        },
    )]));
}

/// Six sockets, the first a major holding a glyph.
fn glyphs(s: &mut UiScript) {
    let mut sockets: [GlyphSocketView; 6] = Default::default();
    for (i, socket) in sockets.iter_mut().enumerate() {
        socket.enabled = true;
        socket.slot_id = 21 + i as u32;
        socket.slot = Some((u32::from(i % 2 == 1), i as u32));
    }
    sockets[0].glyph_id = 251;
    sockets[0].glyph = Some(GlyphView {
        spell_id: 58388,
        name: "Glyph of Heroic Strike".into(),
        icon: Some(r"Interface\Icons\INV_Glyph_MajorWarrior".into()),
    });
    s.set_glyph_sockets(Some(sockets));
}

/// The player tanks the target.
fn threat(s: &mut UiScript) {
    s.set_threat(ThreatTable {
        mobs: HashMap::from([(
            MOB,
            MobThreat {
                highest: Some(ME),
                members: HashMap::from([(ME, 12_000)]),
                thresholds: HashMap::new(),
            },
        )]),
    });
}

/// Quest 1 marks its objective on the displayed map; the map update fills the visible list.
fn quest_poi(s: &mut UiScript) {
    s.set_wrath_quest_pois(vec![WrathQuestPoi {
        quest_id: 1,
        blobs: vec![WrathQuestPoiBlob {
            objective_index: 0,
            area_id: 12,
            points: vec![(-9_460, 60), (-9_440, 80)],
            ..Default::default()
        }],
    }]);
    s.set_world_loc_projector(Box::new(|_, _, _, _| Some((0.4, 0.6))));
    let _ = s.run("QuestMapUpdateAllQuests()");
}

/// The inspected player's 2v2 team.
fn inspect_arena(s: &mut UiScript) {
    s.set_inspect_arena_teams([
        Some(InspectArenaTeamView {
            info: ArenaTeamInfoView {
                name: "Bench Pair".into(),
                size: 2,
                emblem: [0xff00_0000, 1, 0xffff_ffff, 1, 0xffff_ffff],
            },
            rating: 1500,
            season_games: 10,
            season_wins: 6,
            player_games: 10,
            player_rating: 1520,
        }),
        None,
        None,
    ]);

    // The player known by GUID, a binding, and the mail's latest senders.
    s.set_unit_by_guid(
        ME,
        Some(UnitState {
            name: Some("Benchone".into()),
            race: Some("Human".into()),
            race_file: Some("Human".into()),
            class: Some("Warrior".into()),
            class_file: Some("WARRIOR".into()),
            sex: 2,
            ..Default::default()
        }),
    );
    s.run("SetBinding('W', 'MOVEFORWARD') SetBinding('UP', 'MOVEFORWARD')")
        .unwrap();
    s.set_wrath_mail_latest_senders(vec!["Arthas".into(), "Jaina".into(), "Thrall".into()]);

    // A native loot roll whose template has landed.
    s.set_wrath_loot_rolls(WrathLootRollsState {
        rolls: vec![WrathLootRollEntry {
            roll_id: 1,
            entry: 2589,
            count: 1,
            vote_mask: 0b1111,
            texture: Some(r"Interface\Icons\INV_Fabric_Linen_01".into()),
            name: Some("Linen Cloth".into()),
            quality: Some(1),
            time_left_ms: 60_000,
            reasons: [Some(0), Some(0), Some(0)],
            disenchant_skill_required: Some(0),
            ..Default::default()
        }],
    });

    // A custom channel, selected by its roster query, and its roster as the server answers it.
    s.set_wrath_display_channels(vec![WrathDisplayChannel {
        slot: 1,
        name: "Bench".into(),
        channel_id: 0,
        active: true,
        custom_state: 0,
    }]);
    s.run(
        "for i = 1, GetNumDisplayChannels() do \
            if GetChannelDisplayInfo(i) == 'Bench' then GetChannelRosterInfo(i, 1) end \
         end",
    )
    .unwrap();
    s.receive_wrath_channel_roster(
        1,
        "Bench",
        0,
        vec![WrathChannelRosterMember {
            guid: ME,
            flags: 1,
            name: Some("Benchone".into()),
        }],
    );
}
