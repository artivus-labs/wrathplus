//! The charter VM feed and drain: the snapshot the charter windows read, their four events, the
//! composed lines, and the sends for the Lua [`PetitionRequest`]s.
//!
//! `PETITION_SHOW` waits for the record and every signer name (`0x4f419b`-`0x4f41ad`), then fires
//! once (`0x4f4320` fires as the pending count reaches zero); the owner's name is not counted
//! (`0x4f4186`/`0x4f42e8`). The getters answer at any time, nil where nothing has landed.

use bevy::prelude::*;
use wrathplus_ui::script::{
    PetitionRecordView, PetitionRequest, PetitionState as VmPetition, UiScript,
    PETITION_TYPE_CHARTER, PETITION_TYPE_PETITION,
};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::app_core::session::SelfPlayer;
use crate::inventory::find_item;
use crate::inventory::ItemSearch;
use crate::names::NameCache;
use crate::net::{ClientCommand, NetCommands, Objects};
use crate::ui_session::{npc_switched, NpcSession};

use super::{lines, GuildRegistrarState, PetitionState};

/// What the feed last announced, so the events fire on edges rather than every frame.
#[derive(Default)]
pub(super) struct FedPetition {
    registrar: Option<u64>,
    /// The charter shown last frame; `None` while one is still resolving, so the wait is an edge.
    shown_charter: Option<u64>,
    /// The last pushed view and names, so a change while shown re-fires `PETITION_SHOW`.
    shown: Option<PetitionRecordView>,
    shown_signers: Vec<Option<String>>,
}

/// Build the snapshot, push it, drain the queued lines, and fire the four events on their edges.
pub(super) fn feed_petition(
    script: Option<NonSendMut<UiScript>>,
    registrar: Res<GuildRegistrarState>,
    mut petition: ResMut<PetitionState>,
    names: Res<NameCache>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    self_guid: Res<SelfGuid>,
    commands: Res<NetCommands>,
    mut sink: crate::ui_action::MessageSink,
    mut fed: Local<crate::app_core::vm_memo::VmMemo<FedPetition>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let fed = fed.get(&script);

    // Shown before the snapshot, so a refusal lands the same frame as its state change; a key the
    // VM's `GlobalStrings.lua` lacks shows nothing, as in the reference.
    let composed = std::mem::take(&mut petition.lines);
    let resolved: Vec<crate::ui_action::Shown> = composed
        .iter()
        .filter_map(|e| {
            crate::ui_action::ui_error_text(e, &|key| {
                script.lua().globals().get::<String>(key).ok()
            })
            .map(|text| crate::ui_action::Shown::keyed(e.key, text))
        })
        .collect();
    crate::ui_action::show_messages(&mut script, &mut sink, "ui_petition", resolved);

    // Rebuilt every frame: reading the caches issues their queries, so a landing sets no flag and
    // only a rebuild sees it.
    let open = petition.open.clone();
    let me = self_guid.0.unwrap_or(0);

    let signers: Vec<Option<String>> = open
        .as_ref()
        .map(|o| {
            o.signers
                .iter()
                .map(|s| names.resolve(*s, &commands).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let record = open.as_ref().and_then(|o| {
        let r = petition.records.get(o.petition_id)?;
        Some(PetitionRecordView {
            petition_type: if r.is_charter {
                PETITION_TYPE_CHARTER
            } else if r.is_arena {
                wrathplus_ui::script::PETITION_TYPE_ARENA
            } else {
                PETITION_TYPE_PETITION
            }
            .to_string(),
            title: r.name.clone(),
            body_text: r.body_text.clone(),
            max_named: r.maximum,
            min_signatures: r.required,
            // Nil until the name lands, and not waited for.
            originator: names.resolve(o.owner, &commands).map(str::to_string),
            // The record's owner, which the binding compares (`0x4f447a`), not the packet's.
            is_originator: r.owner == me,
        })
    });

    let in_guild = self_q
        .iter()
        .next()
        .is_some_and(|s| s.0.player_guild_id() != 0);
    let signed = open.as_ref().map_or(&[][..], |o| o.signers.as_slice());
    let can_sign = can_sign(record.as_ref(), signed, me, in_guild);

    script.set_petition(VmPetition {
        charter_cost: registrar.cost(),
        signers: signers.clone(),
        record: record.clone(),
        can_sign,
    });

    // ── The registrar's edges. Switching registrars is a close then an open: `ShowUIPanel`
    //    returns early on a visible frame, so the open sound would not replay.
    let now_registrar = registrar.npc();
    if npc_switched(fed.registrar, now_registrar) {
        script.fire_event("GUILD_REGISTRAR_CLOSED", vec![]);
        script.fire_event("GUILD_REGISTRAR_SHOW", vec![]);
        // Drop the close intent `OnHide` just queued, or the drain would close the new registrar.
        let _ = script.drop_petition_close_intents();
    } else {
        match (fed.registrar, now_registrar) {
            (None, Some(_)) => script.fire_event("GUILD_REGISTRAR_SHOW", vec![]),
            (Some(_), None) => script.fire_event("GUILD_REGISTRAR_CLOSED", vec![]),
            _ => {}
        }
    }
    fed.registrar = now_registrar;

    // ── The petition window's edges, once the record and every signer name have landed.
    let ready = record.is_some() && signers.iter().all(Option::is_some);
    let now_charter = open.as_ref().map(|o| o.item).filter(|_| ready);
    match (fed.shown_charter, now_charter) {
        (Some(a), Some(b)) if a != b => {
            // Another charter while one is shown: drop the queued close intent, or the drain would
            // close the new charter and send a `MSG_PETITION_DECLINE` for it.
            script.fire_event("PETITION_CLOSED", vec![]);
            script.fire_event("PETITION_SHOW", vec![]);
            let _ = script.drop_petition_close_intents();
        }
        // The wait's end: the record landed or the last name resolved; fires once.
        (None, Some(_)) => script.fire_event("PETITION_SHOW", vec![]),
        (Some(_), None) => script.fire_event("PETITION_CLOSED", vec![]),
        // A rename echo, or a name resolving after the window was already up.
        (Some(_), Some(_)) if fed.shown != record || fed.shown_signers != signers => {
            script.fire_event("PETITION_SHOW", vec![])
        }
        _ => {}
    }
    fed.shown_charter = now_charter;
    fed.shown = record;
    fed.shown_signers = signers;
}

/// Turn the Lua charter intents into sends. `ClosePetition` can send `MSG_PETITION_DECLINE`
/// (`0x4f3f60`, `decline_on_close`); `CloseGuildRegistrar` sends nothing.
pub(super) fn drain_petition(
    script: Option<NonSendMut<UiScript>>,
    mut registrar: ResMut<GuildRegistrarState>,
    mut petition: ResMut<PetitionState>,
    names: Res<NameCache>,
    objects: Objects,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    self_guid: Res<SelfGuid>,
    selection: Res<crate::app_core::state::Selection>,
    commands: Res<NetCommands>,
    tabard: Res<crate::ui_tabard::TabardOpen>,
    mut errors: ResMut<crate::app_core::errors::UiErrorKeys>,
) {
    let Some(mut script) = script else {
        return;
    };
    let requests = script.take_petition_requests();
    if requests.is_empty() {
        return;
    }
    let me = self_guid.0.unwrap_or(0);
    for request in requests {
        match request {
            PetitionRequest::Buy(name) => {
                // The petition module's latch (`[0xbdceb0]`), not the target or `CGGameUI`'s NPC.
                let Some(npc) = registrar.npc() else {
                    debug!("ui_petition: BuyGuildCharter with no registrar open — dropped");
                    continue;
                };
                let _ = commands.0.send(ClientCommand::PetitionBuy {
                    npc,
                    name,
                    index: 1,
                });
            }
            PetitionRequest::TurnIn => {
                // A fresh bag scan per call (`0x5ef2b0`); the charter's `max_count` is 1.
                let charter = self_q.iter().next().and_then(|store| {
                    find_item(
                        &store.0,
                        &objects,
                        wrathplus_protocol::types::CHARTER_ITEM_ENTRY,
                        ItemSearch::default(),
                    )
                });
                match charter {
                    Some((_, _, item)) => {
                        let _ = commands
                            .0
                            .send(ClientCommand::TurnInPetition { item, banner: None });
                    }
                    // No charter, no packet: a red line (`0x5ef49a`).
                    None => petition.lines.push(lines::no_charter_line()),
                }
            }
            // Sends nothing.
            PetitionRequest::CloseRegistrar => registrar.close(),
            PetitionRequest::TabardInfo => {
                let npc = registrar.npc().unwrap_or(0);
                if let Some(store) = self_q.iter().next() {
                    tabard_vendor_activate(
                        npc,
                        tabard.npc().unwrap_or(0),
                        store,
                        &commands,
                        &mut errors,
                    );
                }
                registrar.close();
            }
            PetitionRequest::ClosePetition => {
                petition.decline_on_close(me, &commands);
                petition.close();
            }
            PetitionRequest::Sign(byte) => {
                let Some(item) = petition.open_item() else {
                    continue;
                };
                let _ = commands.0.send(ClientCommand::PetitionSign { item, byte });
                // The in-flight latch: a close while our signature is outstanding is no decline.
                petition.signing = true;
            }
            PetitionRequest::Offer => {
                let Some(item) = petition.open_item() else {
                    continue; // guard 1, silent
                };
                // `OfferPetition()` offers to the current target; with none it is silent (guard 2).
                let Some(player) = selection.guid else {
                    debug!("ui_petition: OfferPetition with no target — dropped");
                    continue;
                };
                if player == me {
                    // Guard 6; guards 3-5, 7 and 8 are not built, and the server refuses those.
                    petition.lines.push(lines::self_offer_line());
                    continue;
                }
                let _ = commands
                    .0
                    .send(ClientCommand::OfferPetition { item, player });
                // Printed on the send, unconfirmed: the server answers only the target.
                let name = names
                    .resolve(player, &commands)
                    .unwrap_or_default()
                    .to_string();
                petition.lines.push(lines::offered_line(&name));
            }
            PetitionRequest::Rename(name) => {
                let Some(item) = petition.open_item() else {
                    continue;
                };
                let _ = commands
                    .0
                    .send(ClientCommand::PetitionRename { item, name });
            }
            // The client's own name validator refused it; no packet is built.
            PetitionRequest::NameRefused(key) => {
                petition.lines.push(lines::name_refused_line(key));
            }
        }
    }
}

/// `0x5e00e0`, the designer's opener, for `GetTabardInfo` with the registrar's guid `npc`
/// (`0x4f5338`): nothing while the designer is open on that guid (`0x5e00e6`-`0x5e00fc`, so two
/// zero guids send nothing), the refusal while the player's `UNIT_FIELD_DISPLAYID` is not its
/// `UNIT_FIELD_NATIVEDISPLAYID` (`0x5e0102`-`0x5e0116`, error `0x130`), else the packet.
fn tabard_vendor_activate(
    npc: u64,
    designer: u64,
    player: &ObjectStore,
    commands: &NetCommands,
    errors: &mut crate::app_core::errors::UiErrorKeys,
) {
    if designer == npc {
        return;
    }
    if player.0.unit_displayid() != player.0.unit_native_displayid() {
        errors.0.push(crate::app_core::errors::UiError::key(
            "ERR_EMBLEMERROR_NOTABARDGEOSET",
        ));
        return;
    }
    let _ = commands.0.send(ClientCommand::TabardVendorActivate { npc });
}

/// `CanSignPetition` (`0x5cf040`): a guild or arena charter refuses once its signers reach the
/// record's cap (`[+0x1114]`, `0x5cf09d`) and a guild one a guilded player (`0x5cf08b`); the owner
/// (`0x5cf0af`) and a listed signer are refused. With no record only the list is scanned.
fn can_sign(record: Option<&PetitionRecordView>, signers: &[u64], me: u64, in_guild: bool) -> bool {
    let refused = record.is_some_and(|r| {
        let guild = r.petition_type == PETITION_TYPE_CHARTER;
        let capped = guild || r.petition_type == wrathplus_ui::script::PETITION_TYPE_ARENA;
        (guild && in_guild) || (capped && signers.len() as i32 >= r.max_named) || r.is_originator
    });
    !refused && !signers.contains(&me)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::errors::UiErrorKeys;
    use wrathplus_protocol::ObjectFields;

    /// The player's display ids, `UNIT_FIELD_DISPLAYID` (131) and `UNIT_FIELD_NATIVEDISPLAYID`.
    fn player(display: u32, native: u32) -> ObjectStore {
        ObjectStore(ObjectFields::from_fields(&[
            (wrathplus_protocol::types::Field::UnitDisplayId, display),
            (
                wrathplus_protocol::types::Field::UnitNativeDisplayId,
                native,
            ),
        ]))
    }

    fn run(npc: u64, designer: u64, me: &ObjectStore) -> (Vec<ClientCommand>, Vec<&'static str>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut errors = UiErrorKeys::default();
        tabard_vendor_activate(npc, designer, me, &NetCommands(tx), &mut errors);
        (
            rx.try_iter().collect(),
            errors.0.iter().map(|e| e.key).collect(),
        )
    }

    #[test]
    fn get_tabard_info_opens_the_designer_for_the_registrar() {
        let (sent, errors) = run(0x42, 0, &player(49, 49));
        assert!(
            matches!(
                sent.as_slice(),
                [ClientCommand::TabardVendorActivate { npc: 0x42 }]
            ),
            "{sent:?}"
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn get_tabard_info_sends_nothing_to_a_designer_already_open_on_that_npc() {
        let (sent, errors) = run(0x42, 0x42, &player(49, 49));
        assert!(sent.is_empty() && errors.is_empty());
        // With no registrar and no designer both guids are 0: equal, so nothing.
        let (sent, _) = run(0, 0, &player(49, 49));
        assert!(sent.is_empty());
    }

    #[test]
    fn can_sign_petition_caps_guild_and_arena_charters_and_refuses_guild_members_a_guild_one() {
        let arena = PetitionRecordView {
            petition_type: wrathplus_ui::script::PETITION_TYPE_ARENA.into(),
            max_named: 2,
            ..Default::default()
        };
        assert!(
            can_sign(Some(&arena), &[1], 9, true),
            "a guild is no bar to an arena team"
        );
        assert!(
            !can_sign(Some(&arena), &[1, 2], 9, false),
            "full at the cap"
        );
        let guild = PetitionRecordView {
            petition_type: PETITION_TYPE_CHARTER.into(),
            max_named: 9,
            min_signatures: 4,
            ..Default::default()
        };
        assert!(
            can_sign(Some(&guild), &[1, 2, 3, 4], 9, false),
            "past the minimum, under the cap"
        );
        assert!(!can_sign(Some(&guild), &[1], 9, true));
        assert!(!can_sign(None, &[9], 9, false) && can_sign(None, &[], 9, true));
    }

    #[test]
    fn get_tabard_info_refuses_a_shapeshifted_player() {
        let (sent, errors) = run(0x42, 0, &player(2281, 49));
        assert!(sent.is_empty());
        assert_eq!(errors, ["ERR_EMBLEMERROR_NOTABARDGEOSET"]);
    }
}
