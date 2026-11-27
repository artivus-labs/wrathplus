//! Build 12340's StaticPopup host: Native UI's dialog answers ([`WrathPopupRequest`]) become
//! their sends, and the state the dialogs read goes out whole ([`WrathPopupState`]).
//!
//! Sent: the arena-team invite answers and disband (CMSG `0x351`/`0x352`/`0x355`), the
//! Wintergrasp entry and queue answers (`0x4DF`/`0x4E2`), `CancelSummon` (`0x51b110`: `0x2AC`
//! with the summoner and 0, dropping the latch), `DeclineInvite` (`0x410` with the channel), a refund purchase
//! (`0x4B4` for the slot's item), `RespondInstanceLock` (`0x13F`) and the LFG boot vote (`0x36C`).
//!
//! The instance lock, `00513b20` in wire order: `SMSG_INSTANCE_LOCK_WARNING_QUERY` (`0x147`)
//! with a timeout sets it (time left on the `GetTime` clock, the extend byte, the completed
//! mask) and fires `INSTANCE_LOCK_START`; one with none, or `SMSG_INSTANCE_SAVE_CREATED` flag 0,
//! clears it and fires `INSTANCE_LOCK_STOP`. The answer only sends (`0x613ed0`). The encounter
//! counts are `00553830`'s: the `DungeonEncounter.dbc` rows of the map and difficulty, and those
//! whose bit the mask sets. Labelled: its map and difficulty (`0xbd088c`, `0xbd0894`) are read as
//! the current map and the latest `SMSG_INSTANCE_DIFFICULTY`'s. The filled glyph sockets are the
//! active talent group's non-zero glyphs.
//!
//! Also sent: `GMResponseResolve` (`0x4F0`), glyph removal (`0x48A`), friend notes (`0x6B`, by row
//! or name) and chat complaints (`0x3C7`, the line by id or the newest from the named sender, held
//! by the native chat feed; the channel id reads 0).
//!
//! The refer-a-friend grant: `SMSG_PROPOSE_LEVEL_GRANT` latches the granter and fires
//! `LEVEL_GRANT_PROPOSED` with their name once known; `AcceptLevelGrant` answers it (`0x420`).
//! Labelled: `SMSG_REFER_A_FRIEND_FAILURE` stays in its capture (no error line yet).
//!
//! The guild bank: `GetGuildBankTabCost` (`0x5a6b20`) reads the next tab's price from the
//! six-row table `0xa15eec` (100 to 5000 gold) by the purchased count, nil past it.
//! `BuyGuildBankTab` (`0x5a5d30`), with the frame open on a banker, raises
//! `ERR_BANKSLOT_INSUFFICIENT_FUNDS` (`0x10f`) while the vault's money plus the player's falls
//! short of it, else sends CMSG `0x3EA` with the banker and that count (`0x5a5010`).
//! `DepositGuildBankMoney` (`0x5a5e20`, capped by Native UI) sends `0x3EB` with the banker.
//! Labelled: the funds check's two operands are read as the vault's and the player's money
//! (`0xc1dc20` and `0x51a2b0`, by their writers only).
//!
//! `PartyLFGStartBackfill` (`0x55a0c0`) is Native UI's: it reselects the group's dungeon and joins
//! through the LFG outbox.
//!
//! The state: `IsFalling` (our mover airborne, not rooted), `IsOutOfBounds` (`PLAYER_FLAGS`
//! `0x4000`), `PlayerCanTeleport` (no stun, confuse or flee flag) and `UnitIsControlling` (a charm
//! or summon) are published. `PlaceGlyphInSocket` forwards to the cast path's glyph wait
//! ([`crate::spell::targeting::glyph`]). `EndRefund` and `EndBoundTradeable` resume the item
//! enchant (`crate::spell::EnchantResume`) or the sockets (`SocketResume`), whose prompts those
//! seams raise. Not built: `ReplaceTradeEnchant` (`0x80c5f0`, the trade window's untraded slot as
//! an enchant's target, which no seam binds yet) and the "spellenchant" resume.
//! `CannotBeResurrected` reads an aura of type 314 among ours ([`cannot_be_resurrected`]).

use bevy::prelude::*;

use wrathplus_protocol::wrath_instance_control::Packet;
use wrathplus_ui::script::ScriptValue as V;
use wrathplus_ui::script::{UiScript, WrathPopupRequest, WrathPopupState};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::net::{ClientCommand, NetCommands, Objects};

/// The pending instance lock and its session clear. The answers drain inside
/// [`crate::ui_summon`]'s drain ([`PopupInputs`]), which already holds the VM, the summon latch
/// `CancelSummon` answers and the commands, so the host adds no schedule edge of its own.
pub struct UiStaticPopupPlugin;

impl Plugin for UiStaticPopupPlugin {
    fn build(&self, app: &mut App) {
        use crate::net::NetHandlerApp;
        app.init_resource::<InstanceLock>().net_handler(
            wrathplus_protocol::SessionEventKind::Disconnected,
            end_session,
        );
    }
}

/// The lock and the grant die with the session.
fn end_session(In(event): In<wrathplus_protocol::SessionEvent>, mut held: ResMut<InstanceLock>) {
    if matches!(event, wrathplus_protocol::SessionEvent::Disconnected { .. }) {
        *held = InstanceLock::default();
    }
}

/// The session's pending questions: the instance lock, and a refer-a-friend level grant's
/// granter (`SMSG_PROPOSE_LEVEL_GRANT`), with whether its `LEVEL_GRANT_PROPOSED` is still owed
/// (it fires once the granter's name is known).
#[derive(Resource, Default)]
pub(crate) struct InstanceLock {
    lock: Option<PendingLock>,
    granter: Option<u64>,
    grant_owed: bool,
    /// The latest `SMSG_INSTANCE_DIFFICULTY`'s difficulty, read by the LFG feed.
    pub(crate) difficulty: Option<u32>,
    /// Its second word (`0xbd1980`), `GetInstanceInfo`'s dynamic difficulty.
    pub(crate) dynamic_difficulty: u32,
}

#[path = "ui_static_popup_lock.rs"]
mod lock;
pub(crate) use lock::{lock_changes, PendingLock};

/// `IsOutOfBounds` (`0x6123c0`): `PLAYER_FLAGS_IS_OUT_OF_BOUNDS`.
const PLAYER_FLAGS_OUT_OF_BOUNDS: u32 = 0x4000;
/// `PlayerCanTeleport` (`0x60d8f0`): none of stunned, confused and fleeing in `UNIT_FIELD_FLAGS`.
const TELEPORT_BLOCKERS: u32 = 0x4_0000 | 0x40_0000 | 0x80_0000;

/// `UnitIsControlling` (`0x613c90`): the units whose `UNIT_FIELD_CHARM`, else `UNIT_FIELD_SUMMON`,
/// is set, in query order.
pub(crate) fn controlling<'a>(units: impl Iterator<Item = (u64, &'a ObjectStore)>) -> Vec<u64> {
    units
        .filter(|(_, s)| {
            s.0.unit_charm()
                .filter(|&g| g != 0)
                .or(s.0.unit_summon())
                .is_some_and(|g| g != 0)
        })
        .map(|(g, _)| g)
        .collect()
}

/// The 1-based filled glyph sockets of the active talent group.
pub(crate) fn filled_glyph_sockets(
    progress: Option<&crate::net::wrath_notices::WrathProgress>,
) -> Vec<u8> {
    let Some((_, active, groups)) = progress.and_then(|p| p.talents.get(&false)) else {
        return Vec::new();
    };
    groups
        .get(usize::from(*active))
        .map(|g| {
            g.glyphs
                .iter()
                .enumerate()
                .filter(|(_, &id)| id != 0)
                .map(|(i, _)| i as u8 + 1)
                .collect()
        })
        .unwrap_or_default()
}

/// `SPELL_AURA_PREVENT_RESURRECTION`.
const AURA_PREVENT_RESURRECTION: u32 = 314;

/// `CannotBeResurrected` (`0x802270`): the player's aura-type bitset (`+0xf20`, rebuilt by
/// `0x727e70` from each aura's effect mask and its spell's `EffectApplyAuraName`) holds 314 (the
/// tested `+0xf47 & 4`).
pub(crate) fn cannot_be_resurrected<'a>(
    auras: Option<impl Iterator<Item = &'a wrathplus_protocol::wrath_auras::NativeAura>>,
    aura_type: impl Fn(u32, usize) -> u32,
) -> bool {
    auras.is_some_and(|mut list| {
        list.any(|a| {
            (0..3).any(|k| {
                a.flags & (1 << k) != 0 && aura_type(a.spell_id, k) == AURA_PREVENT_RESURRECTION
            })
        })
    })
}

/// What an answer reads besides itself.
#[allow(clippy::type_complexity)] // Bevy system parameters / a closure table, kept inline
pub(crate) struct Lookups<'a> {
    /// A 1-based arena slot's team id, 0 for none.
    pub(crate) team_id: &'a dyn Fn(u8) -> u32,
    /// The latched summoner `CancelSummon` answers.
    pub(crate) summoner: Option<u64>,
    /// The latched level-grant granter `AcceptLevelGrant` answers.
    pub(crate) granter: Option<u64>,
    /// `ContainerRefundItemPurchase`'s item: bag, 1-based slot, equipped.
    pub(crate) refund_item: &'a dyn Fn(i64, u32, bool) -> Option<u64>,
    /// `SetFriendNotes`' friend: a row index or a name.
    pub(crate) friend: &'a dyn Fn(Option<u32>, Option<&str>) -> Option<u64>,
    /// `ComplainChat`'s line: by its id, else the newest from the named sender.
    pub(crate) chat_line: &'a dyn Fn(Option<i32>, Option<&str>) -> Option<ComplaintLine>,
    /// The guild vault in use (`GuildBankOpen`).
    pub(crate) banker: Option<u64>,
}

/// `0xa15eec`: the price of each guild-bank tab in purchase order, copper.
const TAB_COSTS: [u32; 6] = [
    1_000_000, 2_500_000, 5_000_000, 10_000_000, 25_000_000, 50_000_000,
];

/// `GetGuildBankTabCost`: the next tab's price, `None` once all six are bought.
pub(crate) fn tab_cost(purchased: usize) -> Option<u32> {
    TAB_COSTS.get(purchased).copied().filter(|&c| c != 0)
}

/// `BuyGuildBankTab`'s outcome: nothing without an open frame on a banker or past the last tab,
/// else the purchase or the refusal's key.
pub(crate) fn buy_tab(
    banker: Option<u64>,
    (open, purchased, vault): (bool, usize, u64),
    money: u64,
) -> Option<Result<ClientCommand, &'static str>> {
    let banker = banker.filter(|&b| b != 0 && open)?;
    let cost = TAB_COSTS.get(purchased).copied()?;
    if vault.saturating_add(money) < u64::from(cost) {
        return Some(Err("ERR_BANKSLOT_INSUFFICIENT_FUNDS"));
    }
    Some(Ok(ClientCommand::NativeGuildBank(
        wrathplus_protocol::wrath_guild_bank_commands::Command::BuyTab {
            banker,
            tab: purchased as u8,
        },
    )))
}

/// `CMSG_COMPLAIN`'s chat arm for one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ComplaintLine {
    pub(crate) sender: u64,
    pub(crate) language: u32,
    pub(crate) chat_type: u32,
    pub(crate) seconds_since: u32,
}

/// One answer's command, or `None` for one with no writer or nothing to answer.
fn answer(request: &WrathPopupRequest, look: &Lookups) -> Option<ClientCommand> {
    let team_id = look.team_id;
    let summoner = look.summoner;
    let refund_item = look.refund_item;
    use wrathplus_protocol::wrath_arena_team::Command as Arena;
    use wrathplus_protocol::wrath_battleground::Command as Bg;
    use WrathPopupRequest as R;
    Some(match request {
        R::AcceptArenaTeam => ClientCommand::ArenaTeamCommand(Arena::Accept),
        R::DeclineArenaTeam => ClientCommand::ArenaTeamCommand(Arena::Decline),
        R::ArenaTeamDisband { team } => ClientCommand::ArenaTeamCommand(Arena::Disband {
            team_id: team_id(*team),
        }),
        R::BattlefieldMgrEntryInvite { id, accept } => {
            ClientCommand::NativeBattleground(Bg::BfEntryInviteResponse {
                battle_id: *id as u32,
                accept: u8::from(*accept),
            })
        }
        R::BattlefieldMgrQueueInvite { id, accept } => {
            ClientCommand::NativeBattleground(Bg::BfQueueInviteResponse {
                battle_id: *id as u32,
                accept: u8::from(*accept),
            })
        }
        R::CancelSummon => {
            ClientCommand::NativeDeath(wrathplus_protocol::wrath_death::Command::SummonResponse {
                summoner: summoner?,
                agree: false,
            })
        }
        R::DeclineInvite { channel } => ClientCommand::DeclineChannelInvite {
            name: channel.clone(),
        },
        R::RefundItemPurchase {
            bag,
            slot,
            equipped,
        } => ClientCommand::NativeInventory(wrathplus_protocol::wrath_inventory::Command::Refund {
            item_guid: refund_item(*bag, *slot, *equipped)?,
        }),
        R::RespondInstanceLock { accept } => ClientCommand::NativeInstanceControl(
            wrathplus_protocol::wrath_instance_control::Command::LockResponse {
                raw_accept: u8::from(*accept),
            },
        ),
        R::SetLfgBootVote { yes } => ClientCommand::LfgCommand(
            wrathplus_protocol::wrath_lfg::commands::Command::SetBootVote { agree: *yes },
        ),
        // `AcceptLevelGrant()` (`0x523360` → `0x521b00`): CMSG `0x420` with the proposer.
        R::AcceptLevelGrant => ClientCommand::NativeReferFriend(
            wrathplus_protocol::wrath_refer_friend::Command::AcceptLevelGrant {
                granter: look.granter?,
            },
        ),
        // `GMResponseResolve()` (`0x5ad1b0`): CMSG `0x4F0`, empty.
        R::GmResponseResolve => {
            ClientCommand::NativeHelp(wrathplus_protocol::wrath_help::Command::ResolveResponse)
        }
        // `RemoveGlyphFromSocket(socket)` (`0x5b7660`): CMSG `0x48A` with the 0-based socket.
        R::RemoveGlyphFromSocket { socket } => {
            ClientCommand::NativeInspect(wrathplus_protocol::wrath_inspect::Command::RemoveGlyph {
                slot: u32::from(socket.checked_sub(1)?),
            })
        }
        // `SetFriendNotes(index | name, notes)` (`0x6b7c90`): CMSG `0x6B` for the friend.
        R::SetFriendNotes { index, name, note } => ClientCommand::SetContactNotes {
            guid: (look.friend)(*index, name.as_deref())?,
            note: note.clone(),
        },
        // `ComplainChat(lineID)` or `ComplainChat(name [, text])` (`0x50ed00`): the line's sender,
        // language, chat type and age. Labelled: the channel id reads 0.
        R::ComplainChat {
            line_id,
            name,
            text,
        } => {
            let line = (look.chat_line)(*line_id, name.as_deref())?;
            ClientCommand::NativeHelp(wrathplus_protocol::wrath_help::Command::ComplainChat {
                spammer: line.sender,
                language: line.language,
                message_type: line.chat_type,
                channel_id: 0,
                seconds_since: line.seconds_since,
                description: text.clone().unwrap_or_default(),
            })
        }
        R::DepositGuildBankMoney { copper } => ClientCommand::NativeGuildBank(
            wrathplus_protocol::wrath_guild_bank_commands::Command::DepositMoney {
                banker: look.banker.filter(|&b| b != 0)?,
                amount: *copper,
            },
        ),
        _ => return None,
    })
}

/// The per-VM published state.
#[derive(Default)]
pub(crate) struct Memo {
    pushed: Option<WrathPopupState>,
}

/// What the host reads beside the VM, the commands and the summon latch.
#[derive(bevy::ecs::system::SystemParam)]
#[allow(clippy::type_complexity)] // Bevy system parameters / a closure table, kept inline
pub(crate) struct PopupInputs<'w, 's> {
    packets: Option<
        ResMut<'w, crate::net::wrath_instance_control_capture::NativeInstanceControlPackets>,
    >,
    progress: Option<Res<'w, crate::net::wrath_notices::WrathProgress>>,
    social: Option<Res<'w, crate::app_core::interaction::SocialState>>,
    names: Option<Res<'w, crate::names::NameCache>>,
    chat: Option<Res<'w, crate::ui_chat::native_message::NativeChatFeed>>,
    self_q: Query<'w, 's, &'static ObjectStore, With<SelfPlayer>>,
    objects: Objects<'w, 's>,
    lock: Option<ResMut<'w, InstanceLock>>,
    /// `IsFalling`'s published bit; the units, for `UnitIsControlling`.
    falling: Option<Res<'w, crate::player::Falling>>,
    units: Query<'w, 's, (&'static crate::net::Guid, &'static ObjectStore)>,
    refer: Option<ResMut<'w, crate::net::wrath_refer_friend_capture::NativeReferFriendPackets>>,
    /// `PlaceGlyphInSocket`'s forward to the cast path, and `EndRefund` / `EndBoundTradeable`'s
    /// resumes: the item enchant and the sockets.
    forwards: (
        Option<ResMut<'w, crate::spell::targeting::glyph::GlyphPlacements>>,
        Option<ResMut<'w, crate::spell::EnchantResume>>,
        Option<ResMut<'w, crate::ui_native::wrath_sockets::SocketResume>>,
    ),
    /// `CannotBeResurrected`'s read: our auras and their spells' effects.
    auras: (
        Option<Res<'w, crate::net::wrath_auras::PacketAuras>>,
        Option<Res<'w, crate::app_core::state::Spells>>,
        Option<Res<'w, crate::app_core::session::SelfGuid>>,
    ),
    /// The vault and its open banker, for the tab purchase and deposit.
    bank: (
        Option<Res<'w, crate::ui_guild_bank::GuildBank>>,
        Option<Res<'w, crate::app_core::interaction::GuildBankOpen>>,
    ),
    /// A refusal's line: the message sink's halves.
    sink: (
        Option<ResMut<'w, crate::app_core::chat::ChatLog>>,
        Option<ResMut<'w, crate::sound::MessageSounds>>,
    ),
    /// The lock's encounter rows and the map it counts them for.
    instance: (
        Option<Res<'w, crate::ui_lfg::catalog::Encounters>>,
        Option<Res<'w, wrathplus_world::world_map::CurrentMap>>,
        Option<ResMut<'w, crate::ui_instance::InstanceState>>,
    ),
    memo: Local<'s, crate::app_core::vm_memo::VmMemo<Memo>>,
}

/// One frame of the host: take the lock warning, send the answers, publish the state.
pub(crate) fn drive(
    script: &mut UiScript,
    commands: &NetCommands,
    summon: &mut crate::ui_summon::SummonState,
    inputs: PopupInputs,
) {
    let PopupInputs {
        packets,
        refer,
        forwards: (mut placements, mut enchant_resume, mut socket_resume),
        auras: (auras, spells, self_guid),
        falling,
        units,
        progress,
        social,
        names,
        chat,
        self_q,
        objects,
        lock,
        bank: (vault, bank_open),
        sink: (log, sounds),
        instance: (encounters, current_map, instance_state),
        mut memo,
    } = inputs;
    let banker = bank_open.as_deref().and_then(|o| o.banker());
    let purchase = vault.as_deref().map_or((false, 0, 0), |v| v.tab_purchase());
    let mut sink = log
        .zip(sounds)
        .map(|(chat, sounds)| crate::ui_action::MessageSink { chat, sounds });
    let Some(mut lock) = lock else {
        return;
    };
    let mut packets = packets;
    // `SMSG_INSTANCE_DIFFICULTY`: the instance's difficulty, `IsInLFGDungeon`'s location half.
    if let Some(p) = packets.as_mut() {
        p.0.retain(|packet| match *packet {
            Packet::InstanceDifficulty {
                difficulty,
                dynamic_heroic,
            } => {
                lock.difficulty = Some(difficulty);
                lock.dynamic_difficulty = dynamic_heroic;
                false
            }
            _ => true,
        });
    }
    let raw = instance_state.map_or_else(Vec::new, |mut s| s.take_lock_changes());
    for change in lock_changes(raw, script.now()) {
        let event = if change.is_some() {
            "INSTANCE_LOCK_START"
        } else {
            "INSTANCE_LOCK_STOP"
        };
        lock.lock = change;
        script.fire_event(event, Vec::new());
    }

    // `SMSG_PROPOSE_LEVEL_GRANT`: the newest granter is latched; every other receipt stays.
    if let Some(mut refer) = refer {
        use wrathplus_protocol::wrath_refer_friend::Packet as Refer;
        refer.0.retain(|packet| match *packet {
            Refer::ProposeLevelGrant { granter } => {
                lock.granter = Some(granter);
                lock.grant_owed = true;
                false
            }
            _ => true,
        });
    }
    // `LEVEL_GRANT_PROPOSED(name)`, once the granter's name has landed.
    if lock.grant_owed {
        let name = lock
            .granter
            .and_then(|g| names.as_deref().and_then(|n| n.resolve(g, commands)))
            .map(str::to_string);
        if let Some(name) = name {
            lock.grant_owed = false;
            script.fire_event("LEVEL_GRANT_PROPOSED", vec![V::Str(name)]);
        }
    }

    let store = self_q.single().ok();
    let team_id = |team: u8| {
        team.checked_sub(1)
            .filter(|&slot| slot < 3)
            .and_then(|slot| store.and_then(|s| s.0.player_arena_team_id(slot)))
            .unwrap_or(0)
    };
    // `ContainerRefundItemPurchase(bag, slot [, isEquipped])`: a worn item by its 1-based
    // inventory slot, else the bag slot.
    let refund_item = |bag: i64, slot: u32, equipped: bool| {
        let store = store?;
        let slot0 = u8::try_from(slot.max(1) - 1).ok()?;
        if equipped {
            store.0.player_inv_slot(slot0).filter(|&g| g != 0)
        } else {
            crate::inventory::slot_guid(&store.0, bag, slot0, &objects)
        }
    };
    let friend = |index: Option<u32>, name: Option<&str>| {
        social
            .as_deref()
            .zip(names.as_deref())
            .and_then(|(social, names)| social.friend_guid(index, name, |g| names.peek(g)))
    };
    let chat_line = |line_id: Option<i32>, name: Option<&str>| {
        let chat = chat.as_deref()?;
        let line = match (line_id, name) {
            (Some(id), _) => chat.line(u32::try_from(id).ok()?),
            (None, Some(name)) => {
                let names = names.as_deref()?;
                chat.latest_where(|g| names.peek(g).is_some_and(|n| n.eq_ignore_ascii_case(name)))
            }
            (None, None) => None,
        }?;
        Some(ComplaintLine {
            sender: line.sender,
            language: line.language,
            chat_type: u32::from(line.chat_type),
            seconds_since: line.at.elapsed().as_secs() as u32,
        })
    };
    for request in script.take_wrath_popup_requests() {
        // `PlaceGlyphInSocket(socket)`: the cast path checks and commits the glyph wait.
        if let WrathPopupRequest::PlaceGlyphInSocket { socket } = request {
            if let Some(p) = placements.as_mut() {
                p.0.push(socket.saturating_sub(1));
            }
            continue;
        }
        // `EndRefund(type)` (`0x523370`) and `EndBoundTradeable("type")` (`0x5233d0`): the item
        // enchant (1, "itemenchant") or the sockets (2, "gem") resume confirmed. Labelled:
        // "spellenchant", a cast-time item target's prompt, is never raised here.
        let resume = match &request {
            WrathPopupRequest::EndRefund { kind: 1 } => Some(true),
            WrathPopupRequest::EndRefund { kind: 2 } => Some(false),
            WrathPopupRequest::EndBoundTradeable { kind } if kind == "itemenchant" => Some(true),
            WrathPopupRequest::EndBoundTradeable { kind } if kind == "gem" => Some(false),
            _ => None,
        };
        if let Some(enchant) = resume {
            if enchant {
                if let Some(r) = enchant_resume.as_mut() {
                    r.0 += 1;
                }
            } else if let Some(r) = socket_resume.as_mut() {
                r.0 += 1;
            }
            continue;
        }
        if request == WrathPopupRequest::BuyGuildBankTab {
            let money = store.and_then(|s| s.0.player_money()).unwrap_or(0);
            match buy_tab(banker, purchase, u64::from(money)) {
                Some(Ok(command)) => {
                    let _ = commands.0.send(command);
                }
                Some(Err(key)) => {
                    if let (Some(line), Some(sink)) =
                        (crate::ui_action::keyed_line(script, key), sink.as_mut())
                    {
                        crate::ui_action::show_messages(script, sink, "ui_static_popup", [line]);
                    }
                }
                None => debug!("BuyGuildBankTab: no open vault or no tab left"),
            }
            continue;
        }
        if request == WrathPopupRequest::DeclineLevelGrant {
            lock.granter = None;
            continue;
        }
        let look = Lookups {
            team_id: &team_id,
            summoner: summon.pending(),
            granter: lock.granter,
            refund_item: &refund_item,
            friend: &friend,
            chat_line: &chat_line,
            banker,
        };
        match answer(&request, &look) {
            Some(command) => {
                let _ = commands.0.send(command);
                match request {
                    WrathPopupRequest::CancelSummon => summon.cancel(),
                    WrathPopupRequest::AcceptLevelGrant => lock.granter = None,
                    _ => {}
                }
            }
            None => debug!("StaticPopup answer {request:?}: not sent (no writer or nothing held)"),
        }
    }

    let state = WrathPopupState {
        falling: falling.as_deref().is_some_and(|f| f.0),
        out_of_bounds: store.is_some_and(|s| s.0.player_flags() & PLAYER_FLAGS_OUT_OF_BOUNDS != 0),
        can_teleport: store.is_none_or(|s| s.0.unit_flags() & TELEPORT_BLOCKERS == 0),
        controlling: controlling(units.iter().map(|(g, s)| (g.0, s))),
        instance_lock: lock.lock.map(|held| {
            let rows = encounters.as_deref().map_or(&[][..], |e| e.0.as_slice());
            let here = current_map
                .as_deref()
                .map(|m| (m.0, lock.difficulty.unwrap_or(0)));
            held.view(rows, here)
        }),
        filled_glyph_sockets: filled_glyph_sockets(progress.as_deref()),
        cannot_be_resurrected: cannot_be_resurrected(
            self_guid
                .and_then(|g| g.0)
                .and_then(|g| auras.as_deref()?.by_guid.get(&g))
                .map(|list| list.iter().map(|s| &s.aura)),
            |spell, k| {
                spells
                    .as_deref()
                    .and_then(|s| s.catalog.get(spell))
                    .map_or(0, |d| d.effect_apply_aura[k])
            },
        ),
        guild_bank_tab_cost: tab_cost(purchase.1),
    };
    let memo = memo.get(script);
    if memo.pushed.as_ref() != Some(&state) {
        script.set_wrath_popup_state(state.clone());
        memo.pushed = Some(state);
    }
}

#[cfg(test)]
#[path = "ui_static_popup_tests.rs"]
mod tests;
