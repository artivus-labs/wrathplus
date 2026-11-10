//! Admission census for every implemented Wrath command variant. Packet codecs retain
//! their own validation; representative valid values exercise route/admission coherence.
use super::*;

#[test]
fn every_wrath_writer_route_is_admitted() {
    let (sender, _receiver) = crossbeam_channel::unbounded();
    let commands = crate::net::NetCommands(sender);
    let rows = vec![
        (
            "SetFactionAtWar",
            ClientCommand::SetFactionAtWar {
                rep_list_id: 7,
                at_war: true,
            },
        ),
        (
            "SetFactionInactive",
            ClientCommand::SetFactionInactive {
                rep_list_id: 7,
                inactive: true,
            },
        ),
        (
            "SetWatchedFaction",
            ClientCommand::SetWatchedFaction { rep_list_id: -1 },
        ),
        (
            "NativeInspect",
            ClientCommand::NativeInspect(
                wrathplus_protocol::wrath_inspect::Command::LearnPreviewTalents {
                    talents: vec![(1, 0)],
                },
            ),
        ),
        ("Inspect", ClientCommand::Inspect { target: 1 }),
        (
            "LearnTalent",
            ClientCommand::LearnTalent {
                talent_id: 1,
                rank: 0,
            },
        ),
        (
            "TalentWipeConfirm",
            ClientCommand::TalentWipeConfirm { trainer: 1 },
        ),
        (
            "NativeSessionMeta",
            ClientCommand::NativeSessionMeta(
                wrathplus_protocol::wrath_session_meta::Command::ReadyForAccountDataTimes,
            ),
        ),
        ("TutorialFlag", ClientCommand::TutorialFlag { id: 1 }),
        (
            "NativeHelp",
            ClientCommand::NativeHelp(wrathplus_protocol::wrath_help::Command::ResolveResponse),
        ),
        (
            "GameObjUse",
            ClientCommand::GameObjUse {
                guid: 0xf110_0000_0000_0007,
            },
        ),
        ("QueryTime", ClientCommand::QueryTime),
        (
            "UnlearnSkill",
            ClientCommand::UnlearnSkill { skill_id: 186 },
        ),
        (
            "NativeSessionMisc",
            ClientCommand::NativeSessionMisc(
                wrathplus_protocol::wrath_session_misc::Command::Emote { emote: 3 },
            ),
        ),
        (
            "NativeWorldObjects",
            ClientCommand::NativeWorldObjects(
                wrathplus_protocol::wrath_world_objects::Command::ReportUse { object: 1 },
            ),
        ),
        (
            "NativeReferFriend",
            ClientCommand::NativeReferFriend(
                wrathplus_protocol::wrath_refer_friend::Command::AcceptLevelGrant { granter: 1 },
            ),
        ),
        ("GmTicketGet", ClientCommand::GmTicketGet),
        ("GmTicketDelete", ClientCommand::GmTicketDelete),
        ("GmTicketSystemStatus", ClientCommand::GmTicketSystemStatus),
        ("TutorialClear", ClientCommand::TutorialClear),
        ("TutorialReset", ClientCommand::TutorialReset),
        ("PlayedTime", ClientCommand::PlayedTime),
        ("NextCinematicCamera", ClientCommand::NextCinematicCamera),
        ("CompleteCinematic", ClientCommand::CompleteCinematic),
        ("AreaTrigger", ClientCommand::AreaTrigger { trigger_id: 45 }),
        (
            "SetActionBarToggles",
            ClientCommand::SetActionBarToggles { toggles: 1 },
        ),
        (
            "NativeDeath",
            ClientCommand::NativeDeath(
                wrathplus_protocol::wrath_death::Command::HearthAndResurrect,
            ),
        ),
        ("RepopRequest", ClientCommand::RepopRequest),
        ("CorpseQuery", ClientCommand::CorpseQuery),
        ("ReclaimCorpse", ClientCommand::ReclaimCorpse { corpse: 1 }),
        (
            "SummonResponse",
            ClientCommand::SummonResponse { summoner: 1 },
        ),
        ("SelfRes", ClientCommand::SelfRes),
        ("FriendListRequest", ClientCommand::FriendListRequest),
        ("AddFriend", ClientCommand::AddFriend { name: "Bob".into() }),
        ("DelFriend", ClientCommand::DelFriend { guid: 1 }),
        ("AddIgnore", ClientCommand::AddIgnore { name: "Bob".into() }),
        ("DelIgnore", ClientCommand::DelIgnore { guid: 1 }),
        (
            "SetContactNotes",
            ClientCommand::SetContactNotes {
                guid: 1,
                note: "tank".into(),
            },
        ),
        (
            "GuildCreate",
            ClientCommand::GuildCreate { name: "G".into() },
        ),
        ("GuildLeave", ClientCommand::GuildLeave),
        ("GuildDisband", ClientCommand::GuildDisband),
        (
            "GuildLeader",
            ClientCommand::GuildLeader { name: "Bob".into() },
        ),
        (
            "GuildAddRank",
            ClientCommand::GuildAddRank { name: "Alt".into() },
        ),
        ("GuildDelRank", ClientCommand::GuildDelRank),
        (
            "GuildSetPublicNote",
            ClientCommand::GuildSetPublicNote {
                name: "Bob".into(),
                note: "n".into(),
            },
        ),
        (
            "GuildSetOfficerNote",
            ClientCommand::GuildSetOfficerNote {
                name: "Bob".into(),
                note: "n".into(),
            },
        ),
        (
            "GuildInfoText",
            ClientCommand::GuildInfoText { text: "hi".into() },
        ),
        (
            "SaveGuildEmblem",
            ClientCommand::SaveGuildEmblem {
                vendor: 1,
                design: [0; 5],
            },
        ),
        (
            "TabardVendorActivate",
            ClientCommand::TabardVendorActivate { npc: 1 },
        ),
        ("GuildEventLogQuery", ClientCommand::GuildEventLogQuery),
        (
            "BattlefieldStatusRequest",
            ClientCommand::BattlefieldStatusRequest,
        ),
        ("ClearChannelWatch", ClientCommand::ClearChannelWatch),
        (
            "DeclineChannelInvite",
            ClientCommand::DeclineChannelInvite {
                name: "Trade".into(),
            },
        ),
        (
            "NativeRaidBrowser",
            ClientCommand::NativeRaidBrowser(
                wrathplus_protocol::wrath_raid_browser::Command::Join { dungeon: 251 },
            ),
        ),
        (
            "PetitionShowList",
            ClientCommand::PetitionShowList { npc: 1 },
        ),
        (
            "PetitionBuy",
            ClientCommand::PetitionBuy {
                npc: 1,
                name: "G".into(),
                index: 1,
            },
        ),
        (
            "PetitionShowSignatures",
            ClientCommand::PetitionShowSignatures { item: 1 },
        ),
        (
            "PetitionSign",
            ClientCommand::PetitionSign { item: 1, byte: 1 },
        ),
        (
            "OfferPetition",
            ClientCommand::OfferPetition { item: 1, player: 2 },
        ),
        (
            "TurnInPetition",
            ClientCommand::TurnInPetition {
                item: 1,
                banner: None,
            },
        ),
        (
            "PetitionQuery",
            ClientCommand::PetitionQuery {
                petition_id: 1,
                item: 1,
            },
        ),
        (
            "PetitionRename",
            ClientCommand::PetitionRename {
                item: 1,
                name: "N".into(),
            },
        ),
        (
            "PetitionDecline",
            ClientCommand::PetitionDecline { item: 1 },
        ),
        (
            "NativeAppearance",
            ClientCommand::NativeAppearance(
                wrathplus_protocol::wrath_appearance::Command::ShowingHelm { show: true },
            ),
        ),
        (
            "NativeGuildBankSwap",
            ClientCommand::NativeGuildBankSwap(wrathplus_protocol::wrath_guild_bank_swap::Swap {
                banker: 1,
                transfer: wrathplus_protocol::wrath_guild_bank_swap::Transfer::AutoStore {
                    source: wrathplus_protocol::wrath_guild_bank_swap::BankSlot {
                        tab: 0,
                        slot: 0,
                        item: 1,
                    },
                    count: 1,
                },
            }),
        ),
        (
            "NativeGuildBankTab",
            ClientCommand::NativeGuildBankTab(
                wrathplus_protocol::wrath_guild_bank_tab::UpdateTab {
                    banker: 1,
                    tab: 0,
                    name: b"Mats".to_vec(),
                    icon: b"INV_Box_01".to_vec(),
                },
            ),
        ),
        (
            "NativeGuildBankText",
            ClientCommand::NativeGuildBankText(
                wrathplus_protocol::wrath_guild_bank_text::Command::QueryText { tab: 0 },
            ),
        ),
        (
            "NativeGuildRank",
            ClientCommand::NativeGuildRank(wrathplus_protocol::wrath_guild_rank::SaveRank {
                rank: 1,
                flags: 0x41,
                name: b"Officer".to_vec(),
                money_limit: 0,
                tabs: std::array::from_fn(|_| {
                    wrathplus_protocol::wrath_guild_rank::TabPermission {
                        rights: 0,
                        item_limit: 0,
                    }
                }),
            }),
        ),
        (
            "NativeGuildBank",
            ClientCommand::NativeGuildBank(
                wrathplus_protocol::wrath_guild_bank_commands::Command::BuyTab {
                    banker: 1,
                    tab: 0,
                },
            ),
        ),
        ("DuelAccepted", ClientCommand::DuelAccepted { arbiter: 1 }),
        ("DuelCancelled", ClientCommand::DuelCancelled { arbiter: 1 }),
        (
            "NativeBattleground",
            ClientCommand::NativeBattleground(
                wrathplus_protocol::wrath_battleground::Command::BfExit { battle_id: 1 },
            ),
        ),
        (
            "BattlegroundJoin",
            ClientCommand::NativeBattleground(
                wrathplus_protocol::wrath_battleground::Command::Join {
                    battlemaster: 1,
                    bg_type: 2,
                    instance_id: 0,
                    as_group: 0,
                },
            ),
        ),
        (
            "BattlemasterHello",
            ClientCommand::BattlemasterHello { npc: 1 },
        ),
        (
            "AreaSpiritHealerQuery",
            ClientCommand::AreaSpiritHealerQuery { healer: 1 },
        ),
        (
            "AreaSpiritHealerQueue",
            ClientCommand::AreaSpiritHealerQueue { healer: 1 },
        ),
        (
            "SpiritHealerActivate",
            ClientCommand::SpiritHealerActivate { npc: 1 },
        ),
        (
            "InspectHonorStats",
            ClientCommand::InspectHonorStats { target: 1 },
        ),
        ("TogglePvp", ClientCommand::TogglePvp),
        (
            "RequestBattlefieldScoreData",
            ClientCommand::RequestBattlefieldScoreData,
        ),
        (
            "RequestBattlefieldPositions",
            ClientCommand::RequestBattlefieldPositions,
        ),
        ("LeaveBattlefield", ClientCommand::LeaveBattlefield),
        (
            "Chat",
            ClientCommand::Chat {
                kind: crate::net::ChatKind::Say,
                target: None,
                text: "hi".into(),
                language: None,
            },
        ),
        (
            "AddonMessage",
            ClientCommand::AddonMessage {
                distribution: wrathplus_ui::script::AddonDistribution::Party,
                text: "p\tm".into(),
                target: None,
            },
        ),
        (
            "JoinChannel",
            ClientCommand::JoinChannel {
                name: "General - Elwynn Forest".into(),
                password: String::new(),
                channel_id: 1,
            },
        ),
        (
            "LeaveChannel",
            ClientCommand::LeaveChannel { name: "x".into() },
        ),
        (
            "ChannelList",
            ClientCommand::ChannelList { name: "x".into() },
        ),
        (
            "ChannelOwner",
            ClientCommand::ChannelOwner { name: "x".into() },
        ),
        (
            "ChannelSetOwner",
            ClientCommand::ChannelSetOwner {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelPassword",
            ClientCommand::ChannelPassword {
                name: "x".into(),
                password: "p".into(),
            },
        ),
        (
            "ChannelModerator",
            ClientCommand::ChannelModerator {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelUnmoderator",
            ClientCommand::ChannelUnmoderator {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelMute",
            ClientCommand::ChannelMute {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelUnmute",
            ClientCommand::ChannelUnmute {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelInvite",
            ClientCommand::ChannelInvite {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelKick",
            ClientCommand::ChannelKick {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelBan",
            ClientCommand::ChannelBan {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelUnban",
            ClientCommand::ChannelUnban {
                name: "x".into(),
                player: "Bob".into(),
            },
        ),
        (
            "ChannelAnnouncements",
            ClientCommand::ChannelAnnouncements { name: "x".into() },
        ),
        (
            "ChannelModerate",
            ClientCommand::ChannelModerate { name: "x".into() },
        ),
        (
            "TextEmote",
            ClientCommand::TextEmote {
                text_id: 101,
                target: 0,
            },
        ),
        ("ChatIgnored", ClientCommand::ChatIgnored { guid: 1 }),
        (
            "NativeParty",
            ClientCommand::NativeParty(
                wrathplus_protocol::wrath_party::Command::ReadyCheckFinished,
            ),
        ),
        (
            "PartyUninviteGuid",
            ClientCommand::NativeParty(wrathplus_protocol::wrath_party::Command::UninviteGuid {
                guid: 1,
                reason: String::new(),
            }),
        ),
        (
            "GroupUninvite",
            ClientCommand::GroupUninvite { name: "Bob".into() },
        ),
        ("GroupSetLeader", ClientCommand::GroupSetLeader { guid: 1 }),
        ("MinimapPing", ClientCommand::MinimapPing { x: 1.0, y: 2.0 }),
        ("RandomRoll", ClientCommand::RandomRoll { min: 1, max: 100 }),
        (
            "NativeQuest",
            ClientCommand::NativeQuest(
                wrathplus_protocol::wrath_quest::Command::StatusMultipleQuery,
            ),
        ),
        (
            "QuestPoiQuery",
            ClientCommand::NativeQuest(wrathplus_protocol::wrath_quest::Command::PoiQuery {
                quests: vec![783],
            }),
        ),
        (
            "QuestPushResult",
            ClientCommand::QuestPushResult {
                sharer: 1,
                quest: 783,
                msg: wrathplus_protocol::types::QuestShareMsg::DECLINE_QUEST,
            },
        ),
        (
            "NativeCalendar",
            ClientCommand::NativeCalendar(wrathplus_protocol::wrath_calendar::Command::GetCalendar),
        ),
        (
            "CalendarRsvp",
            ClientCommand::NativeCalendar(wrathplus_protocol::wrath_calendar::Command::Rsvp {
                event_id: 1,
                invite_id: 2,
                status: 1,
            }),
        ),
        (
            "NativeInspectAchievements",
            ClientCommand::NativeInspectAchievements { player: 1 },
        ),
        (
            "NativeExchange",
            ClientCommand::NativeExchange(
                wrathplus_protocol::wrath_exchange::Command::OptOutOfLoot { pass: 1 },
            ),
        ),
        (
            "ExchangePendingSales",
            ClientCommand::NativeExchange(
                wrathplus_protocol::wrath_exchange::Command::AuctionListPendingSales {
                    auctioneer: 1,
                },
            ),
        ),
        (
            "AuctionHello",
            ClientCommand::AuctionHello { auctioneer: 1 },
        ),
        (
            "AuctionListItems",
            ClientCommand::AuctionListItems {
                auctioneer: 1,
                list_from: 0,
                searched_name: "Linen".into(),
                level_min: 0,
                level_max: 0,
                slot_id: u32::MAX,
                main_category: u32::MAX,
                sub_category: u32::MAX,
                quality: u32::MAX,
                usable: 0,
            },
        ),
        (
            "AuctionListOwnerItems",
            ClientCommand::AuctionListOwnerItems {
                auctioneer: 1,
                list_from: 0,
            },
        ),
        (
            "AuctionListBidderItems",
            ClientCommand::AuctionListBidderItems {
                auctioneer: 1,
                list_from: 0,
                auction_ids: vec![2],
            },
        ),
        (
            "AuctionSellItem",
            ClientCommand::AuctionSellItem {
                auctioneer: 1,
                items: vec![(2, 1)],
                bid: 3,
                buyout: 4,
                etime_minutes: 720,
            },
        ),
        (
            "AuctionPlaceBid",
            ClientCommand::AuctionPlaceBid {
                auctioneer: 1,
                auction_id: 2,
                price: 3,
            },
        ),
        (
            "AuctionRemoveItem",
            ClientCommand::AuctionRemoveItem {
                auctioneer: 1,
                auction_id: 2,
            },
        ),
        (
            "LootMasterGive",
            ClientCommand::LootMasterGive {
                guid: 1,
                slot: 0,
                target: 2,
            },
        ),
        ("InitiateTrade", ClientCommand::InitiateTrade { target: 1 }),
        ("BeginTrade", ClientCommand::BeginTrade),
        ("BusyTrade", ClientCommand::BusyTrade),
        ("IgnoreTrade", ClientCommand::IgnoreTrade),
        ("AcceptTrade", ClientCommand::AcceptTrade),
        ("UnacceptTrade", ClientCommand::UnacceptTrade),
        ("CancelTrade", ClientCommand::CancelTrade),
        ("SetTradeGold", ClientCommand::SetTradeGold { copper: 1 }),
        (
            "SetTradeItem",
            ClientCommand::SetTradeItem {
                trade_slot: 0,
                bag: 255,
                slot: 23,
            },
        ),
        (
            "ClearTradeItem",
            ClientCommand::ClearTradeItem { trade_slot: 0 },
        ),
        (
            "NativeInventory",
            ClientCommand::NativeInventory(
                wrathplus_protocol::wrath_inventory::Command::ReadItem { bag: 255, slot: 23 },
            ),
        ),
        (
            "InventoryEquipmentSetDelete",
            ClientCommand::NativeInventory(
                wrathplus_protocol::wrath_inventory::Command::EquipmentSetDelete { set_guid: 1 },
            ),
        ),
        (
            "InventoryRefund",
            ClientCommand::NativeInventory(wrathplus_protocol::wrath_inventory::Command::Refund {
                item_guid: 1,
            }),
        ),
        (
            "UseItem",
            ClientCommand::UseItem {
                bag_index: 255,
                slot: 23,
                spell_id: 8690,
                item_guid: 1,
                glyph_index: 0,
                target: wrathplus_protocol::types::UseItemTarget::SelfImplicit,
            },
        ),
        (
            "OpenItem",
            ClientCommand::OpenItem {
                bag_index: 255,
                slot: 23,
            },
        ),
        (
            "WrapItem",
            ClientCommand::WrapItem {
                gift_bag: 255,
                gift_slot: 23,
                item_bag: 255,
                item_slot: 24,
            },
        ),
        (
            "AutoEquipItem",
            ClientCommand::AutoEquipItem {
                bag_index: 255,
                slot: 23,
            },
        ),
        ("SetAmmo", ClientCommand::SetAmmo { entry: 2512 }),
        (
            "SwapInvItem",
            ClientCommand::SwapInvItem {
                src_slot: 23,
                dst_slot: 24,
            },
        ),
        (
            "SwapItem",
            ClientCommand::SwapItem {
                dst_bag: 19,
                dst_slot: 0,
                src_bag: 255,
                src_slot: 23,
            },
        ),
        (
            "AutoStoreBagItem",
            ClientCommand::AutoStoreBagItem {
                src_bag: 255,
                src_slot: 23,
                dst_bag: 19,
            },
        ),
        (
            "SplitItem",
            ClientCommand::SplitItem {
                src_bag: 255,
                src_slot: 23,
                dst_bag: 255,
                dst_slot: 24,
                count: 1,
            },
        ),
        (
            "DestroyItem",
            ClientCommand::DestroyItem {
                bag_index: 255,
                slot: 23,
                count: 0,
            },
        ),
        ("BankerActivate", ClientCommand::BankerActivate { guid: 1 }),
        ("BuyBankSlot", ClientCommand::BuyBankSlot { guid: 1 }),
        (
            "AutoBankItem",
            ClientCommand::AutoBankItem { bag: 255, slot: 23 },
        ),
        (
            "AutoStoreBankItem",
            ClientCommand::AutoStoreBankItem { bag: 255, slot: 39 },
        ),
        (
            "PageTextQuery",
            ClientCommand::PageTextQuery {
                page_id: 1,
                guid: 1,
            },
        ),
        (
            "NativeInstanceControl",
            ClientCommand::NativeInstanceControl(
                wrathplus_protocol::wrath_instance_control::Command::ResetInstances,
            ),
        ),
        (
            "InstanceDungeonDifficulty",
            ClientCommand::NativeInstanceControl(
                wrathplus_protocol::wrath_instance_control::Command::SetDungeonDifficulty {
                    mode: 1,
                },
            ),
        ),
        (
            "InstanceRaidDifficulty",
            ClientCommand::NativeInstanceControl(
                wrathplus_protocol::wrath_instance_control::Command::SetRaidDifficulty { mode: 3 },
            ),
        ),
        (
            "InstanceLockResponse",
            ClientCommand::NativeInstanceControl(
                wrathplus_protocol::wrath_instance_control::Command::LockResponse { raw_accept: 1 },
            ),
        ),
        (
            "InstanceExtend",
            ClientCommand::NativeInstanceControl(
                wrathplus_protocol::wrath_instance_control::Command::SetSavedInstanceExtend {
                    map: 1,
                    difficulty: 3,
                    raw_extend: 1,
                },
            ),
        ),
        (
            "NativeMirrorImageQuery",
            ClientCommand::NativeMirrorImageQuery { guid: u64::MAX },
        ),
        (
            "NativeWhoCommand",
            ClientCommand::NativeWhoCommand(wrathplus_protocol::wrath_who::Command::Whois {
                name: b"Synthetic".to_vec(),
            }),
        ),
        ("NativeUiTimerQuery", ClientCommand::NativeUiTimerQuery),
        (
            "GuildQuery",
            ClientCommand::GuildQuery {
                guild_id: Default::default(),
            },
        ),
        ("GuildInfoRequest", ClientCommand::GuildInfoRequest),
        ("GuildRosterRequest", ClientCommand::GuildRosterRequest),
        (
            "GuildInvite",
            ClientCommand::GuildInvite {
                name: Default::default(),
            },
        ),
        ("GuildAccept", ClientCommand::GuildAccept),
        ("GuildDecline", ClientCommand::GuildDecline),
        (
            "GuildPromote",
            ClientCommand::GuildPromote {
                name: Default::default(),
            },
        ),
        (
            "GuildDemote",
            ClientCommand::GuildDemote {
                name: Default::default(),
            },
        ),
        (
            "GuildRemove",
            ClientCommand::GuildRemove {
                name: Default::default(),
            },
        ),
        (
            "GuildMotd",
            ClientCommand::GuildMotd {
                motd: Default::default(),
            },
        ),
        (
            "NativeQuestsCompletedQuery",
            ClientCommand::NativeQuestsCompletedQuery,
        ),
        (
            "NativeWhoCommand",
            ClientCommand::NativeWhoCommand(wrathplus_protocol::wrath_who::Command::Whois {
                name: b"Benchone".to_vec(),
            }),
        ),
        (
            "NativeMailMutation",
            ClientCommand::NativeMailMutation(
                wrathplus_protocol::wrath_mail_mutations::Command::TakeMoney {
                    mailbox: 1,
                    mail_id: 2,
                },
            ),
        ),
        (
            "PetNameQuery",
            ClientCommand::PetNameQuery {
                pet_number: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "PetAction",
            ClientCommand::PetAction {
                pet_guid: 1,
                packed: 0x0700_0003,
                target_guid: 2,
            },
        ),
        (
            "PetSetAction",
            ClientCommand::PetSetAction {
                pet_guid: 1,
                entries: vec![(3, 0xC100_0001)],
            },
        ),
        (
            "PetStopAttack",
            ClientCommand::PetStopAttack { pet_guid: 1 },
        ),
        (
            "PetCancelAura",
            ClientCommand::PetCancelAura {
                pet_guid: 1,
                spell_id: 7,
            },
        ),
        (
            "PetSpellAutocast",
            ClientCommand::PetSpellAutocast {
                pet_guid: 1,
                spell_id: 7,
                enabled: true,
            },
        ),
        ("PetAbandon", ClientCommand::PetAbandon { pet_guid: 1 }),
        (
            "PetRename",
            ClientCommand::PetRename {
                pet_guid: 1,
                name: "Rex".into(),
            },
        ),
        ("CancelAutoRepeat", ClientCommand::CancelAutoRepeat),
        (
            "TeleportAck",
            ClientCommand::TeleportAck {
                guid: 1,
                counter: 2,
            },
        ),
        ("MountSpecial", ClientCommand::MountSpecial),
        (
            "MoveTimeSkipped",
            ClientCommand::MoveTimeSkipped {
                guid: 1,
                lag_ms: 250,
            },
        ),
        (
            "NativeMovementMisc",
            ClientCommand::NativeMovementMisc(
                wrathplus_protocol::wrath_movement_misc::Command::TaxiBenchmark { on: true },
            ),
        ),
        (
            "MovementVehicleExit",
            ClientCommand::NativeMovementMisc(
                wrathplus_protocol::wrath_movement_misc::Command::VehicleExit,
            ),
        ),
        (
            "MovementVehicleSwitchSeat",
            ClientCommand::NativeMovementMisc(
                wrathplus_protocol::wrath_movement_misc::Command::VehicleSwitchSeat {
                    vehicle: 1,
                    seat: 0,
                },
            ),
        ),
        (
            "MovementEjectPassenger",
            ClientCommand::NativeMovementMisc(
                wrathplus_protocol::wrath_movement_misc::Command::EjectPassenger { passenger: 2 },
            ),
        ),
        (
            "CancelChannelling",
            ClientCommand::CancelChannelling { spell_id: 5143 },
        ),
        (
            "ResurrectResponse",
            ClientCommand::ResurrectResponse {
                caster: 1,
                accept: true,
            },
        ),
        ("FarSight", ClientCommand::FarSight { engage: true }),
        (
            "SpellControlDismissCritter",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::DismissCritter { critter: 1 },
            ),
        ),
        (
            "SpellControlStableRevivePet",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::StableRevivePet {
                    npc: 1,
                    pet_number: 2,
                },
            ),
        ),
        (
            "SpellControlPreviewTalentsPet",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::LearnPreviewTalentsPet {
                    pet: 1,
                    talents: vec![(2107, 0)],
                },
            ),
        ),
        (
            "NativeSpellControl",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::SpellClick { unit: 1 },
            ),
        ),
        (
            "SpellControlTotemDestroyed",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::TotemDestroyed { slot: 1 },
            ),
        ),
        (
            "SpellControlCancelMountAura",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::CancelMountAura,
            ),
        ),
        (
            "SpellControlCancelGrowthAura",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::CancelGrowthAura,
            ),
        ),
        (
            "SpellControlRequestPetInfo",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::RequestPetInfo,
            ),
        ),
        (
            "SpellControlPetLearnTalent",
            ClientCommand::NativeSpellControl(
                wrathplus_protocol::wrath_spell_control::Command::PetLearnTalent {
                    pet: 1,
                    talent_id: 2107,
                    rank: 0,
                },
            ),
        ),
        (
            "NativeWorldportAck",
            ClientCommand::NativeWorldportAck(wrathplus_protocol::wrath_transfer::Token {
                epoch: 1,
                sequence: 2,
            }),
        ),
        (
            "ArenaTeamCommand",
            ClientCommand::ArenaTeamCommand(wrathplus_protocol::wrath_arena_team::Command::Accept),
        ),
        (
            "LfgCommand",
            ClientCommand::LfgCommand(wrathplus_protocol::wrath_lfg::commands::Command::GetStatus),
        ),
        (
            "LootRoll",
            ClientCommand::LootRoll {
                looted_target: Default::default(),
                item_slot: Default::default(),
                roll_type: Default::default(),
            },
        ),
        (
            "SetActiveMover",
            ClientCommand::SetActiveMover {
                guid: Default::default(),
            },
        ),
        (
            "NotActiveMover",
            ClientCommand::NotActiveMover {
                guid: Default::default(),
                pos: [0.0; 3],
                orientation: 0.0,
            },
        ),
        (
            "NativePetCastSpell",
            ClientCommand::NativePetCastSpell {
                caster: 0x50,
                spell_id: 1,
                target: wrathplus_protocol::wrath_session::CastTarget::Dest([0.0; 3]),
            },
        ),
        (
            "NativeVehicleControl",
            ClientCommand::NativeVehicleControl(
                wrathplus_protocol::wrath_authority::VehicleControl::Dismiss,
            ),
        ),
        ("Logout", ClientCommand::Logout),
        ("ForceLogout", ClientCommand::ForceLogout),
        ("LogoutCancel", ClientCommand::LogoutCancel),
        (
            "Move",
            ClientCommand::Move {
                kind: crate::net::MoveKind::Heartbeat,
                flags: Default::default(),
                pos: Default::default(),
                orientation: Default::default(),
                pitch: Default::default(),
                fall_time: Default::default(),
                jump: Default::default(),
                transport: Default::default(),
            },
        ),
        (
            "ItemQuery",
            ClientCommand::ItemQuery {
                entry: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "Loot",
            ClientCommand::Loot {
                guid: Default::default(),
            },
        ),
        (
            "AutostoreLootItem",
            ClientCommand::AutostoreLootItem {
                slot: Default::default(),
            },
        ),
        ("LootMoney", ClientCommand::LootMoney),
        (
            "LootRelease",
            ClientCommand::LootRelease {
                guid: Default::default(),
            },
        ),
        (
            "SetSelection",
            ClientCommand::SetSelection {
                guid: Default::default(),
            },
        ),
        (
            "AttackSwing",
            ClientCommand::AttackSwing {
                guid: Default::default(),
            },
        ),
        ("AttackStop", ClientCommand::AttackStop),
        (
            "StandStateChange",
            ClientCommand::StandStateChange {
                state: Default::default(),
            },
        ),
        (
            "SetSheathed",
            ClientCommand::SetSheathed {
                state: Default::default(),
            },
        ),
        (
            "GroupInvite",
            ClientCommand::GroupInvite {
                name: Default::default(),
            },
        ),
        ("GroupAccept", ClientCommand::GroupAccept),
        ("GroupDecline", ClientCommand::GroupDecline),
        (
            "PartyAssignment",
            ClientCommand::PartyAssignment {
                assignment: Default::default(),
                apply: Default::default(),
                guid: Default::default(),
            },
        ),
        ("GroupLeave", ClientCommand::GroupLeave),
        ("GroupRaidConvert", ClientCommand::GroupRaidConvert),
        (
            "LootMethod",
            ClientCommand::LootMethod {
                method: Default::default(),
                master: Default::default(),
                threshold: Default::default(),
            },
        ),
        (
            "SetRaidTarget",
            ClientCommand::SetRaidTarget {
                icon: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "GroupChangeSubGroup",
            ClientCommand::GroupChangeSubGroup {
                name: Default::default(),
                group: Default::default(),
            },
        ),
        (
            "GroupSwapSubGroup",
            ClientCommand::GroupSwapSubGroup {
                name: Default::default(),
                other: Default::default(),
            },
        ),
        (
            "GroupAssistantLeader",
            ClientCommand::GroupAssistantLeader {
                guid: Default::default(),
                grant: Default::default(),
            },
        ),
        ("ReadyCheckStart", ClientCommand::ReadyCheckStart),
        (
            "ReadyCheckAnswer",
            ClientCommand::ReadyCheckAnswer {
                ready: Default::default(),
            },
        ),
        (
            "RequestPartyMemberStats",
            ClientCommand::RequestPartyMemberStats {
                guid: Default::default(),
            },
        ),
        (
            "NameQuery",
            ClientCommand::NameQuery {
                guid: Default::default(),
            },
        ),
        (
            "CreatureQuery",
            ClientCommand::CreatureQuery {
                entry: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "GameObjectQuery",
            ClientCommand::GameObjectQuery {
                entry: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "SetActionButton",
            ClientCommand::SetActionButton {
                button: Default::default(),
                packed: Default::default(),
            },
        ),
        (
            "CastSpell",
            ClientCommand::CastSpell {
                spell_id: Default::default(),
                target: Default::default(),
            },
        ),
        (
            "CastSpellAtDest",
            ClientCommand::CastSpellAtDest {
                spell_id: Default::default(),
                dest: Default::default(),
            },
        ),
        (
            "CastSpellAtSource",
            ClientCommand::CastSpellAtSource {
                spell_id: Default::default(),
                src: Default::default(),
            },
        ),
        (
            "CastSpellGameObject",
            ClientCommand::CastSpellGameObject {
                spell_id: Default::default(),
                go_guid: Default::default(),
            },
        ),
        (
            "CastSpellItem",
            ClientCommand::CastSpellItem {
                spell_id: Default::default(),
                item_guid: Default::default(),
            },
        ),
        (
            "CastSpellCorpse",
            ClientCommand::CastSpellCorpse {
                spell_id: Default::default(),
                target: wrathplus_protocol::types::CorpseTarget::Ally,
                corpse_guid: Default::default(),
            },
        ),
        (
            "CancelCast",
            ClientCommand::CancelCast {
                spell_id: Default::default(),
            },
        ),
        (
            "CancelAura",
            ClientCommand::CancelAura {
                spell_id: Default::default(),
            },
        ),
        (
            "GossipHello",
            ClientCommand::GossipHello {
                guid: Default::default(),
            },
        ),
        ("GossipSelectOption", ClientCommand::gossip_select(0, 0)),
        (
            "NpcTextQuery",
            ClientCommand::NpcTextQuery {
                text_id: Default::default(),
                guid: Default::default(),
            },
        ),
        (
            "QuestgiverStatusQuery",
            ClientCommand::QuestgiverStatusQuery {
                npc: Default::default(),
            },
        ),
        (
            "QuestgiverHello",
            ClientCommand::QuestgiverHello {
                npc: Default::default(),
            },
        ),
        (
            "QuestgiverQuery",
            ClientCommand::QuestgiverQuery {
                npc: Default::default(),
                quest: Default::default(),
            },
        ),
        (
            "QuestgiverAccept",
            ClientCommand::QuestgiverAccept {
                npc: Default::default(),
                quest: Default::default(),
            },
        ),
        (
            "QuestgiverComplete",
            ClientCommand::QuestgiverComplete {
                npc: Default::default(),
                quest: Default::default(),
            },
        ),
        (
            "QuestgiverRequestReward",
            ClientCommand::QuestgiverRequestReward {
                npc: Default::default(),
                quest: Default::default(),
            },
        ),
        (
            "QuestgiverChooseReward",
            ClientCommand::QuestgiverChooseReward {
                npc: Default::default(),
                quest: Default::default(),
                choice: Default::default(),
            },
        ),
        (
            "QuestQuery",
            ClientCommand::QuestQuery {
                quest: Default::default(),
            },
        ),
        (
            "QuestlogRemove",
            ClientCommand::QuestlogRemove {
                slot: Default::default(),
            },
        ),
        (
            "PushQuestToParty",
            ClientCommand::PushQuestToParty {
                quest: Default::default(),
            },
        ),
        (
            "QuestConfirmAccept",
            ClientCommand::QuestConfirmAccept {
                quest: Default::default(),
            },
        ),
        (
            "ListInventory",
            ClientCommand::ListInventory {
                guid: Default::default(),
            },
        ),
        (
            "BuyItem",
            ClientCommand::BuyItem {
                vendor: Default::default(),
                entry: Default::default(),
                count: Default::default(),
            },
        ),
        (
            "BuyItemInSlot",
            ClientCommand::BuyItemInSlot {
                vendor: Default::default(),
                entry: Default::default(),
                bag_guid: Default::default(),
                bag_slot: Default::default(),
                count: Default::default(),
            },
        ),
        (
            "SellItem",
            ClientCommand::SellItem {
                vendor: Default::default(),
                item_guid: Default::default(),
                count: Default::default(),
            },
        ),
        (
            "BuybackItem",
            ClientCommand::BuybackItem {
                vendor: Default::default(),
                slot: Default::default(),
            },
        ),
        (
            "RepairItem",
            ClientCommand::RepairItem {
                vendor: Default::default(),
                item_guid: Default::default(),
            },
        ),
        (
            "TaxiNodeStatusQuery",
            ClientCommand::TaxiNodeStatusQuery {
                guid: Default::default(),
            },
        ),
        (
            "TaxiQueryNodes",
            ClientCommand::TaxiQueryNodes {
                guid: Default::default(),
            },
        ),
        (
            "ActivateTaxi",
            ClientCommand::ActivateTaxi {
                guid: Default::default(),
                source_node: Default::default(),
                dest_node: Default::default(),
            },
        ),
        (
            "ActivateTaxiExpress",
            ClientCommand::ActivateTaxiExpress {
                guid: Default::default(),
                nodes: Default::default(),
            },
        ),
        (
            "BinderActivate",
            ClientCommand::BinderActivate {
                binder: Default::default(),
            },
        ),
        (
            "TrainerList",
            ClientCommand::TrainerList {
                trainer: Default::default(),
            },
        ),
        (
            "ListStabledPets",
            ClientCommand::ListStabledPets {
                npc: Default::default(),
            },
        ),
        (
            "StablePet",
            ClientCommand::StablePet {
                npc: Default::default(),
            },
        ),
        (
            "UnstablePet",
            ClientCommand::UnstablePet {
                npc: Default::default(),
                pet_number: Default::default(),
            },
        ),
        (
            "StableSwapPet",
            ClientCommand::StableSwapPet {
                npc: Default::default(),
                pet_number: Default::default(),
            },
        ),
        (
            "BuyStableSlot",
            ClientCommand::BuyStableSlot {
                npc: Default::default(),
            },
        ),
        (
            "TrainerBuySpell",
            ClientCommand::TrainerBuySpell {
                trainer: Default::default(),
                spell_id: Default::default(),
            },
        ),
        (
            "GetMailList",
            ClientCommand::GetMailList {
                mailbox: Default::default(),
            },
        ),
        (
            "SetChannelWatch",
            ClientCommand::SetChannelWatch {
                name: Default::default(),
            },
        ),
        ("RequestRaidInfo", ClientCommand::RequestRaidInfo),
        (
            "GuildBankQuery",
            ClientCommand::GuildBankQuery(
                wrathplus_protocol::wrath_guild_bank_query::Query::Permissions,
            ),
        ),
        (
            "LfgRequestPlayerLockInfo",
            ClientCommand::LfgRequestPlayerLockInfo,
        ),
        (
            "LfgRequestPartyLockInfo",
            ClientCommand::LfgRequestPartyLockInfo,
        ),
        (
            "ChannelDisplayList",
            ClientCommand::ChannelDisplayList {
                name: Default::default(),
            },
        ),
        ("QueryNextMailTime", ClientCommand::QueryNextMailTime),
        (
            "MoveSplineDone",
            ClientCommand::MoveSplineDone {
                flags: Default::default(),
                pos: Default::default(),
                orientation: Default::default(),
                spline_id: Default::default(),
            },
        ),
    ];
    let mut covered = std::collections::BTreeSet::new();
    for (name, command) in rows {
        assert!(
            has_writer_route(&command),
            "fixture {name} lost its writer route"
        );
        assert!(
            commands.supports(&command),
            "writer route rejected by admission: {name}"
        );
        covered.insert(name);
    }
    // Deliberate exclusion: legacy un-tokened WorldportAck has no native route.
    // NativeWorldportAck above is admitted, while the writer still requires a matching
    // live transfer capability/epoch.
    {
        let command = ClientCommand::WorldportAck;
        assert!(!has_writer_route(&command));
        assert!(!commands.supports(&command));
    }
    // These variants have routes, but invalid field values must remain inadmissible.
    for command in [
        ClientCommand::AutostoreLootItem { slot: 18 },
        ClientCommand::SetSheathed { state: 3 },
        ClientCommand::StandStateChange { state: 2 },
        ClientCommand::Move {
            kind: crate::net::MoveKind::Heartbeat,
            flags: 0,
            pos: [0.; 3],
            orientation: 0.,
            pitch: 1.,
            fall_time: 0,
            jump: None,
            transport: None,
        },
    ] {
        assert!(has_writer_route(&command));
        assert!(!commands.supports(&command));
    }
    // Keep the table exhaustive when a new ClientCommand arm is added to route().
    // This is a structural census, not a codec test: failures request a real fixture.
    let source = include_str!("wrath.rs");
    let route_body = source
        .split("fn route(command:")
        .nth(1)
        .unwrap()
        .split("/// Only implemented wire routes")
        .next()
        .unwrap();
    let disabled = ["WorldportAck"];
    for tail in route_body.split("ClientCommand::").skip(1) {
        let name = tail
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap();
        assert!(
            covered.contains(name) || disabled.contains(&name),
            "add a valid admission fixture for {name}, or explicitly document its exclusion"
        );
    }
}
