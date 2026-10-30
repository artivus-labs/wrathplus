//! Wrath 3.3.5a app-session backend.
//!
//! This backend keeps the build-12340 cipher writer on one thread. Adapter replies and app
//! commands share that thread, so a time-sync response can never race a ping or lifecycle packet.

mod recorder;

mod mover_event;
pub(crate) use mover_event::mover_order_event;

#[path = "wrath/transfer_authority.rs"]
mod transfer_authority;

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{Receiver, Sender};
use wrathplus_protocol::wrath_authority::{
    self, ForceRootOrder, GroundPose, MoverOrder, OnFootPose, UnsetCanFlyOrder,
};
use wrathplus_protocol::wrath_combat;
use wrathplus_protocol::wrath_events::{
    WrathDispatch, WrathEvent, WrathEventAdapter, WrathEventError, WrathReply,
};
use wrathplus_protocol::wrath_gameplay::{MovementPosition, ObjectUpdate, UPDATE_FLAG_SELF};
use wrathplus_protocol::wrath_ground::{self, SessionClock};
use wrathplus_protocol::wrath_loot;
use wrathplus_protocol::wrath_world::{
    WrathCharacter, WrathPartialFrameRead, WrathWardenRequired, WrathWorldAuthReject,
    WrathWorldSession,
};
use wrathplus_protocol::{CharEnumItem, Character, LoginStage, SessionEnd, SessionEvent};

use super::wrath_send_log::opcode_label;
use super::{note_inbound, Parks, PingClock, PING_INTERVAL, SEND_WARN_CAP};
use crate::net::{CharRequest, ClientCommand, RealmRequest, SessionCommand, SessionCommandSender};

// AzerothCore 3.3.5a Opcodes.h, also used by the validated `wrath_objects` probe.
const CMSG_ITEM_QUERY_SINGLE: u32 = 0x056;
const SMSG_CHARACTER_LOGIN_FAILED: u16 = 0x041;
const CMSG_PING: u32 = 0x01dc;
const SMSG_PONG: u16 = 0x01dd;
const CMSG_SET_ACTIVE_MOVER: u32 = 0x026a;
const SMSG_CLIENT_CONTROL_UPDATE: u16 = 0x0159;
const MAX_PENDING_PRE_SELF_ORDERS: usize = 8;
const PRE_SELF_ORDER_TIMEOUT: Duration = Duration::from_secs(2);
const READ_SLICE: Duration = Duration::from_millis(250);
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const REALM_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn spawn_threads(
    events_tx: Sender<SessionEvent>,
    cmd_rx: Receiver<SessionCommand>,
    parks: Parks,
    abandon: Arc<AtomicU64>,
    ping_clock: Arc<Mutex<PingClock>>,
    command_tx: Sender<SessionCommand>,
    command_endpoints: Sender<SessionCommandSender>,
) {
    let (control_tx, control_rx) = crossbeam_channel::unbounded::<WrathWriterControl>();
    let (fatal_tx, fatal_rx) = crossbeam_channel::unbounded::<EpochFatal>();
    let authority = Arc::new(MovementAuthority::default());
    let writer_clock = Arc::clone(&ping_clock);
    let writer_control = control_tx.clone();
    thread::Builder::new()
        .name("wow-net-write-wrath".into())
        .spawn(move || {
            wrathplus_world::thread_qos::promote_current_thread(
                wrathplus_world::thread_qos::QosClass::UserInitiated,
            );
            writer_loop(
                &cmd_rx,
                control_rx,
                &writer_control,
                &writer_clock,
                &fatal_tx,
            );
        })
        .expect("spawn Wrath network writer");

    thread::Builder::new()
        .name("wow-net-wrath".into())
        .spawn(move || {
            wrathplus_world::thread_qos::promote_current_thread(
                wrathplus_world::thread_qos::QosClass::UserInitiated,
            );
            let mut next_epoch = 0;
            loop {
                ping_clock.lock().unwrap_or_else(|p| p.into_inner()).clear();
                match run(
                    &events_tx,
                    &control_tx,
                    &fatal_rx,
                    &parks,
                    &abandon,
                    &ping_clock,
                    &command_tx,
                    &command_endpoints,
                    &mut next_epoch,
                    &authority,
                ) {
                    Ok(WrathCycle::Exit) => return,
                    Ok(WrathCycle::Repark) => {}
                    Ok(WrathCycle::LoggedOut) => {
                        if events_tx
                            .send(SessionEvent::Disconnected {
                                reason: "logged out".into(),
                                end: SessionEnd::LoggedOut,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        bevy::log::error!("net(wrath): {error:#}");
                        if events_tx
                            .send(SessionEvent::Disconnected {
                                reason: format!("Wrath session disconnected: {error:#}"),
                                end: SessionEnd::Lost,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        })
        .expect("spawn Wrath network reader");
}

#[derive(Clone, Copy)]
enum WrathCycle {
    Exit,
    Repark,
    LoggedOut,
}

fn run(
    events: &Sender<SessionEvent>,
    control: &Sender<WrathWriterControl>,
    fatal: &Receiver<EpochFatal>,
    parks: &Parks,
    abandon: &AtomicU64,
    ping_clock: &Mutex<PingClock>,
    command_tx: &Sender<SessionCommand>,
    command_endpoints: &Sender<SessionCommandSender>,
    next_epoch: &mut u64,
    authority: &Arc<MovementAuthority>,
) -> Result<WrathCycle> {
    let request = match parks.login_rx.recv() {
        Ok(request) => request,
        Err(_) => return Ok(WrathCycle::Exit),
    };
    let generation = request.generation;
    let canceled = || abandon.load(Ordering::SeqCst) != generation;
    send(
        events,
        SessionEvent::LoginStage {
            stage: LoginStage::Connecting,
        },
    )?;
    send(
        events,
        SessionEvent::LoginStage {
            stage: LoginStage::Authenticating,
        },
    )?;
    let mut logon =
        match wrathplus_protocol::wrath_auth::logon(&request.host, &request.user, &request.pass) {
            Ok(logon) => logon,
            Err(error) => {
                if canceled() {
                    return Ok(WrathCycle::Repark);
                }
                let refusal = error
                    .downcast_ref::<wrathplus_protocol::AuthReject>()
                    .map(|reject| wrathplus_protocol::LoginRefusal::Logon(reject.code));
                send(
                    events,
                    SessionEvent::LoginFailed {
                        refusal,
                        reason: format!("Wrath authentication: {error:#}"),
                        terminal: false,
                        dial: None,
                    },
                )?;
                return Ok(WrathCycle::Repark);
            }
        };
    if canceled() {
        return Ok(WrathCycle::Repark);
    }

    let mut realms = normalize_realms(&logon.realms);
    let mut selected = if realms.is_empty() {
        None
    } else {
        drain(&parks.realm_rx);
        loop {
            send(
                events,
                SessionEvent::RealmList {
                    realms: realms.clone(),
                },
            )?;
            match parks.realm_rx.recv() {
                Err(_) => return Ok(WrathCycle::Exit),
                Ok(_) if canceled() => return Ok(WrathCycle::Repark),
                Ok(RealmRequest::Abandon) => return Ok(WrathCycle::Repark),
                Ok(RealmRequest::Refresh) => {
                    logon.refresh_realms(REALM_REFRESH_TIMEOUT);
                    realms = normalize_realms(&logon.realms);
                }
                Ok(RealmRequest::Enter(name)) => {
                    if let Some(index) = realms.iter().position(|realm| realm.name == name) {
                        break Some(index);
                    }
                }
            }
        }
    };

    'realm: loop {
        let (world_addr, realm_id, realm_record) = match selected {
            Some(index) => {
                let raw = &logon.realms[index];
                (
                    raw.address.clone(),
                    u32::from(raw.id),
                    Some(realms[index].clone()),
                )
            }
            None => {
                let (host, _) = wrathplus_protocol::host_port(
                    &request.host,
                    wrathplus_protocol::wrath_world::WRATH_WORLD_PORT,
                );
                (
                    format!(
                        "{host}:{}",
                        wrathplus_protocol::wrath_world::WRATH_WORLD_PORT
                    ),
                    0,
                    None,
                )
            }
        };
        send(
            events,
            SessionEvent::LoginStage {
                stage: LoginStage::Handshaking,
            },
        )?;
        let mut session = match WrathWorldSession::connect(
            &world_addr,
            &request.user,
            logon.session_key,
            realm_id,
        ) {
            Ok(session) => session,
            Err(error) => {
                if canceled() {
                    return Ok(WrathCycle::Repark);
                }
                if error.downcast_ref::<WrathWardenRequired>().is_some() {
                    send_login_failure(events, "Wrath server requires unsupported Warden", true);
                } else {
                    let refusal = error
                        .downcast_ref::<WrathWorldAuthReject>()
                        .map(|reject| wrathplus_protocol::LoginRefusal::World(reject.code));
                    send(
                        events,
                        SessionEvent::LoginFailed {
                            refusal,
                            reason: format!("Wrath world handshake with {world_addr}: {error:#}"),
                            terminal: false,
                            dial: None,
                        },
                    )?;
                }
                return Ok(WrathCycle::Repark);
            }
        };
        if canceled() {
            return Ok(WrathCycle::Repark);
        }
        send(
            events,
            SessionEvent::WrathAccountExpansion {
                expansion: session.account_expansion(),
            },
        )?;
        let mut characters = normalize_characters(session.char_enum().context("Wrath roster")?);
        if canceled() {
            return Ok(WrathCycle::Repark);
        }
        drain(&parks.pick_rx);
        drain(&parks.realm_rx);
        send(
            events,
            SessionEvent::CharacterList {
                characters: characters.clone(),
                realm: realm_record.clone(),
            },
        )?;

        let guid = loop {
            crossbeam_channel::select! {
                recv(parks.realm_rx) -> choice => match choice {
                    Err(_) => return Ok(WrathCycle::Exit),
                    Ok(RealmRequest::Abandon) => {}
                    Ok(RealmRequest::Refresh) => {
                        logon.refresh_realms(REALM_REFRESH_TIMEOUT);
                        realms = normalize_realms(&logon.realms);
                        send(events, SessionEvent::RealmList { realms: realms.clone() })?;
                    }
                    Ok(RealmRequest::Enter(name)) => {
                        if let Some(index) = realms.iter().position(|realm| realm.name == name) {
                            selected = Some(index);
                            continue 'realm;
                        }
                    }
                },
                recv(parks.pick_rx) -> pick => match pick {
                    Err(_) => return Ok(WrathCycle::Exit),
                    Ok(CharRequest::Enter(guid)) => break guid,
                    Ok(CharRequest::Abandon) => return Ok(WrathCycle::Repark),
                    Ok(CharRequest::Create(request)) => {
                        let native = wrathplus_protocol::wrath_world::WrathCreateCharacter {
                            name: request.name, race: request.race, class: request.class,
                            gender: request.gender, skin: request.skin, face: request.face,
                            hair_style: request.hair_style, hair_color: request.hair_color,
                            facial_hair: request.facial_hair, outfit_id: 0,
                        };
                        let code = session.create_character_result(&native)?;
                        finish_character_action(&mut session, &mut characters, realm_record.clone(),
                            wrathplus_protocol::CharAction::Create, code, |event| send(events, event))?;
                    }
                    Ok(CharRequest::Service(command)) => {
                        let reply = session.char_service_result(&command)?;
                        char_service::publish_char_service(reply, || {
                            characters = normalize_characters(session.char_enum().context("Wrath refreshed roster")?);
                            Ok(SessionEvent::CharacterList { characters: characters.clone(), realm: realm_record.clone() })
                        }, |event| send(events, event))?;
                    }
                    Ok(CharRequest::Delete(guid)) => {
                        let code = session.delete_character_result(guid)?;
                        finish_character_action(&mut session, &mut characters, realm_record.clone(),
                            wrathplus_protocol::CharAction::Delete, code, |event| send(events, event))?;
                    }
                }
            }
        };
        let name = characters
            .iter()
            .find(|character| character.guid == guid)
            .map_or_else(String::new, |character| character.name.clone());
        let chat_language = characters
            .iter()
            .find(|character| character.guid == guid)
            .map_or(wrathplus_protocol::types::LANGUAGE_COMMON, |character| {
                wrathplus_protocol::types::faction_language(character.race)
            });

        // Lossless handoff: send login, split immediately, announce the session, then consume every
        // queued packet through the Wrath adapter.
        session.begin_player_login(guid)?;
        // The auth bootstrap the select screen kept (`SMSG_ADDON_INFO`, `SMSG_CLIENTCACHE_VERSION`).
        let addon_info = session.addon_info();
        let bootstrap = session.take_bootstrap_packets();
        let (mut reader, writer) = session.into_split()?;
        let session_clock = Arc::new(SessionClock::new());
        *next_epoch = next_epoch
            .checked_add(1)
            .expect("Wrath command epoch overflow");
        let epoch = *next_epoch;
        let authority = authority.install(epoch, guid);
        let (installed_tx, installed_rx) = crossbeam_channel::bounded(0);
        control
            .send(WrathWriterControl::Install {
                epoch,
                session: Box::new(WrathWriterSwap {
                    writer: super::wrath_send_log::LoggedWriter::new(writer),
                    chat_language,
                    clock: Arc::clone(&session_clock),
                    active_mover: guid,
                    authority: authority.clone(),
                }),
                installed: installed_tx,
            })
            .map_err(|_| anyhow!("Wrath writer stopped"))?;
        installed_rx
            .recv()
            .map_err(|_| anyhow!("Wrath writer stopped before install acknowledgement"))?
            .map_err(|reason| anyhow!(reason))?;
        let _retire = WriterRetireGuard {
            epoch,
            control: control.clone(),
            authority: authority.clone(),
        };
        command_endpoints
            .send(SessionCommandSender::new(command_tx.clone(), epoch))
            .map_err(|_| anyhow!("app exited before Wrath command endpoint publication"))?;
        send(
            events,
            SessionEvent::Connected {
                self_guid: guid,
                name,
                billing_time_rested: 0,
                tutorial_flags: None,
                // The stock addons `SMSG_ADDON_INFO` marked secure; `None` (nothing loads) if silent.
                addon_info,
            },
        )?;
        for packet in bootstrap {
            match wrathplus_protocol::wrath_bootstrap::decode(packet.opcode, &packet.body) {
                Ok(decoded) => {
                    for event in decoded {
                        send(events, event)?;
                    }
                }
                Err(error) => {
                    bevy::log::warn!(
                        "net(wrath): bootstrap {:#06x} dropped: {error}",
                        packet.opcode
                    );
                    send(
                        events,
                        SessionEvent::PacketDropped {
                            opcode: packet.opcode,
                            unparseable: true,
                        },
                    )?;
                }
            }
        }
        reader.set_read_timeout(Some(READ_SLICE))?;
        let mut adapter = WrathEventAdapter::new();
        let mut packet_recorder = recorder::Recorder::from_env();
        // `SMSG_MULTIPLE_MOVES` records, handled in order as the packets they name before the next
        // socket read, so a bundled self order raises its authority barrier like a lone one.
        let mut bundled = std::collections::VecDeque::new();
        loop {
            if let Ok(failure) = fatal.try_recv() {
                if failure.epoch == epoch {
                    bail!("{}", failure.reason);
                }
            }
            let packet = match bundled.pop_front() {
                Some(packet) => packet,
                None => {
                    let packet = match reader.recv_raw_with_frame_timeout(FRAME_TIMEOUT) {
                        Ok(packet) => packet,
                        Err(error) if retryable_timeout(&error) => continue,
                        Err(error) => return Err(error),
                    };
                    recorder::record(&mut packet_recorder, &packet);
                    note_inbound();
                    packet
                }
            };
            match packet.opcode {
                wrathplus_protocol::wrath_movement_misc::SMSG_MULTIPLE_MOVES => {
                    let records =
                        wrathplus_protocol::wrath_movement_misc::split_multiple_moves(&packet.body)
                            .context("Wrath SMSG_MULTIPLE_MOVES")?;
                    // A bundle inside a bundle is not something AzerothCore writes.
                    if records.iter().any(|(opcode, _)| *opcode == packet.opcode) {
                        bail!("nested Wrath SMSG_MULTIPLE_MOVES");
                    }
                    bundled.extend(records.into_iter().map(|(opcode, body)| {
                        wrathplus_protocol::wrath_world::WrathServerPacket { opcode, body }
                    }));
                }
                SMSG_CHARACTER_LOGIN_FAILED => {
                    let result = one_byte(&packet.body, "SMSG_CHARACTER_LOGIN_FAILED")?;
                    send(events, SessionEvent::CharacterLoginFailed { result })?;
                    return Ok(WrathCycle::LoggedOut);
                }
                opcode if wrathplus_protocol::wrath_logout::handles(opcode) => {
                    let Some(event) = authority.logout_event(opcode, &packet.body)? else {
                        continue;
                    };
                    let complete = matches!(event, SessionEvent::LoggedOut);
                    send(events, event)?;
                    if complete {
                        return Ok(WrathCycle::LoggedOut);
                    }
                }
                SMSG_PONG => {
                    let sequence = le_u32_exact(&packet.body, "SMSG_PONG")?;
                    let _ = ping_clock
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .record_pong(sequence);
                }
                0x003e..=0x0040 => {
                    let packet = match wrathplus_protocol::wrath_transfer::decode(
                        packet.opcode,
                        &packet.body,
                    ) {
                        Ok(packet) => packet,
                        Err(error) => {
                            authority.revoke();
                            return Err(error.into());
                        }
                    };
                    authority.publish_transfer(packet, |event| send(events, event))?;
                }
                SMSG_CLIENT_CONTROL_UPDATE => {
                    let (mover, allow_move) = client_control_update(&packet.body)?;
                    authority.observe_control(mover, allow_move)?;
                    send(events, SessionEvent::ClientControl { mover, allow_move })?;
                }
                wrath_authority::SMSG_MOVE_UNSET_CAN_FLY => {
                    let order = match wrath_authority::decode_unset_can_fly(&packet.body) {
                        Ok(order) => order,
                        Err(error) => {
                            authority.revoke();
                            return Err(error).context("Wrath SMSG_MOVE_UNSET_CAN_FLY");
                        }
                    };
                    if !authority.raise_unset_barrier(order)? {
                        continue;
                    }
                    // The controller lands a flier and drops the grant (`0x6ef860` event 0x30).
                    send(
                        events,
                        SessionEvent::MoveMode {
                            guid: order.mover_guid,
                            counter: order.counter,
                            mode: wrathplus_protocol::types::MoveMode::CanFly,
                            apply: false,
                        },
                    )?;
                    control
                        .send(WrathWriterControl::Internal {
                            epoch,
                            command: WrathWriterCommand::UnsetCanFly(order),
                        })
                        .map_err(|_| anyhow!("Wrath writer stopped"))?;
                }
                wrath_authority::SMSG_FORCE_MOVE_ROOT | wrath_authority::SMSG_FORCE_MOVE_UNROOT => {
                    let rooted = packet.opcode == wrath_authority::SMSG_FORCE_MOVE_ROOT;
                    let order = match wrath_authority::decode_force_root(&packet.body) {
                        Ok(order) => order,
                        Err(error) => {
                            authority.revoke();
                            return Err(error).context("Wrath force root order");
                        }
                    };
                    // The reader raises the barrier before enqueue, so no later app movement can
                    // overtake receipt of this server authority order.
                    if !authority.raise_root_barrier(order, rooted)? {
                        continue;
                    }
                    // The controller stops translating (it may still turn), as a 1.12 root does.
                    send(
                        events,
                        SessionEvent::MoveMode {
                            guid: order.mover_guid,
                            counter: order.counter,
                            mode: wrathplus_protocol::types::MoveMode::Root,
                            apply: rooted,
                        },
                    )?;
                    control
                        .send(WrathWriterControl::Internal {
                            epoch,
                            command: WrathWriterCommand::ForceRoot { order, rooted },
                        })
                        .map_err(|_| anyhow!("Wrath writer stopped"))?;
                }
                opcode if wrath_authority::is_mover_order(opcode) => {
                    let order = match wrath_authority::decode_mover_order(opcode, &packet.body) {
                        Ok(order) => order,
                        Err(error) => {
                            authority.revoke();
                            return Err(error).context("Wrath mover order");
                        }
                    };
                    // Barrier first, as for root: no later app movement may overtake the order.
                    if !authority.raise_mover_barrier(order)? {
                        continue;
                    }
                    if let Some(event) = mover_order_event(&order) {
                        send(events, event)?;
                    }
                    // A near teleport is acknowledged by the controller once it has snapped.
                    if !matches!(order, MoverOrder::TeleportNear { .. }) {
                        control
                            .send(WrathWriterControl::Internal {
                                epoch,
                                command: WrathWriterCommand::Mover(order),
                            })
                            .map_err(|_| anyhow!("Wrath writer stopped"))?;
                    }
                }
                _ => {
                    reject_unsupported_movement(&authority, packet.opcode)?;
                    // A spline onto or off a vehicle seat (`MoveSplineInit::Launch`).
                    let seated = match packet.opcode {
                        0x00dd | 0x02ae => wrathplus_protocol::wrath_vehicle::monster_move_seat(
                            &packet.body,
                            packet.opcode == 0x02ae,
                        )
                        .and_then(|(mover, seat)| authority.observe_seat(mover, seat)),
                        _ => None,
                    };
                    if let Some(change) = seated {
                        send(events, SessionEvent::WrathVehicle(change))?;
                    }
                    dispatch_packet(
                        events,
                        control,
                        epoch,
                        &mut adapter,
                        &packet,
                        &session_clock,
                        &authority,
                    )?;
                }
            }
        }
    }
}

fn dispatch_packet(
    events: &Sender<SessionEvent>,
    control: &Sender<WrathWriterControl>,
    epoch: u64,
    adapter: &mut WrathEventAdapter,
    packet: &wrathplus_protocol::wrath_world::WrathServerPacket,
    clock: &SessionClock,
    authority: &MovementAuthorityHandle,
) -> Result<()> {
    let now = clock.now_ms();
    if wrathplus_protocol::wrath_relay::handles(packet.opcode) {
        let event = match wrathplus_protocol::wrath_relay::decode(packet.opcode, &packet.body) {
            Ok(event) => event,
            // Another unit swimming, flying or riding is outside the on-foot relay subset: drop
            // that one packet. Only our own mover's server-authored pose is authority.
            Err(error)
                if wrathplus_protocol::wire::read_packed_guid(&mut packet.body.as_slice())
                    .is_ok_and(|guid| guid != authority.self_guid) =>
            {
                bevy::log::debug!(
                    "net(wrath): foreign movement relay {:#05x} outside the on-foot subset dropped: {error}",
                    packet.opcode
                );
                return send(
                    events,
                    SessionEvent::PacketDropped {
                        opcode: packet.opcode,
                        unparseable: true,
                    },
                );
            }
            Err(error) => return Err(error).context("Wrath on-foot movement relay"),
        };
        if let SessionEvent::UnitMove { guid, .. } = &event {
            let raw =
                wrathplus_protocol::wrath_gameplay::OutboundMovement::from_bytes(&packet.body)?;
            authority.observe_server_pose(*guid, &raw.movement)?;
            let seat = movement_seat(&raw.movement);
            if let Some(change) = authority.observe_seat(*guid, seat) {
                send(events, event)?;
                return send(events, SessionEvent::WrathVehicle(change));
            }
        }
        return send(events, event);
    }
    match adapter.dispatch_packet(packet, now) {
        Ok(WrathDispatch::Handled {
            session_events,
            wrath_events,
            replies,
        }) => {
            let mut vehicle_events = Vec::new();
            for event in &wrath_events {
                authority.observe_vehicles(event, &mut vehicle_events);
                if let WrathEvent::Object(object) = event {
                    if let ObjectUpdate::Create { guid, movement, .. } = object.as_ref() {
                        if movement.update_flags & UPDATE_FLAG_SELF == 0 {
                            authority.observe_unit_create(*guid, &movement.position);
                        } else {
                            authority.observe_self_create(*guid, &movement.position)?;
                            control
                                .send(WrathWriterControl::Internal {
                                    epoch,
                                    command: WrathWriterCommand::PoseReady,
                                })
                                .map_err(|_| anyhow!("Wrath writer stopped"))?;
                        }
                    }
                }
            }
            // The adapter's vectors preserve packet/block order. Replies use the sole cipher writer.
            for event in session_events {
                authority.observe_taxi_spline(&event);
                if *crate::net::CAST_TRACE
                    && !matches!(
                        &event,
                        SessionEvent::WrathWho {
                            packet: wrathplus_protocol::wrath_who::Packet::Whois { .. }
                        }
                    )
                {
                    bevy::log::info!("net(wrath) RECV {event:?}");
                }
                send(events, event)?;
            }
            for event in vehicle_events {
                send(events, SessionEvent::WrathVehicle(event))?;
            }
            for reply in replies {
                control
                    .send(WrathWriterControl::Internal {
                        epoch,
                        command: WrathWriterCommand::Reply(reply),
                    })
                    .map_err(|_| anyhow!("Wrath writer stopped"))?;
            }
        }
        Ok(WrathDispatch::Unhandled { opcode }) => {
            if *crate::net::CAST_TRACE {
                bevy::log::info!("net(wrath) RECV unhandled opcode 0x{opcode:04x}");
            }
            send(
                events,
                SessionEvent::PacketDropped {
                    opcode,
                    unparseable: false,
                },
            )?;
        }
        Err(WrathEventError::WardenRequired) => bail!("Wrath server requires unsupported Warden"),
        Err(error) => bail!("{error}"),
    }
    Ok(())
}

#[derive(Debug)]
enum WrathWriterCommand {
    Reply(WrathReply),
    UnsetCanFly(UnsetCanFlyOrder),
    ForceRoot {
        order: ForceRootOrder,
        rooted: bool,
    },
    /// A speed, mode, knockback or collision-height order the writer acknowledges itself.
    Mover(MoverOrder),
    PoseReady,
}

enum WrathWriterControl {
    Install {
        epoch: u64,
        session: Box<WrathWriterSwap>,
        installed: Sender<Result<(), String>>,
    },
    Internal {
        epoch: u64,
        command: WrathWriterCommand,
    },
    Retire {
        epoch: u64,
        retired: Sender<()>,
    },
    Fatal {
        epoch: u64,
        reason: String,
    },
}

struct WriterRetireGuard {
    epoch: u64,
    control: Sender<WrathWriterControl>,
    authority: MovementAuthorityHandle,
}

impl Drop for WriterRetireGuard {
    fn drop(&mut self) {
        // Reader exit is the authority boundary. Revoke under the movement-send mutex before
        // retirement is queued, so a delayed writer cannot send after the session has ended.
        self.authority.revoke();
        let (retired_tx, retired_rx) = crossbeam_channel::bounded(0);
        if self
            .control
            .send(WrathWriterControl::Retire {
                epoch: self.epoch,
                retired: retired_tx,
            })
            .is_ok()
        {
            // No next install can be issued until the sole writer has retired this epoch.
            let _ = retired_rx.recv();
        }
    }
}

struct EpochFatal {
    epoch: u64,
    reason: String,
}

struct WrathWriterSwap {
    writer: super::wrath_send_log::LoggedWriter,
    /// The character's own tongue for chat that names none (`faction_language` of its race).
    chat_language: u32,
    clock: Arc<SessionClock>,
    active_mover: u64,
    authority: MovementAuthorityHandle,
}

#[derive(Default)]
struct MovementAuthority {
    state: Mutex<MovementAuthorityState>,
}

#[derive(Default)]
struct MovementAuthorityState {
    epoch: u64,
    self_guid: u64,
    self_created: bool,
    self_control: bool,
    control_decided: bool,
    ground_pose: Option<OnFootPose>,
    pending_orders: std::collections::VecDeque<PendingAuthorityOrder>,
    last_order_counter: Option<u32>,
    rooted: bool,
    taxi_spline: Option<(u32, bool)>,
    logout_pending: bool,
    transfer_sequence: u64,
    transfer_ack: Option<wrathplus_protocol::wrath_transfer::Token>,
    transfer_waiting: bool,
    transfer_transport: bool,
    /// The unit the server lets us drive (AzerothCore `m_mover`): our own guid until
    /// `SMSG_CLIENT_CONTROL_UPDATE` hands us a vehicle or a possessed unit. `ground_pose` and
    /// `rooted` are this mover's.
    mover: u64,
    /// The last known pose and root of every other unit we may come to drive or have driven: our
    /// own body while we drive another, and each unit created with a living movement block.
    parked: std::collections::HashMap<u64, ParkedMover>,
    /// Units seen as vehicles and every passenger's seat, for the vehicle feed's change events.
    vehicles: std::collections::HashSet<u64>,
    riders: std::collections::HashMap<u64, (u64, i8)>,
}

/// A unit's pose and root while it is not the active mover.
#[derive(Debug, Clone, Copy, Default)]
struct ParkedMover {
    pose: Option<OnFootPose>,
    rooted: bool,
}

impl MovementAuthorityState {
    /// Whether an order for `guid` has a pose to acknowledge from: the active mover, or a unit we
    /// parked (our own body while we drive another).
    fn drives_or_parked(&self, guid: u64) -> bool {
        guid == self.mover || self.parked.contains_key(&guid)
    }

    fn pose_of(&self, guid: u64) -> Option<OnFootPose> {
        if guid == self.mover {
            self.ground_pose
        } else {
            self.parked.get(&guid).and_then(|p| p.pose)
        }
    }

    fn set_pose_of(&mut self, guid: u64, pose: OnFootPose) {
        if guid == self.mover {
            self.ground_pose = Some(pose);
        } else {
            self.parked.entry(guid).or_default().pose = Some(pose);
        }
    }

    fn set_rooted_of(&mut self, guid: u64, rooted: bool) {
        if guid == self.mover {
            self.rooted = rooted;
        } else {
            self.parked.entry(guid).or_default().rooted = rooted;
        }
    }

    /// Hand the active mover to `to` (AzerothCore `Player::SetMover`): park the current one with
    /// its pose and root, and take `to`'s.
    fn switch_mover(&mut self, to: u64) {
        if to == self.mover {
            return;
        }
        let parked = ParkedMover {
            pose: self.ground_pose,
            rooted: self.rooted,
        };
        self.parked.insert(self.mover, parked);
        let taken = self.parked.remove(&to).unwrap_or_default();
        self.ground_pose = taken.pose;
        self.rooted = taken.rooted;
        self.mover = to;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PendingAuthorityOrder {
    UnsetCanFly(UnsetCanFlyOrder),
    ForceRoot {
        order: ForceRootOrder,
        rooted: bool,
    },
    /// Every other self order. A near teleport waits for the controller's `TeleportAck`; the
    /// rest the writer acknowledges from the last reported pose.
    Mover(MoverOrder),
}

#[derive(Clone)]
struct MovementAuthorityHandle {
    shared: Arc<MovementAuthority>,
    epoch: u64,
    self_guid: u64,
}

impl MovementAuthority {
    fn install(self: &Arc<Self>, epoch: u64, self_guid: u64) -> MovementAuthorityHandle {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        *state = MovementAuthorityState {
            epoch,
            self_guid,
            self_created: false,
            self_control: false,
            control_decided: false,
            ground_pose: None,
            pending_orders: std::collections::VecDeque::new(),
            last_order_counter: None,
            rooted: false,
            taxi_spline: None,
            logout_pending: false,
            transfer_sequence: 0,
            transfer_ack: None,
            transfer_waiting: false,
            transfer_transport: false,
            mover: self_guid,
            parked: std::collections::HashMap::new(),
            vehicles: std::collections::HashSet::new(),
            riders: std::collections::HashMap::new(),
        };
        MovementAuthorityHandle {
            shared: Arc::clone(self),
            epoch,
            self_guid,
        }
    }
}

impl MovementAuthorityHandle {
    fn observe_self_create(&self, guid: u64, movement: &MovementPosition) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(());
        }
        if state.transfer_waiting {
            return Ok(());
        }
        if guid != self.self_guid {
            state.self_control = false;
            state.control_decided = true;
            bail!("Wrath self create named foreign mover {guid:#018x}");
        }
        // A transport NEW_WORLD is deck-local; only the post-ACK self create supplies
        // AzerothCore's world pose and full transport tail (MovementHandler.cpp).
        if state.transfer_transport
            && !matches!(movement, MovementPosition::Living(info)
                if info.flags & wrath_ground::MOVE_ON_TRANSPORT != 0 && info.transport.is_some())
        {
            return Ok(());
        }
        let reported = match movement {
            MovementPosition::Living(movement) => Ok(living_pose(movement)),
            MovementPosition::Stationary(position) => {
                GroundPose::stationary(position).map(Into::into)
            }
            _ => Err(wrath_authority::AuthorityCodecError::UnsupportedGroundState),
        };
        let pose = reported.context("Wrath self create pose")?;
        state.set_pose_of(self.self_guid, pose);
        state.self_created = true;
        // AzerothCore calls SetMover(this) before UpdateVisibilityForPlayer emits this player's
        // CREATE_OBJECT with UPDATEFLAG_SELF (Player.cpp:11816; Object.cpp:197). It does not send
        // an initial SMSG_CLIENT_CONTROL_UPDATE, so that self flag is the server's normal-login
        // confirmation of both self creation and initial self control unless an explicit control
        // decision for this epoch arrived first.
        if !state.control_decided {
            state.self_control = true;
        }
        Ok(())
    }

    /// Another unit's create with a living movement block: its pose, should the server hand it to us
    /// (a vehicle or a possessed unit) and order it before we report it.
    fn observe_unit_create(&self, guid: u64, movement: &MovementPosition) {
        let MovementPosition::Living(movement) = movement else {
            return;
        };
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid || guid == self.self_guid
        {
            return;
        }
        state.set_pose_of(guid, living_pose(movement));
    }

    fn observe_server_pose(
        &self,
        guid: u64,
        movement: &wrathplus_protocol::wrath_gameplay::MovementInfo,
    ) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || !state.self_created
            || !(guid == self.self_guid || state.drives_or_parked(guid))
        {
            return Ok(());
        }
        let mut movement = movement.clone();
        // ROOT is an authority bit, not locomotion. Its command/FIFO owns permission; an echo
        // updates current XYZO only and must never grant control or clear pending orders.
        movement.flags &= !wrath_authority::MOVEMENT_FLAG_ROOT;
        // A server-authored pose in a state the writer does not report (flight) keeps the last
        // reported pose rather than ending the session.
        match OnFootPose::from_movement(&movement) {
            Ok(pose) => state.set_pose_of(guid, pose),
            Err(error) => bevy::log::debug!(
                "net(wrath): pose of {guid:#x} not representable ({error}); last pose kept"
            ),
        }
        Ok(())
    }

    /// `SMSG_CLIENT_CONTROL_UPDATE`. AzerothCore's `Player::SetClientControl` makes the unit our
    /// mover when it allows movement (`SetMover`), so a grant for a vehicle or a possessed unit
    /// hands it to us with the pose we last saw for it; a refusal for the active mover holds it.
    fn observe_control(&self, mover: u64, allow_move: bool) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(());
        }
        state.control_decided = true;
        if mover == state.mover {
            state.self_control = allow_move;
        } else if allow_move {
            state.switch_mover(mover);
            state.self_control = true;
            if state.ground_pose.is_none() {
                bevy::log::warn!(
                    "net(wrath): control of {mover:#018x} granted before any pose of it; its orders wait for the first report"
                );
            }
        } else {
            // A refusal for a unit we do not drive changes nothing (`SetMover` runs only on allow).
            bevy::log::debug!("net(wrath): control refusal for inactive mover {mover:#018x}");
        }
        Ok(())
    }

    /// The vehicle we drive and its halted pose, for a dismiss or seat change: only while we drive a
    /// unit other than our body with no order pending. The halted pose becomes the reported one.
    fn with_vehicle_pose(
        &self,
        send: impl FnOnce(u64, OnFootPose) -> Result<()>,
    ) -> Option<Result<()>> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || state.mover == self.self_guid
            || !state.self_control
            || !state.pending_orders.is_empty()
        {
            return None;
        }
        let pose = state.ground_pose?.halted();
        let result = send(state.mover, pose);
        if result.is_ok() {
            state.ground_pose = Some(pose);
        }
        Some(result)
    }

    /// Vehicles and seats from an object block: a create's `UPDATEFLAG_VEHICLE` id, any living
    /// movement's transport seat, and units leaving the world.
    fn observe_vehicles(
        &self,
        event: &WrathEvent,
        out: &mut Vec<wrathplus_protocol::wrath_vehicle::VehicleEvent>,
    ) {
        use wrathplus_protocol::wrath_vehicle::VehicleEvent;
        let gone = |guid: u64, out: &mut Vec<VehicleEvent>| {
            let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.vehicles.remove(&guid) | state.riders.remove(&guid).is_some() {
                out.push(VehicleEvent::Gone { unit: guid });
            }
        };
        match event {
            WrathEvent::Destroy { guid, .. } => gone(*guid, out),
            WrathEvent::Object(object) => match object.as_ref() {
                ObjectUpdate::OutOfRange(guids) => {
                    for &guid in guids {
                        gone(guid, out);
                    }
                }
                ObjectUpdate::Create { guid, movement, .. } => {
                    if let Some((vehicle_id, _)) = movement.vehicle {
                        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
                        state.vehicles.insert(*guid);
                        out.push(VehicleEvent::Kit {
                            unit: *guid,
                            vehicle_id,
                        });
                    }
                    if let MovementPosition::Living(info) = &movement.position {
                        out.extend(self.observe_seat(*guid, movement_seat(info)));
                    }
                }
                ObjectUpdate::Movement { guid, movement } => {
                    if let MovementPosition::Living(info) = &movement.position {
                        out.extend(self.observe_seat(*guid, movement_seat(info)));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// `passenger` is seen in `seat`: an event only when that changes.
    fn observe_seat(
        &self,
        passenger: u64,
        seat: Option<(u64, i8)>,
    ) -> Option<wrathplus_protocol::wrath_vehicle::VehicleEvent> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return None;
        }
        let previous = match seat {
            Some(seat) => state.riders.insert(passenger, seat),
            None => state.riders.remove(&passenger),
        };
        (previous != seat)
            .then_some(wrathplus_protocol::wrath_vehicle::VehicleEvent::Seat { passenger, seat })
    }

    /// The controller claims `guid` with `CMSG_SET_ACTIVE_MOVER`: only the mover the server handed
    /// us. Root and pending orders do not hold the claim; the reference sends it at hand-over.
    fn claim_mover(&self, guid: u64, send: impl FnOnce() -> Result<()>) -> Option<Result<()>> {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || !state.self_created
            || state.mover != guid
        {
            return None;
        }
        Some(send())
    }

    fn revoke(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch == self.epoch && state.self_guid == self.self_guid {
            state.logout_pending = false;
            state.self_control = false;
            state.control_decided = true;
            state.taxi_spline = None;
            state.transfer_ack = None;
        }
    }

    #[cfg(test)]
    fn with_movement_send<T>(&self, send: impl FnOnce() -> T) -> Option<T> {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || !state.self_created
            || !state.self_control
            || !state.pending_orders.is_empty()
            || state.rooted
        {
            return None;
        }
        // Hold the same mutex used by reader-side revocation through the socket write. Once a
        // revoke acquires it, no later movement packet can pass an earlier authority check.
        Some(send())
    }

    /// Targeting/melee require this session's observed self and control, but root does not
    /// prevent a player from selecting or swinging. Serialize against reader-side retirement.
    fn with_player_action<T>(&self, send: impl FnOnce() -> T) -> Option<T> {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || !state.self_created
            || !state.self_control
        {
            return None;
        }
        Some(send())
    }

    #[cfg(test)]
    fn with_ground_send(
        &self,
        pose: GroundPose,
        send: impl FnOnce() -> Result<()>,
    ) -> Option<Result<()>> {
        self.with_on_foot_send(self.self_guid, pose.into(), |_, _| send())
    }

    #[cfg(test)]
    fn with_self_send(
        &self,
        pose: OnFootPose,
        send: impl FnOnce() -> Result<()>,
    ) -> Option<Result<()>> {
        self.with_on_foot_send(self.self_guid, pose, |_, _| send())
    }

    /// One movement packet from the controller for `claimed`, the mover it last claimed: sent only
    /// while that is the active mover, it is controllable and no order is pending. ROOT follows the
    /// root orders, not the controller: a rooted mover reports it and may only turn (a moving packet
    /// is dropped, AzerothCore would drop it too), and a free one never reports it.
    fn with_on_foot_send(
        &self,
        claimed: u64,
        pose: OnFootPose,
        send: impl FnOnce(u64, OnFootPose) -> Result<()>,
    ) -> Option<Result<()>> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch
            || state.self_guid != self.self_guid
            || !state.self_created
            || !state.self_control
            || !state.pending_orders.is_empty()
            || state.mover != claimed
        {
            return None;
        }
        let mut pose = pose;
        if state.rooted {
            if pose.flags & ROOTED_MOVING != 0 || pose.fall.is_some() {
                return None;
            }
            pose.flags |= wrath_ground::MOVE_ROOT;
        } else {
            pose.flags &= !wrath_ground::MOVE_ROOT;
        }
        let result = send(claimed, pose);
        if result.is_ok() {
            state.ground_pose = Some(pose);
        }
        Some(result)
    }

    fn raise_unset_barrier(&self, order: UnsetCanFlyOrder) -> Result<bool> {
        self.raise_authority_order(PendingAuthorityOrder::UnsetCanFly(order))
    }

    fn raise_root_barrier(&self, order: ForceRootOrder, rooted: bool) -> Result<bool> {
        self.raise_authority_order(PendingAuthorityOrder::ForceRoot { order, rooted })
    }

    /// Queue an order behind the barrier. `Ok(false)`: it names a unit we neither drive nor parked,
    /// whose ACK AzerothCore would refuse (`guid != m_mover`); it is logged and not queued, but its
    /// counter still advances the session's.
    fn raise_authority_order(&self, pending: PendingAuthorityOrder) -> Result<bool> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            bail!("stale Wrath movement-authority order");
        }
        let order = match pending {
            PendingAuthorityOrder::UnsetCanFly(order)
            | PendingAuthorityOrder::ForceRoot { order, .. } => order,
            PendingAuthorityOrder::Mover(order) => UnsetCanFlyOrder {
                mover_guid: order.mover_guid(),
                counter: order.counter(),
            },
        };
        let known = order.mover_guid == self.self_guid || state.drives_or_parked(order.mover_guid);
        if state.last_order_counter.is_some_and(|previous| {
            let advance = order.counter.wrapping_sub(previous);
            advance == 0 || advance >= 0x8000_0000
        }) {
            state.self_control = false;
            state.control_decided = true;
            bail!(
                "duplicate or stale Wrath movement-authority counter {} after {:?}",
                order.counter,
                state.last_order_counter
            );
        }
        // AzerothCore reconnect login may send an initial UNROOT before initial self packets.
        state.last_order_counter = Some(order.counter);
        if !known {
            bevy::log::warn!(
                "net(wrath): movement order for unknown mover {:#018x} ignored",
                order.mover_guid
            );
            return Ok(false);
        }
        state.pending_orders.push_back(pending);
        Ok(true)
    }

    fn send_root_ack(
        &self,
        order: ForceRootOrder,
        rooted: bool,
        timestamp: u32,
        mut send: impl FnMut(u32, &[u8]) -> Result<()>,
    ) -> Result<UnsetAckResult> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(UnsetAckResult::Stale);
        }
        let expected = PendingAuthorityOrder::ForceRoot { order, rooted };
        if state.waits_for_teleport(expected) {
            return Ok(UnsetAckResult::PendingTeleport);
        }
        if !state.drives_or_parked(order.mover_guid)
            || state.pending_orders.front() != Some(&expected)
        {
            state.self_control = false;
            state.control_decided = true;
            bail!("Wrath force-root ACK did not match the pending order head");
        }
        let Some(mut pose) = state.pose_of(order.mover_guid) else {
            return Ok(UnsetAckResult::PendingSelfPose);
        };
        // The medium and granted modes alone are not motion: a still swimmer, flier or passenger
        // needs no STOP.
        if rooted && pose.halted() != pose {
            pose = pose.halted();
            let stop = pose.encode(wrath_ground::MSG_MOVE_STOP, order.mover_guid, timestamp)?;
            send(wrath_ground::MSG_MOVE_STOP, &stop)?;
            // STOP was successfully written and became the last pose reported to the server. Keep
            // that honest progress if the following ACK write fatally fails; this epoch is retired
            // rather than retried through a socket whose delivery state is no longer trustworthy.
            state.set_pose_of(order.mover_guid, pose);
        }
        let body = wrath_authority::encode_force_root_on_foot_ack(
            order.mover_guid,
            order.counter,
            pose,
            timestamp,
            rooted,
        )?;
        send(
            if rooted {
                wrath_authority::CMSG_FORCE_MOVE_ROOT_ACK
            } else {
                wrath_authority::CMSG_FORCE_MOVE_UNROOT_ACK
            },
            &body,
        )?;
        let retired = state.pending_orders.pop_front();
        debug_assert_eq!(retired, Some(expected));
        state.set_rooted_of(order.mover_guid, rooted);
        Ok(UnsetAckResult::Sent)
    }

    fn send_unset_ack(
        &self,
        order: UnsetCanFlyOrder,
        timestamp: u32,
        send: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<UnsetAckResult> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(UnsetAckResult::Stale);
        }
        let expected = PendingAuthorityOrder::UnsetCanFly(order);
        if state.waits_for_teleport(expected) {
            return Ok(UnsetAckResult::PendingTeleport);
        }
        if !state.drives_or_parked(order.mover_guid)
            || state.pending_orders.front() != Some(&expected)
        {
            state.self_control = false;
            state.control_decided = true;
            bail!("Wrath UNSET_CAN_FLY ACK did not match the pending order head");
        }
        let Some(pose) = state.pose_of(order.mover_guid) else {
            return Ok(UnsetAckResult::PendingSelfPose);
        };
        let body = wrath_authority::encode_unset_can_fly_on_foot_ack(
            order.mover_guid,
            order.counter,
            pose,
            timestamp,
        )?;
        send(&body)?;
        let landed = pose.landed();
        state.set_pose_of(
            order.mover_guid,
            OnFootPose {
                flags: landed.flags & !wrath_ground::MOVE_CAN_FLY,
                ..landed
            },
        );
        let retired = state.pending_orders.pop_front();
        debug_assert_eq!(retired, Some(expected));
        Ok(UnsetAckResult::Sent)
    }

    fn raise_mover_barrier(&self, order: MoverOrder) -> Result<bool> {
        if !self.raise_authority_order(PendingAuthorityOrder::Mover(order))? {
            return Ok(false);
        }
        // A near teleport ends any flight path: no SPLINE_DONE may follow it.
        if matches!(order, MoverOrder::TeleportNear { .. }) {
            let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.epoch == self.epoch && state.self_guid == self.self_guid {
                state.taxi_spline = None;
            }
        }
        Ok(true)
    }

    /// Acknowledge the head speed, mode, knockback or collision-height order from the last pose
    /// reported to the server, as root and unset-can-fly do. A near teleport is never acknowledged
    /// here: it waits for the controller's snap ([`Self::send_teleport_ack`]).
    fn send_mover_ack(
        &self,
        order: MoverOrder,
        timestamp: u32,
        send: impl FnOnce(u32, &[u8]) -> Result<()>,
    ) -> Result<UnsetAckResult> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(UnsetAckResult::Stale);
        }
        let expected = PendingAuthorityOrder::Mover(order);
        let guid = order.mover_guid();
        if state.waits_for_teleport(expected) {
            return Ok(UnsetAckResult::PendingTeleport);
        }
        if !state.drives_or_parked(guid)
            || state.pending_orders.front() != Some(&expected)
            || matches!(order, MoverOrder::TeleportNear { .. })
        {
            state.self_control = false;
            state.control_decided = true;
            bail!("Wrath mover-order ACK did not match the pending order head");
        }
        let Some(pose) = state.pose_of(guid) else {
            return Ok(UnsetAckResult::PendingSelfPose);
        };
        let (opcode, body, reported) = match order {
            MoverOrder::Speed {
                kind,
                counter,
                speed,
                ..
            } => (
                kind.ack_opcode(),
                wrath_authority::encode_value_ack(guid, counter, pose, timestamp, speed)?,
                pose,
            ),
            MoverOrder::CollisionHeight {
                counter, height, ..
            } => (
                wrath_authority::CMSG_MOVE_SET_COLLISION_HGT_ACK,
                wrath_authority::encode_value_ack(guid, counter, pose, timestamp, height)?,
                pose,
            ),
            MoverOrder::Mode {
                kind,
                apply,
                counter,
                ..
            } => {
                let body =
                    wrath_authority::encode_mode_ack(guid, counter, pose, timestamp, kind, apply)?;
                // Water walking, slow fall and can-fly are bits the mover keeps reporting (losing
                // can-fly lands a flier); hover and gravity stay outside the reported pose.
                let mut reported = if kind == wrath_authority::ModeOrderKind::CanFly && !apply {
                    pose.landed()
                } else {
                    pose
                };
                let bit = kind.flag()
                    & (wrathplus_protocol::wrath_ground::GROUND_FLAG_MASK
                        | wrathplus_protocol::wrath_ground::MOVE_CAN_FLY);
                if apply {
                    reported.flags |= bit;
                } else {
                    reported.flags &= !bit;
                }
                (kind.ack_opcode(apply), body, reported)
            }
            MoverOrder::KnockBack { counter, .. } => (
                wrath_authority::CMSG_MOVE_KNOCK_BACK_ACK,
                wrath_authority::encode_knock_back_ack(guid, counter, pose, timestamp, &order)?,
                pose.landed(),
            ),
            MoverOrder::TeleportNear { .. } => unreachable!("refused above"),
        };
        send(opcode, &body)?;
        state.set_pose_of(guid, reported);
        let retired = state.pending_orders.pop_front();
        debug_assert_eq!(retired, Some(expected));
        Ok(UnsetAckResult::Sent)
    }

    /// The controller snapped to a near-teleport destination: acknowledge it (`MSG_MOVE_TELEPORT_ACK`)
    /// only when it is the head order, so no movement from the old pose can follow the ACK.
    fn send_teleport_ack(
        &self,
        guid: u64,
        counter: u32,
        timestamp: u32,
        send: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<bool> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid || guid != self.self_guid
        {
            return Ok(false);
        }
        let Some(&PendingAuthorityOrder::Mover(
            order @ MoverOrder::TeleportNear {
                counter: pending,
                position,
                orientation,
                transport,
                ..
            },
        )) = state.pending_orders.front()
        else {
            return Ok(false);
        };
        if pending != counter {
            return Ok(false);
        }
        send(&wrath_authority::encode_teleport_ack(
            self.self_guid,
            counter,
            timestamp,
        ))?;
        state.ground_pose = Some(OnFootPose {
            flags: if transport.is_some() {
                wrath_ground::MOVE_ON_TRANSPORT
            } else {
                0
            },
            position,
            orientation,
            fall_time: 0,
            fall: None,
            pitch: None,
            transport,
        });
        let retired = state.pending_orders.pop_front();
        debug_assert_eq!(retired, Some(PendingAuthorityOrder::Mover(order)));
        Ok(true)
    }
}

impl MovementAuthorityState {
    /// Near teleports (MSG_MOVE_TELEPORT) wait for the controller snap; subsequent ACKs must
    /// retain receipt order and use the destination pose, not overtake the teleport ACK.
    fn waits_for_teleport(&self, expected: PendingAuthorityOrder) -> bool {
        matches!(
            self.pending_orders.front(),
            Some(PendingAuthorityOrder::Mover(
                MoverOrder::TeleportNear { .. }
            ))
        ) && !matches!(
            expected,
            PendingAuthorityOrder::Mover(MoverOrder::TeleportNear { .. })
        ) && self.pending_orders.get(1) == Some(&expected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsetAckResult {
    Sent,
    PendingSelfPose,
    PendingTeleport,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrathRoute {
    GroundMovement,
    TaxiSplineDone,
    SetActiveMover,
    NotActiveMover,
    NativeVehicleControl,
    Logout,
    LogoutCancel,
    NativeItemQuery,
    NativeMelee,
    NativeLoot,
    NativeSession,
    NativeLfg,
    NativeGuild,
    NativePetNameQuery,
    NativeWho,
    NativeMailMutation,
    NativeQuestsCompletedQuery,
    NativeUiTimerQuery,
    NativeMirrorImageQuery,
    NativeInstanceControl,
    NativeInventory,
    NativeExchange,
    NativeSpellControl,
    NativeInspectAchievements,
    NativeCalendar,
    NativeQuest,
    NativeParty,
    NativeChatLine,
    NativeChannel,
    NativeBattleground,
    NativeContacts,
    NativeDeath,
    NativeSessionMeta,
    NativeAppearance,
    NativePetition,
    NativeRaidBrowser,
    NativeHelp,
    NativeReferFriend,
    NativeWorldObjects,
    NativeSessionMisc,
    NativeInspect,
    NativeGuildBank,
    NativeFaction,
    NativeWorldportAck,
    NativeTeleportAck,
    NativeMovementMisc,
    NativeArenaTeam,
    DisabledMovement,
    DisabledGameAction,
}

fn route(command: &ClientCommand) -> WrathRoute {
    match command {
        ClientCommand::GuildQuery { .. }
        | ClientCommand::GuildInfoRequest
        | ClientCommand::GuildRosterRequest
        | ClientCommand::GuildInvite { .. }
        | ClientCommand::GuildAccept
        | ClientCommand::GuildDecline
        | ClientCommand::GuildPromote { .. }
        | ClientCommand::GuildDemote { .. }
        | ClientCommand::GuildRemove { .. }
        | ClientCommand::GuildMotd { .. }
        | ClientCommand::GuildCreate { .. }
        | ClientCommand::GuildLeave
        | ClientCommand::GuildDisband
        | ClientCommand::GuildLeader { .. }
        | ClientCommand::GuildAddRank { .. }
        | ClientCommand::GuildDelRank
        | ClientCommand::GuildSetPublicNote { .. }
        | ClientCommand::GuildSetOfficerNote { .. }
        | ClientCommand::GuildInfoText { .. }
        | ClientCommand::SaveGuildEmblem { .. }
        | ClientCommand::TabardVendorActivate { .. }
        | ClientCommand::GuildEventLogQuery
        | ClientCommand::NativeGuildRank(_) => WrathRoute::NativeGuild,
        ClientCommand::NativeQuestsCompletedQuery => WrathRoute::NativeQuestsCompletedQuery,
        ClientCommand::NativeInstanceControl(_) => WrathRoute::NativeInstanceControl,
        ClientCommand::NativeInventory(_)
        | ClientCommand::OpenItem { .. }
        | ClientCommand::WrapItem { .. }
        | ClientCommand::AutoEquipItem { .. }
        | ClientCommand::SetAmmo { .. }
        | ClientCommand::SwapInvItem { .. }
        | ClientCommand::SwapItem { .. }
        | ClientCommand::AutoStoreBagItem { .. }
        | ClientCommand::SplitItem { .. }
        | ClientCommand::DestroyItem { .. }
        | ClientCommand::BankerActivate { .. }
        | ClientCommand::BuyBankSlot { .. }
        | ClientCommand::AutoBankItem { .. }
        | ClientCommand::AutoStoreBankItem { .. }
        | ClientCommand::PageTextQuery { .. } => WrathRoute::NativeInventory,
        ClientCommand::NativeInspectAchievements { .. } => WrathRoute::NativeInspectAchievements,
        ClientCommand::NativeCalendar(_) => WrathRoute::NativeCalendar,
        ClientCommand::NativeQuest(_) | ClientCommand::QuestPushResult { .. } => {
            WrathRoute::NativeQuest
        }
        ClientCommand::Chat { .. } | ClientCommand::AddonMessage { .. } => {
            WrathRoute::NativeChatLine
        }
        ClientCommand::SetFactionAtWar { .. }
        | ClientCommand::SetFactionInactive { .. }
        | ClientCommand::SetWatchedFaction { .. } => WrathRoute::NativeFaction,
        ClientCommand::NativeInspect(_)
        | ClientCommand::Inspect { .. }
        | ClientCommand::LearnTalent { .. }
        | ClientCommand::TalentWipeConfirm { .. } => WrathRoute::NativeInspect,
        ClientCommand::NativeSessionMeta(_)
        | ClientCommand::TutorialFlag { .. }
        | ClientCommand::TutorialClear
        | ClientCommand::TutorialReset
        | ClientCommand::PlayedTime
        | ClientCommand::NextCinematicCamera
        | ClientCommand::CompleteCinematic
        | ClientCommand::AreaTrigger { .. }
        | ClientCommand::SetActionBarToggles { .. } => WrathRoute::NativeSessionMeta,
        ClientCommand::NativeHelp(_)
        | ClientCommand::GmTicketGet
        | ClientCommand::GmTicketDelete
        | ClientCommand::GmTicketSystemStatus => WrathRoute::NativeHelp,
        ClientCommand::NativeReferFriend(_) => WrathRoute::NativeReferFriend,
        // Chests, doors, mailboxes, herbs, ore and fishing bobbers: build 12340 keeps the full guid.
        ClientCommand::NativeSessionMisc(_)
        | ClientCommand::QueryTime
        | ClientCommand::UnlearnSkill { .. } => WrathRoute::NativeSessionMisc,
        ClientCommand::NativeWorldObjects(_) | ClientCommand::GameObjUse { .. } => {
            WrathRoute::NativeWorldObjects
        }
        ClientCommand::NativeDeath(_)
        | ClientCommand::RepopRequest
        | ClientCommand::CorpseQuery
        | ClientCommand::ReclaimCorpse { .. }
        | ClientCommand::SummonResponse { .. }
        | ClientCommand::SelfRes => WrathRoute::NativeDeath,
        ClientCommand::FriendListRequest
        | ClientCommand::AddFriend { .. }
        | ClientCommand::DelFriend { .. }
        | ClientCommand::AddIgnore { .. }
        | ClientCommand::DelIgnore { .. }
        | ClientCommand::DuelAccepted { .. }
        | ClientCommand::DuelCancelled { .. }
        | ClientCommand::SetContactNotes { .. } => WrathRoute::NativeContacts,
        ClientCommand::NativeAppearance(_) => WrathRoute::NativeAppearance,
        ClientCommand::PetitionShowList { .. }
        | ClientCommand::PetitionBuy { .. }
        | ClientCommand::PetitionShowSignatures { .. }
        | ClientCommand::PetitionSign { .. }
        | ClientCommand::OfferPetition { .. }
        | ClientCommand::TurnInPetition { .. }
        | ClientCommand::PetitionQuery { .. }
        | ClientCommand::PetitionRename { .. }
        | ClientCommand::PetitionDecline { .. } => WrathRoute::NativePetition,
        ClientCommand::NativeGuildBank(_)
        | ClientCommand::NativeGuildBankSwap(_)
        | ClientCommand::NativeGuildBankTab(_)
        | ClientCommand::NativeGuildBankText(_) => WrathRoute::NativeGuildBank,
        ClientCommand::NativeBattleground(_)
        | ClientCommand::BattlemasterHello { .. }
        | ClientCommand::AreaSpiritHealerQuery { .. }
        | ClientCommand::AreaSpiritHealerQueue { .. }
        | ClientCommand::SpiritHealerActivate { .. }
        | ClientCommand::InspectHonorStats { .. }
        | ClientCommand::TogglePvp
        | ClientCommand::RequestBattlefieldScoreData
        | ClientCommand::RequestBattlefieldPositions
        | ClientCommand::LeaveBattlefield => WrathRoute::NativeBattleground,
        ClientCommand::NativeRaidBrowser(_) => WrathRoute::NativeRaidBrowser,
        ClientCommand::JoinChannel { .. }
        | ClientCommand::ClearChannelWatch
        | ClientCommand::DeclineChannelInvite { .. }
        | ClientCommand::LeaveChannel { .. }
        | ClientCommand::ChannelList { .. }
        | ClientCommand::ChannelOwner { .. }
        | ClientCommand::ChannelSetOwner { .. }
        | ClientCommand::ChannelPassword { .. }
        | ClientCommand::ChannelModerator { .. }
        | ClientCommand::ChannelUnmoderator { .. }
        | ClientCommand::ChannelMute { .. }
        | ClientCommand::ChannelUnmute { .. }
        | ClientCommand::ChannelInvite { .. }
        | ClientCommand::ChannelKick { .. }
        | ClientCommand::ChannelBan { .. }
        | ClientCommand::ChannelUnban { .. }
        | ClientCommand::ChannelAnnouncements { .. }
        | ClientCommand::ChannelModerate { .. }
        | ClientCommand::TextEmote { .. }
        | ClientCommand::ChatIgnored { .. } => WrathRoute::NativeChannel,
        ClientCommand::NativeParty(_)
        | ClientCommand::GroupUninvite { .. }
        | ClientCommand::GroupSetLeader { .. }
        | ClientCommand::MinimapPing { .. }
        | ClientCommand::RandomRoll { .. } => WrathRoute::NativeParty,
        ClientCommand::NativeExchange(_)
        | ClientCommand::AuctionHello { .. }
        | ClientCommand::AuctionListItems { .. }
        | ClientCommand::AuctionListOwnerItems { .. }
        | ClientCommand::AuctionListBidderItems { .. }
        | ClientCommand::AuctionSellItem { .. }
        | ClientCommand::AuctionPlaceBid { .. }
        | ClientCommand::AuctionRemoveItem { .. }
        | ClientCommand::LootMasterGive { .. }
        | ClientCommand::InitiateTrade { .. }
        | ClientCommand::BeginTrade
        | ClientCommand::BusyTrade
        | ClientCommand::IgnoreTrade
        | ClientCommand::AcceptTrade
        | ClientCommand::UnacceptTrade
        | ClientCommand::CancelTrade
        | ClientCommand::SetTradeGold { .. }
        | ClientCommand::SetTradeItem { .. }
        | ClientCommand::ClearTradeItem { .. } => WrathRoute::NativeExchange,
        ClientCommand::NativeSpellControl(_)
        | ClientCommand::PetAction { .. }
        | ClientCommand::PetSetAction { .. }
        | ClientCommand::PetStopAttack { .. }
        | ClientCommand::PetCancelAura { .. }
        | ClientCommand::PetSpellAutocast { .. }
        | ClientCommand::PetAbandon { .. }
        | ClientCommand::PetRename { .. }
        | ClientCommand::CancelAutoRepeat
        | ClientCommand::CancelChannelling { .. }
        | ClientCommand::FarSight { .. }
        | ClientCommand::ResurrectResponse { .. } => WrathRoute::NativeSpellControl,
        ClientCommand::NativeMirrorImageQuery { .. } => WrathRoute::NativeMirrorImageQuery,
        ClientCommand::NativeUiTimerQuery => WrathRoute::NativeUiTimerQuery,
        ClientCommand::NativeMailMutation(_) => WrathRoute::NativeMailMutation,
        ClientCommand::NativeWhoCommand(_) => WrathRoute::NativeWho,
        ClientCommand::PetNameQuery { .. } => WrathRoute::NativePetNameQuery,
        ClientCommand::NativeWorldportAck(_) => WrathRoute::NativeWorldportAck,
        ClientCommand::ArenaTeamCommand(_) => WrathRoute::NativeArenaTeam,
        ClientCommand::LfgCommand(_) => WrathRoute::NativeLfg,
        ClientCommand::LootRoll { roll_type, .. } if *roll_type > 3 => {
            WrathRoute::DisabledGameAction
        }
        ClientCommand::SetActiveMover { .. } => WrathRoute::SetActiveMover,
        ClientCommand::Logout | ClientCommand::ForceLogout => WrathRoute::Logout,
        ClientCommand::LogoutCancel => WrathRoute::LogoutCancel,
        ClientCommand::Move { .. } => WrathRoute::GroundMovement,
        ClientCommand::ItemQuery { .. } => WrathRoute::NativeItemQuery,
        ClientCommand::Loot { .. }
        | ClientCommand::AutostoreLootItem { .. }
        | ClientCommand::LootMoney
        | ClientCommand::LootRelease { .. } => WrathRoute::NativeLoot,
        ClientCommand::SetSelection { .. }
        | ClientCommand::AttackSwing { .. }
        | ClientCommand::AttackStop
        | ClientCommand::StandStateChange { .. }
        | ClientCommand::SetSheathed { .. } => WrathRoute::NativeMelee,
        ClientCommand::GroupInvite { .. }
        | ClientCommand::GroupAccept
        | ClientCommand::GroupDecline
        | ClientCommand::PartyAssignment { .. }
        | ClientCommand::GroupLeave
        | ClientCommand::GroupRaidConvert
        | ClientCommand::LootMethod { .. }
        | ClientCommand::SetRaidTarget { .. }
        | ClientCommand::GroupChangeSubGroup { .. }
        | ClientCommand::GroupSwapSubGroup { .. }
        | ClientCommand::GroupAssistantLeader { .. }
        | ClientCommand::ReadyCheckStart
        | ClientCommand::ReadyCheckAnswer { .. }
        | ClientCommand::RequestPartyMemberStats { .. }
        | ClientCommand::NameQuery { .. }
        | ClientCommand::CreatureQuery { .. }
        | ClientCommand::GameObjectQuery { .. }
        | ClientCommand::SetActionButton { .. }
        | ClientCommand::CastSpell { .. }
        | ClientCommand::CastSpellAtDest { .. }
        | ClientCommand::NativePetCastSpell { .. }
        | ClientCommand::CastSpellAtSource { .. }
        | ClientCommand::CastSpellGameObject { .. }
        | ClientCommand::CastSpellItem { .. }
        | ClientCommand::CastSpellCorpse { .. }
        | ClientCommand::UseItem { .. }
        | ClientCommand::CancelCast { .. }
        | ClientCommand::CancelAura { .. }
        | ClientCommand::GossipHello { .. }
        | ClientCommand::GossipSelectOption { .. }
        | ClientCommand::NpcTextQuery { .. }
        | ClientCommand::QuestgiverStatusQuery { .. }
        | ClientCommand::QuestgiverHello { .. }
        | ClientCommand::QuestgiverQuery { .. }
        | ClientCommand::QuestgiverAccept { .. }
        | ClientCommand::QuestgiverComplete { .. }
        | ClientCommand::QuestgiverRequestReward { .. }
        | ClientCommand::QuestgiverChooseReward { .. }
        | ClientCommand::QuestQuery { .. }
        | ClientCommand::QuestlogRemove { .. }
        | ClientCommand::PushQuestToParty { .. }
        | ClientCommand::QuestConfirmAccept { .. }
        | ClientCommand::ListInventory { .. }
        | ClientCommand::BuyItem { .. }
        | ClientCommand::BuyItemInSlot { .. }
        | ClientCommand::SellItem { .. }
        | ClientCommand::BuybackItem { .. }
        | ClientCommand::RepairItem { .. }
        | ClientCommand::TaxiNodeStatusQuery { .. }
        | ClientCommand::TaxiQueryNodes { .. }
        | ClientCommand::ActivateTaxi { .. }
        | ClientCommand::ActivateTaxiExpress { .. }
        | ClientCommand::BinderActivate { .. }
        | ClientCommand::TrainerList { .. }
        | ClientCommand::ListStabledPets { .. }
        | ClientCommand::StablePet { .. }
        | ClientCommand::UnstablePet { .. }
        | ClientCommand::StableSwapPet { .. }
        | ClientCommand::BuyStableSlot { .. }
        | ClientCommand::TrainerBuySpell { .. }
        | ClientCommand::LootRoll { .. }
        | ClientCommand::GetMailList { .. }
        | ClientCommand::SetChannelWatch { .. }
        | ClientCommand::RequestRaidInfo
        | ClientCommand::GuildBankQuery(_)
        | ClientCommand::LfgRequestPlayerLockInfo
        | ClientCommand::LfgRequestPartyLockInfo
        | ClientCommand::ChannelDisplayList { .. }
        | ClientCommand::QueryNextMailTime => WrathRoute::NativeSession,
        // Deferred Wrath feature queries have no wire effect and never fabricate server state.
        ClientCommand::BattlefieldStatusRequest => WrathRoute::NativeBattleground,
        ClientCommand::MoveSplineDone { .. } => WrathRoute::TaxiSplineDone,
        ClientCommand::TeleportAck { .. } => WrathRoute::NativeTeleportAck,
        ClientCommand::MountSpecial
        | ClientCommand::MoveTimeSkipped { .. }
        | ClientCommand::NativeMovementMisc(_) => WrathRoute::NativeMovementMisc,
        ClientCommand::NotActiveMover { .. } => WrathRoute::NotActiveMover,
        ClientCommand::NativeVehicleControl(_) => WrathRoute::NativeVehicleControl,
        ClientCommand::WorldportAck => WrathRoute::DisabledMovement,
        _ => WrathRoute::DisabledGameAction,
    }
}

/// The one warning a failed command logs when it is dropped, or `None` when it ends the session.
/// A socket failure always ends it ([`SendFailed`](super::wrath_send_log::SendFailed)); any other
/// failure is the client's own (a body that would not encode or a value the codec refuses) and the
/// command is dropped with one warning, never a session end. The exceptions keep ordered server state: losing one would desync
/// it, so any failure on them is fatal, as before:
/// - `SetActiveMover`, `NotActiveMover`, `NativeVehicleControl`, `GroundMovement`: the mover
///   handoff and the movement-authority order FIFO (force root/unroot, speed and fly-flag ACKs
///   queue behind the self pose; an overflow or a mismatched ACK means the server and we disagree
///   about who moves and how);
/// - `NativeTeleportAck`, `NativeWorldportAck`, `TaxiSplineDone`: acknowledgements of a
///   server-ordered teleport, map transfer or flight, the server waits on each;
/// - `Logout`, `LogoutCancel`: the logout handshake the session lifecycle tracks.
///
/// The mover ACK path (root, fly-flag and mover-order ACKs) and the protocol replies the reader
/// queues go through the control arm, outside this rule, and stay fail-closed. A pose that fails to
/// encode on a movement route is dropped at its encode site ([`encoded_or_dropped`]) instead.
fn dropped_line(path: WrathRoute, error: &anyhow::Error) -> Option<String> {
    use WrathRoute as R;
    let fatal = error
        .downcast_ref::<super::wrath_send_log::SendFailed>()
        .is_some()
        || matches!(
            path,
            R::SetActiveMover
                | R::NotActiveMover
                | R::NativeVehicleControl
                | R::GroundMovement
                | R::NativeTeleportAck
                | R::NativeWorldportAck
                | R::TaxiSplineDone
                | R::Logout
                | R::LogoutCancel
        );
    (!fatal).then(|| format!("net(wrath): {path:?} not encodable, dropped: {error:#}"))
}

/// Only implemented wire routes grant command admission. Deferred queries have no writer.
/// Shape/value validation and epoch/capability checks remain at their existing boundaries.
pub(in crate::net) fn has_writer_route(command: &ClientCommand) -> bool {
    !matches!(
        route(command),
        WrathRoute::DisabledMovement | WrathRoute::DisabledGameAction
    )
}

fn writer_loop(
    commands: &Receiver<SessionCommand>,
    control: Receiver<WrathWriterControl>,
    control_tx: &Sender<WrathWriterControl>,
    clock: &Mutex<PingClock>,
    fatal: &Sender<EpochFatal>,
) {
    let mut writer: Option<(u64, WrathWriterSwap)> = None;
    let mut ping_tick = crossbeam_channel::never();
    let mut pre_self_timeout = crossbeam_channel::never();
    let mut warned = 0;
    let mut pending_authority = std::collections::VecDeque::new();
    loop {
        crossbeam_channel::select! {
            recv(control) -> message => match message {
                Ok(WrathWriterControl::Install { epoch, session, installed }) => {
                    // Install is ordered with retire/internal/fatal traffic. Movement remains
                    // disabled until the reader confirms this session's self create/control.
                    writer = Some((epoch, *session));
                    pending_authority.clear();
                    pre_self_timeout = crossbeam_channel::never();
                    warned = 0;
                    clock.lock().unwrap_or_else(|p| p.into_inner()).clear();
                    ping_tick = crossbeam_channel::after(PING_INTERVAL);
                    let _ = installed.send(Ok(()));
                }
                Ok(WrathWriterControl::Internal { epoch, command }) => {
                    let result = match writer.as_mut() {
                        Some((active_epoch, active)) if *active_epoch == epoch => match command {
                            WrathWriterCommand::Reply(reply) => {
                                active.writer.send_raw(reply.opcode, &reply.body)
                            }
                            WrathWriterCommand::UnsetCanFly(order) => queue_or_send_authority(
                                active,
                                PendingAuthorityOrder::UnsetCanFly(order),
                                &mut pending_authority,
                                &mut pre_self_timeout,
                            ),
                            WrathWriterCommand::ForceRoot { order, rooted } => queue_or_send_authority(
                                active,
                                PendingAuthorityOrder::ForceRoot { order, rooted },
                                &mut pending_authority,
                                &mut pre_self_timeout,
                            ),
                            WrathWriterCommand::Mover(order) => queue_or_send_authority(
                                active,
                                PendingAuthorityOrder::Mover(order),
                                &mut pending_authority,
                                &mut pre_self_timeout,
                            ),
                            WrathWriterCommand::PoseReady => {
                                drain_authority_queue(
                                    &mut pending_authority,
                                    &mut pre_self_timeout,
                                    |pending| send_pending_authority(active, pending),
                                )
                            }
                        },
                        _ => continue,
                    };
                    if let Err(error) = result {
                        let _ = control_tx.send(WrathWriterControl::Fatal {
                            epoch,
                            reason: format!("Wrath protocol reply failed: {error:#}"),
                        });
                    }
                }
                Ok(WrathWriterControl::Retire { epoch, retired }) => {
                    if writer.as_ref().is_some_and(|(active, _)| *active == epoch) {
                        // Disable timers in the same ordered retirement step, before any later install.
                        ping_tick = crossbeam_channel::never();
                        pre_self_timeout = crossbeam_channel::never();
                        writer = None;
                        pending_authority.clear();
                    }
                    let _ = retired.send(());
                }
                Ok(WrathWriterControl::Fatal { epoch, reason }) => {
                    if writer.as_ref().is_some_and(|(active, _)| *active == epoch) {
                        ping_tick = crossbeam_channel::never();
                        pre_self_timeout = crossbeam_channel::never();
                        writer = None;
                        pending_authority.clear();
                        let _ = fatal.send(EpochFatal { epoch, reason });
                    }
                }
                Err(_) => return,
            },
            recv(ping_tick) -> _ => {
                ping_tick = crossbeam_channel::never();
                let mut failed = None;
                if let Some((_epoch, active)) = writer.as_mut() {
                    ping_tick = crossbeam_channel::after(PING_INTERVAL);
                    let (sequence, last_rtt) = {
                        let mut clock = clock.lock().unwrap_or_else(|p| p.into_inner());
                        clock.sequence += 1;
                        clock.sent_at = Some(Instant::now());
                        (clock.sequence, clock.last_rtt_ms.unwrap_or(0))
                    };
                    let (opcode, body) = ping_packet(sequence, last_rtt);
                    failed = active.writer.send_raw(opcode, &body).err();
                }
                if let Some(error) = failed {
                    let epoch = writer.as_ref().map_or(0, |(epoch, _)| *epoch);
                    let _ = control_tx.send(WrathWriterControl::Fatal {
                        epoch,
                        reason: format!("Wrath ping failed: {error:#}"),
                    });
                }
            },
            recv(pre_self_timeout) -> _ => {
                pre_self_timeout = crossbeam_channel::never();
                if !pending_authority.is_empty() {
                    if let Some((epoch, active)) = writer.as_ref() {
                        expire_pre_self_orders(
                            &active.authority,
                            *epoch,
                            pending_authority.len(),
                            control_tx,
                        );
                    }
                }
            },
            recv(commands) -> command => {
                let Ok(SessionCommand { epoch, command }) = command else { return };
                let Some((active_epoch, active)) = writer.as_mut() else {
                    if warned < SEND_WARN_CAP {
                        bevy::log::warn!("net(wrath): dropping command without live session: {command:?}");
                        warned += 1;
                    }
                    continue;
                };
                if !accepts_epoch(epoch, *active_epoch) {
                    continue;
                }
                let path = route(&command);
                let result = match path {
                    WrathRoute::SetActiveMover => {
                        let ClientCommand::SetActiveMover { guid } = command else { unreachable!() };
                        // Only the mover the server handed us (`SMSG_CLIENT_CONTROL_UPDATE`); from
                        // here the controller's movement goes out under its guid (`0x729010`).
                        let authority = active.authority.clone();
                        match authority.claim_mover(guid, || {
                            let (opcode, body) = active_mover_packet(guid);
                            active.writer.send_raw(opcode, &body)
                        }) {
                            Some(result) => {
                                if result.is_ok() {
                                    active.active_mover = guid;
                                }
                                result
                            }
                            None => {
                                bevy::log::warn!("net(wrath): active-mover claim for {guid:#018x} not granted; withheld");
                                Ok(())
                            }
                        }
                    }
                    WrathRoute::NotActiveMover => send_not_active_mover(active, command),
                    WrathRoute::NativeVehicleControl => {
                        let ClientCommand::NativeVehicleControl(request) = command else { unreachable!() };
                        let authority = active.authority.clone();
                        let timestamp = active.clock.now_ms();
                        authority
                            .with_vehicle_pose(|vehicle, pose| {
                                let body = request.encode(vehicle, pose, timestamp);
                                match encoded_or_dropped("vehicle request", body) {
                                    Some(body) => active.writer.send_raw(request.opcode(), &body),
                                    None => Ok(()),
                                }
                            })
                            .unwrap_or_else(|| {
                                bevy::log::warn!("net(wrath): {request:?} without a driven vehicle; withheld");
                                Ok(())
                            })
                    }
                    WrathRoute::Logout => {
                        let authority = active.authority.clone();
                        authority.send_logout(|| {
                            let (opcode, body) = logout_packet();
                            active.writer.send_raw(opcode, &body)
                        })
                    }
                    WrathRoute::LogoutCancel => {
                        let authority = active.authority.clone();
                        authority.send_logout_cancel(|| {
                            let (opcode, body) = wrathplus_protocol::wrath_logout::encode_cancel();
                            active.writer.send_raw(opcode, &body)
                        })
                    }
                    WrathRoute::GroundMovement => send_ground_movement(active, command),
                    WrathRoute::TaxiSplineDone => {
                        let ClientCommand::MoveSplineDone { flags, pos, orientation, spline_id } = command else { unreachable!() };
                        active.authority.send_taxi_done(flags, pos, orientation, spline_id, active.clock.now_ms(), |opcode, body| active.writer.send_raw(opcode, body))
                    }
                    WrathRoute::NativeItemQuery => {
                        let ClientCommand::ItemQuery { entry, guid } = command else {
                            unreachable!()
                        };
                        let (opcode, body) = item_query_packet(entry, guid);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeMelee => send_melee(active, command),
                    WrathRoute::NativeLoot => send_loot(active, command),
                    WrathRoute::NativeWorldportAck => {
                        let ClientCommand::NativeWorldportAck(token) = command else { unreachable!() };
                        active.authority.send_worldport_ack(token, || active.writer.send_raw(wrathplus_protocol::wrath_transfer::MSG_MOVE_WORLDPORT_ACK, &[]))
                    }
                    WrathRoute::NativeInstanceControl => {
                        let ClientCommand::NativeInstanceControl(command) = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_instance_control::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeInventory => {
                        let (opcode, body) = wrathplus_protocol::wrath_inventory::encode(&inventory_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeInspectAchievements => {
                        let ClientCommand::NativeInspectAchievements { player } = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_achievement::encode_inspect_query(player);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeChatLine => match chat_line(command, active.chat_language) {
                        Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                        Err(error) => {
                            bevy::log::warn!("net(wrath): invalid chat line rejected before send: {error}");
                            continue;
                        }
                    },
                    WrathRoute::NativeFaction => {
                        use wrathplus_protocol::wrath_faction as f;
                        let (opcode, body) = match command {
                            ClientCommand::SetFactionAtWar { rep_list_id, at_war } => f::encode_at_war(rep_list_id, at_war),
                            ClientCommand::SetFactionInactive { rep_list_id, inactive } => f::encode_inactive(rep_list_id, inactive),
                            ClientCommand::SetWatchedFaction { rep_list_id } => f::encode_watched(rep_list_id),
                            _ => unreachable!(),
                        };
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeInspect => {
                        use wrathplus_protocol::wrath_inspect::{encode, Command as I};
                        let (opcode, body) = encode(&match command {
                            ClientCommand::NativeInspect(command) => command,
                            ClientCommand::Inspect { target } => I::Inspect { target },
                            ClientCommand::LearnTalent { talent_id, rank } => I::LearnTalent { talent_id, rank },
                            ClientCommand::TalentWipeConfirm { trainer } => I::TalentWipeConfirm { trainer },
                            _ => unreachable!(),
                        });
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeGuildBank => {
                        let packet = match command {
                            ClientCommand::NativeGuildBank(c) => Ok((c.opcode(), c.encode())),
                            ClientCommand::NativeGuildBankSwap(c) => Ok((c.opcode(), c.encode())),
                            ClientCommand::NativeGuildBankTab(c) => c.encode().map(|b| (c.opcode(), b)),
                            ClientCommand::NativeGuildBankText(c) => c.encode().map(|b| (c.opcode(), b)),
                            _ => unreachable!(),
                        };
                        match packet {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid guild-bank command rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeRaidBrowser => {
                        let ClientCommand::NativeRaidBrowser(command) = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_raid_browser::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativePetition => {
                        let (opcode, body) = wrathplus_protocol::wrath_petition::encode(&petition_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeAppearance => {
                        let ClientCommand::NativeAppearance(command) = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_appearance::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeSessionMeta => {
                        let (opcode, body) = wrathplus_protocol::wrath_session_meta::encode(&session_meta_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeHelp => match wrathplus_protocol::wrath_help::encode(&help_command(command)) {
                        Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                        Err(error) => {
                            bevy::log::warn!("net(wrath): invalid help request rejected before send: {error}");
                            continue;
                        }
                    },
                    WrathRoute::NativeSessionMisc => {
                        use wrathplus_protocol::wrath_session_misc::Command as S;
                        let command = match command {
                            ClientCommand::NativeSessionMisc(command) => command,
                            ClientCommand::QueryTime => S::QueryTime,
                            ClientCommand::UnlearnSkill { skill_id } => S::UnlearnSkill { skill: skill_id },
                            other => unreachable!("not a session-misc route: {other:?}"),
                        };
                        let (opcode, body) = wrathplus_protocol::wrath_session_misc::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeWorldObjects => {
                        use wrathplus_protocol::wrath_world_objects::Command as O;
                        let command = match command {
                            ClientCommand::NativeWorldObjects(command) => command,
                            ClientCommand::GameObjUse { guid } => O::Use { object: guid },
                            other => unreachable!("not a world-object route: {other:?}"),
                        };
                        let (opcode, body) = wrathplus_protocol::wrath_world_objects::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeReferFriend => {
                        let ClientCommand::NativeReferFriend(command) = command else {
                            unreachable!("not a refer-a-friend route")
                        };
                        let (opcode, body) = wrathplus_protocol::wrath_refer_friend::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeDeath => {
                        let (opcode, body) = wrathplus_protocol::wrath_death::encode(&death_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeContacts => match contacts_packet(command) {
                        Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                        Err(error) => {
                            bevy::log::warn!("net(wrath): invalid contact command rejected before send: {error}");
                            continue;
                        }
                    },
                    WrathRoute::NativeBattleground => {
                        let (opcode, body) = battleground_packet(command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeChannel => {
                        let (opcode, body) = wrathplus_protocol::wrath_chat_notices::encode(&channel_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeParty => {
                        let (opcode, body) = wrathplus_protocol::wrath_party::encode(&party_command(command));
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeQuest => match quest_packet(command) {
                        Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                        Err(error) => {
                            bevy::log::warn!("net(wrath): invalid native quest command rejected before send: {error}");
                            continue;
                        }
                    },
                    WrathRoute::NativeCalendar => {
                        let ClientCommand::NativeCalendar(command) = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_calendar::encode(&command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeExchange => {
                        let (opcode, body) = exchange_packet(command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeSpellControl => {
                        match super::spell_control_packet(command) {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid spell-control command rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeMirrorImageQuery => {
                        let ClientCommand::NativeMirrorImageQuery { guid } = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_mirror_image::encode_query(guid);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeUiTimerQuery => {
                        let (opcode, body) = wrathplus_protocol::wrath_ui_timer::encode_query();
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeQuestsCompletedQuery => {
                        let (opcode, body) = wrathplus_protocol::wrath_quests_completed::encode_query();
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeMailMutation => {
                        let ClientCommand::NativeMailMutation(command) = command else { unreachable!() };
                        match native_mail_packet(&command) {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid native mail rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeWho => {
                        let ClientCommand::NativeWhoCommand(command) = command else { unreachable!() };
                        match command.encode() {
                            Ok(body) => active.writer.send_raw(command.opcode(), &body),
                            Err(_) => {
                                bevy::log::warn!("net(wrath): invalid native WHO command rejected before send");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativePetNameQuery => {
                        let ClientCommand::PetNameQuery { pet_number, guid } = command else { unreachable!() };
                        let (opcode, body) = wrathplus_protocol::wrath_pet_name::encode_query(pet_number, guid);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeArenaTeam => {
                        let ClientCommand::ArenaTeamCommand(command) = command else { unreachable!() };
                        match arena_team_packet(&command) {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid arena command rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeGuild => {
                        match guild_packet(command) {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid guild command rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeLfg => {
                        let ClientCommand::LfgCommand(command) = command else { unreachable!() };
                        match lfg_packet(&command) {
                            Ok((opcode, body)) => active.writer.send_raw(opcode, &body),
                            Err(error) => {
                                bevy::log::warn!("net(wrath): invalid LFG command rejected before send: {error}");
                                continue;
                            }
                        }
                    }
                    WrathRoute::NativeSession => {
                        let (opcode, body) = session_packet(command);
                        active.writer.send_raw(opcode, &body)
                    }
                    WrathRoute::NativeTeleportAck => {
                        let ClientCommand::TeleportAck { guid, counter } = command else { unreachable!() };
                        let authority = active.authority.clone();
                        let timestamp = active.clock.now_ms();
                        match authority.send_teleport_ack(guid, counter, timestamp, |body| {
                            active.writer.send_raw(u32::from(wrath_authority::MSG_MOVE_TELEPORT_ACK), body)
                        }) {
                            Ok(true) => drain_authority_queue(
                                &mut pending_authority,
                                &mut pre_self_timeout,
                                |pending| send_pending_authority(active, pending),
                            ),
                            Ok(false) => {
                                bevy::log::warn!("net(wrath): teleport ACK {counter} for {guid:#x} is not the pending head order; not sent");
                                continue;
                            }
                            Err(error) => Err(error),
                        }
                    }
                    WrathRoute::NativeMovementMisc => {
                        use wrathplus_protocol::wrath_movement_misc::{encode, Command};
                        let native = match command {
                            ClientCommand::MountSpecial => Command::MountSpecialAnim,
                            ClientCommand::MoveTimeSkipped { guid, lag_ms } => Command::TimeSkipped { mover: guid, lag_ms },
                            ClientCommand::NativeMovementMisc(native) => native,
                            _ => unreachable!(),
                        };
                        let (opcode, body) = encode(&native);
                        active.writer.send_raw(opcode, &body)
                    }
                    // Not yet encoded for build 12340: dropped, never sent in Vanilla's format, and
                    // never fatal: the stock UI issues many of these routinely.
                    WrathRoute::DisabledMovement | WrathRoute::DisabledGameAction => {
                        if warned < SEND_WARN_CAP {
                            bevy::log::warn!("net(wrath): command not yet encoded for build 12340; dropped: {command:?}");
                            warned += 1;
                        }
                        continue;
                    }
                };
                if let Err(error) = result {
                    match dropped_line(path, &error) {
                        Some(line) => bevy::log::warn!("{line}"),
                        None => {
                            let _ = control_tx.send(WrathWriterControl::Fatal {
                                epoch,
                                reason: format!("Wrath lifecycle send failed: {error:#}"),
                            });
                        }
                    }
                }
            }
        }
    }
}

fn melee_packet(command: ClientCommand) -> Result<(u32, Vec<u8>)> {
    Ok(match command {
        ClientCommand::SetSelection { guid } => (
            wrath_combat::CMSG_SET_SELECTION,
            wrath_combat::encode_set_selection(guid),
        ),
        ClientCommand::AttackSwing { guid } => (
            wrath_combat::CMSG_ATTACK_SWING,
            wrath_combat::encode_attack_swing(guid),
        ),
        ClientCommand::AttackStop => (
            wrath_combat::CMSG_ATTACK_STOP,
            wrath_combat::encode_attack_stop(),
        ),
        ClientCommand::StandStateChange { state } => (
            wrath_combat::CMSG_STAND_STATE_CHANGE,
            wrath_combat::encode_stand_state_change(state)
                .context(opcode_label(wrath_combat::CMSG_STAND_STATE_CHANGE))?,
        ),
        ClientCommand::SetSheathed { state } => {
            let state = match state {
                0 => wrath_combat::SheathState::Unarmed,
                1 => wrath_combat::SheathState::Melee,
                2 => wrath_combat::SheathState::Ranged,
                _ => bail!(
                    "{}: invalid sheath state {state}",
                    opcode_label(wrath_combat::CMSG_SET_SHEATHED)
                ),
            };
            (
                wrath_combat::CMSG_SET_SHEATHED,
                wrath_combat::encode_set_sheathed(state),
            )
        }
        _ => bail!("not a native Wrath melee command"),
    })
}

fn send_melee(active: &mut WrathWriterSwap, command: ClientCommand) -> Result<()> {
    let (opcode, body) = melee_packet(command)?;
    let authority = active.authority.clone();
    authority
        .with_player_action(|| active.writer.send_raw(opcode, &body))
        .unwrap_or(Ok(()))
}

fn loot_packet(command: ClientCommand) -> Result<(u32, Vec<u8>)> {
    Ok(match command {
        ClientCommand::Loot { guid } if guid != 0 => {
            (wrath_loot::CMSG_LOOT, wrath_loot::encode_loot(guid))
        }
        ClientCommand::AutostoreLootItem { slot } if slot < wrath_loot::MAX_LOOT_ITEMS => (
            wrath_loot::CMSG_AUTOSTORE_LOOT_ITEM,
            wrath_loot::encode_autostore_loot_item(slot),
        ),
        ClientCommand::LootMoney => (wrath_loot::CMSG_LOOT_MONEY, wrath_loot::encode_loot_money()),
        ClientCommand::LootRelease { guid } if guid != 0 => (
            wrath_loot::CMSG_LOOT_RELEASE,
            wrath_loot::encode_loot_release(guid),
        ),
        ClientCommand::Loot { .. } => bail!("{}: no corpse", opcode_label(wrath_loot::CMSG_LOOT)),
        ClientCommand::AutostoreLootItem { slot } => bail!(
            "{}: slot {slot} past the loot window",
            opcode_label(wrath_loot::CMSG_AUTOSTORE_LOOT_ITEM)
        ),
        ClientCommand::LootRelease { .. } => {
            bail!("{}: no corpse", opcode_label(wrath_loot::CMSG_LOOT_RELEASE))
        }
        _ => bail!("not a native Wrath solo corpse-loot command"),
    })
}

// TODO(App LFG L3): guard the current proposal ID and session lifecycle before
// exposing ProposalResult through Lua/UI. This route is raw transport only.
fn lfg_packet(
    command: &wrathplus_protocol::wrath_lfg::commands::Command,
) -> std::io::Result<(u32, Vec<u8>)> {
    Ok((command.opcode(), command.encode()?))
}

/// Queries, the action bar and casting, in build 12340's layouts.
/// A chat-bar or addon line as build 12340's `CMSG_MESSAGECHAT`; a line naming no language
/// speaks `own_language`, an addon line `LANG_ADDON`.
fn chat_line(command: ClientCommand, own_language: u32) -> std::io::Result<(u32, Vec<u8>)> {
    use wrathplus_protocol::wrath_chat::{encode, Command, LANGUAGE_ADDON};
    let command = match command {
        ClientCommand::Chat {
            kind,
            target,
            text,
            language,
        } => {
            let chat_type = kind.wrath_chat_type();
            Command {
                chat_type,
                language: language.unwrap_or(own_language),
                // Whisper and channel name their target; the rest send none.
                target: matches!(chat_type, 0x07 | 0x11)
                    .then(|| target.unwrap_or_default().into_bytes()),
                text: text.into_bytes(),
            }
        }
        ClientCommand::AddonMessage {
            distribution,
            text,
            target,
        } => Command {
            chat_type: crate::net::wrath_addon_chat_type(distribution),
            language: LANGUAGE_ADDON,
            // The WHISPER lane's recipient (`0x07` requires one); every other lane has none.
            target: target.map(String::into_bytes),
            text: text.into_bytes(),
        },
        other => unreachable!("not a chat line: {other:?}"),
    };
    encode(&command)
}

/// The native Help Request command for a [`WrathRoute::NativeHelp`] route; the ticket window's
/// create and update arrive already native (`ui_gm_ticket`).
fn help_command(command: ClientCommand) -> wrathplus_protocol::wrath_help::Command {
    use wrathplus_protocol::wrath_help::Command as H;
    match command {
        ClientCommand::NativeHelp(command) => command,
        ClientCommand::GmTicketGet => H::GetTicket,
        ClientCommand::GmTicketDelete => H::DeleteTicket,
        ClientCommand::GmTicketSystemStatus => H::SystemStatus,
        other => unreachable!("not a help-request route: {other:?}"),
    }
}

/// The native session-metadata command for a [`WrathRoute::NativeSessionMeta`] route.
fn session_meta_command(command: ClientCommand) -> wrathplus_protocol::wrath_session_meta::Command {
    use wrathplus_protocol::wrath_session_meta::Command as M;
    match command {
        ClientCommand::NativeSessionMeta(command) => command,
        ClientCommand::TutorialFlag { id } => M::TutorialFlag { id },
        ClientCommand::TutorialClear => M::TutorialClear,
        ClientCommand::TutorialReset => M::TutorialReset,
        // `/played`: AC echoes the byte, and 1 asks for the chat line.
        ClientCommand::PlayedTime => M::PlayedTime { trigger: 1 },
        ClientCommand::NextCinematicCamera => M::NextCinematicCamera,
        ClientCommand::CompleteCinematic => M::CompleteCinematic,
        // A teleporting trigger's transfer is the movement authority's.
        ClientCommand::AreaTrigger { trigger_id } => M::AreaTrigger { trigger_id },
        ClientCommand::SetActionBarToggles { toggles } => M::ActionBarToggles { toggles },
        other => unreachable!("not a session-metadata route: {other:?}"),
    }
}

/// The native death or summon command for a [`WrathRoute::NativeDeath`] route.
fn death_command(command: ClientCommand) -> wrathplus_protocol::wrath_death::Command {
    use wrathplus_protocol::wrath_death::Command as D;
    match command {
        ClientCommand::NativeDeath(command) => command,
        ClientCommand::RepopRequest => D::Repop,
        ClientCommand::CorpseQuery => D::CorpseQuery,
        ClientCommand::ReclaimCorpse { corpse } => D::ReclaimCorpse { corpse },
        // The shared command is the accept; a decline sends nothing and lets the summon lapse.
        ClientCommand::SummonResponse { summoner } => D::SummonResponse {
            summoner,
            agree: true,
        },
        ClientCommand::SelfRes => D::SelfRes,
        other => unreachable!("not a death route: {other:?}"),
    }
}

/// The native charter command for a [`WrathRoute::NativePetition`] command. The registrar sells
/// one guild charter, so a buy names row 1 (AC reads the index for arena-team charters only).
fn petition_command(command: ClientCommand) -> wrathplus_protocol::wrath_petition::Command {
    use wrathplus_protocol::wrath_petition::Command as P;
    match command {
        ClientCommand::PetitionShowList { npc } => P::ShowList { npc },
        ClientCommand::PetitionBuy { npc, name, index } => P::Buy { npc, name, index },
        ClientCommand::PetitionShowSignatures { item } => P::ShowSignatures { item },
        ClientCommand::PetitionSign { item, byte } => P::Sign { item, byte },
        ClientCommand::OfferPetition { item, player } => P::Offer { item, player },
        ClientCommand::TurnInPetition { item, banner } => P::TurnIn { item, banner },
        ClientCommand::PetitionQuery { petition_id, item } => P::Query { petition_id, item },
        ClientCommand::PetitionRename { item, name } => P::Rename { item, name },
        ClientCommand::PetitionDecline { item } => P::Decline { item },
        other => unreachable!("not a petition route: {other:?}"),
    }
}

/// The native contact or duel body for a [`WrathRoute::NativeContacts`] command, through the
/// existing `wrath_social`/`wrath_duel` encoders.
fn contacts_packet(command: ClientCommand) -> std::io::Result<(u32, Vec<u8>)> {
    use wrathplus_protocol::wrath_duel::Command as D;
    use wrathplus_protocol::wrath_social::Command as S;
    let social = |command: S| Ok((command.opcode(), command.encode()?));
    let duel = |command: D| Ok((command.opcode(), command.encode()));
    match command {
        // Friends, ignores and mutes: AC lists the contacts carrying any asked flag.
        ClientCommand::FriendListRequest => social(S::ContactList { flags: 0x07 }),
        ClientCommand::AddFriend { name } => social(S::AddFriend {
            name: name.into_bytes(),
            note: Vec::new(),
        }),
        ClientCommand::DelFriend { guid } => social(S::DeleteFriend { guid }),
        ClientCommand::AddIgnore { name } => social(S::AddIgnore {
            name: name.into_bytes(),
        }),
        ClientCommand::DelIgnore { guid } => social(S::DeleteIgnore { guid }),
        ClientCommand::SetContactNotes { guid, note } => social(S::SetContactNotes {
            guid,
            note: note.into_bytes(),
        }),
        ClientCommand::DuelAccepted { arbiter } => duel(D::Accepted { arbiter }),
        ClientCommand::DuelCancelled { arbiter } => duel(D::Cancelled { arbiter }),
        other => unreachable!("not a contacts route: {other:?}"),
    }
}

/// The native body for a [`WrathRoute::NativeBattleground`] command. The shared commands below keep
/// their 3.3.5 shapes (a GUID or nothing); list, join and port need this build's bg type and are
/// native-only.
fn battleground_packet(command: ClientCommand) -> (u32, Vec<u8>) {
    use wrathplus_protocol::wrath_battleground::{encode, encode_leave};
    let guid = |opcode, guid: u64| (opcode, guid.to_le_bytes().to_vec());
    match command {
        ClientCommand::NativeBattleground(command) => encode(&command),
        // The world-enter reset's empty `CMSG_BATTLEFIELD_STATUS` (`0x54ca50`).
        ClientCommand::BattlefieldStatusRequest => {
            encode(&wrathplus_protocol::wrath_battleground::Command::Status)
        }
        ClientCommand::BattlemasterHello { npc } => guid(0x02d7, npc),
        ClientCommand::AreaSpiritHealerQuery { healer } => guid(0x02e2, healer),
        ClientCommand::AreaSpiritHealerQueue { healer } => guid(0x02e3, healer),
        ClientCommand::SpiritHealerActivate { npc } => guid(0x021c, npc),
        ClientCommand::InspectHonorStats { target } => guid(0x02d6, target),
        // An empty body toggles; AC also takes one byte setting the flag outright.
        ClientCommand::TogglePvp => (0x0253, Vec::new()),
        ClientCommand::RequestBattlefieldScoreData => (0x02e0, Vec::new()),
        ClientCommand::RequestBattlefieldPositions => (0x02e9, Vec::new()),
        // AC reads none of the body and leaves the current battleground.
        ClientCommand::LeaveBattlefield => encode_leave(),
        other => unreachable!("not a battleground route: {other:?}"),
    }
}

/// The native channel, emote and ignore command for a [`WrathRoute::NativeChannel`] route.
fn channel_command(command: ClientCommand) -> wrathplus_protocol::wrath_chat_notices::Command {
    use wrathplus_protocol::wrath_chat_notices::Command as C;
    let pair = |opcode, name, other| C::NamedPair {
        opcode,
        name,
        other,
    };
    match command {
        ClientCommand::JoinChannel {
            name,
            password,
            channel_id,
        } => C::Join {
            channel_id,
            name,
            password,
        },
        ClientCommand::LeaveChannel { name } => C::Leave { name },
        ClientCommand::ChannelList { name } => C::Named {
            opcode: 0x009a,
            name,
        },
        ClientCommand::ClearChannelWatch => C::ClearWatch,
        ClientCommand::DeclineChannelInvite { name } => C::Named {
            opcode: 0x0410,
            name,
        },
        ClientCommand::ChannelOwner { name } => C::Named {
            opcode: 0x009e,
            name,
        },
        ClientCommand::ChannelAnnouncements { name } => C::Named {
            opcode: 0x00a7,
            name,
        },
        ClientCommand::ChannelModerate { name } => C::Named {
            opcode: 0x00a8,
            name,
        },
        ClientCommand::ChannelPassword { name, password } => pair(0x009c, name, password),
        ClientCommand::ChannelSetOwner { name, player } => pair(0x009d, name, player),
        ClientCommand::ChannelModerator { name, player } => pair(0x009f, name, player),
        ClientCommand::ChannelUnmoderator { name, player } => pair(0x00a0, name, player),
        ClientCommand::ChannelMute { name, player } => pair(0x00a1, name, player),
        ClientCommand::ChannelUnmute { name, player } => pair(0x00a2, name, player),
        ClientCommand::ChannelInvite { name, player } => pair(0x00a3, name, player),
        ClientCommand::ChannelKick { name, player } => pair(0x00a4, name, player),
        ClientCommand::ChannelBan { name, player } => pair(0x00a5, name, player),
        ClientCommand::ChannelUnban { name, player } => pair(0x00a6, name, player),
        // AC echoes the emote number and uses nothing else of it.
        ClientCommand::TextEmote { text_id, target } => C::TextEmote {
            text_emote: text_id,
            emote_num: 0,
            target,
        },
        ClientCommand::ChatIgnored { guid } => C::ChatIgnored { guid },
        other => unreachable!("not a channel route: {other:?}"),
    }
}

/// The native party command for a [`WrathRoute::NativeParty`] route.
fn party_command(command: ClientCommand) -> wrathplus_protocol::wrath_party::Command {
    use wrathplus_protocol::wrath_party::Command as P;
    match command {
        ClientCommand::NativeParty(command) => command,
        ClientCommand::GroupUninvite { name } => P::Uninvite { name },
        ClientCommand::GroupSetLeader { guid } => P::SetLeader { guid },
        ClientCommand::MinimapPing { x, y } => P::MinimapPing { x, y },
        ClientCommand::RandomRoll { min, max } => P::RandomRoll { min, max },
        other => unreachable!("not a party route: {other:?}"),
    }
}

/// The native body for a [`WrathRoute::NativeQuest`] command.
fn quest_packet(command: ClientCommand) -> std::io::Result<(u32, Vec<u8>)> {
    use wrathplus_protocol::wrath_quest::{encode, encode_push_result};
    match command {
        ClientCommand::NativeQuest(command) => encode(&command),
        ClientCommand::QuestPushResult { sharer, quest, msg } => {
            Ok(encode_push_result(sharer, quest, msg.0))
        }
        other => unreachable!("not a quest route: {other:?}"),
    }
}

/// The native trade, auction and master-loot body for a [`WrathRoute::NativeExchange`] command.
pub(super) fn exchange_packet(command: ClientCommand) -> (u32, Vec<u8>) {
    use wrathplus_protocol::wrath_exchange::{encode, Command as X};
    use wrathplus_protocol::wrath_trade::Command as T;
    let trade = |command: T| (command.opcode(), command.encode());
    match command {
        ClientCommand::NativeExchange(command) => encode(&command),
        ClientCommand::AuctionHello { auctioneer } => encode(&X::AuctionHello { auctioneer }),
        ClientCommand::AuctionListItems {
            auctioneer,
            list_from,
            searched_name,
            level_min,
            level_max,
            slot_id,
            main_category,
            sub_category,
            quality,
            usable,
        } => encode(&X::AuctionListItems {
            auctioneer,
            list_from,
            searched_name,
            level_min,
            level_max,
            slot_id,
            main_category,
            sub_category,
            quality,
            usable,
        }),
        ClientCommand::AuctionListOwnerItems {
            auctioneer,
            list_from,
        } => encode(&X::AuctionListOwner {
            auctioneer,
            list_from,
        }),
        ClientCommand::AuctionListBidderItems {
            auctioneer,
            list_from,
            auction_ids,
        } => encode(&X::AuctionListBidder {
            auctioneer,
            list_from,
            auction_ids,
        }),
        ClientCommand::AuctionSellItem {
            auctioneer,
            items,
            bid,
            buyout,
            etime_minutes,
        } => encode(&X::AuctionSell {
            auctioneer,
            items,
            bid,
            buyout,
            etime_minutes,
        }),
        ClientCommand::AuctionPlaceBid {
            auctioneer,
            auction_id,
            price,
        } => encode(&X::AuctionPlaceBid {
            auctioneer,
            auction_id,
            price,
        }),
        ClientCommand::AuctionRemoveItem {
            auctioneer,
            auction_id,
        } => encode(&X::AuctionRemove {
            auctioneer,
            auction_id,
        }),
        ClientCommand::LootMasterGive { guid, slot, target } => encode(&X::LootMasterGive {
            loot: guid,
            slot,
            target,
        }),
        ClientCommand::InitiateTrade { target } => trade(T::Initiate { target }),
        ClientCommand::BeginTrade => trade(T::Begin),
        ClientCommand::BusyTrade => trade(T::Busy),
        ClientCommand::IgnoreTrade => trade(T::Ignore),
        // AC skips the word; the native client sends 1.
        ClientCommand::AcceptTrade => trade(T::Accept { raw: 1 }),
        ClientCommand::UnacceptTrade => trade(T::Unaccept),
        ClientCommand::CancelTrade => trade(T::Cancel),
        ClientCommand::SetTradeGold { copper } => trade(T::SetGold { gold: copper }),
        ClientCommand::SetTradeItem {
            trade_slot,
            bag,
            slot,
        } => trade(T::SetItem {
            trade_slot,
            bag,
            slot,
        }),
        ClientCommand::ClearTradeItem { trade_slot } => trade(T::ClearItem { trade_slot }),
        other => unreachable!("not an exchange route: {other:?}"),
    }
}

/// The native item/inventory body for a [`WrathRoute::NativeInventory`] command.
fn inventory_command(command: ClientCommand) -> wrathplus_protocol::wrath_inventory::Command {
    use wrathplus_protocol::wrath_inventory::Command as I;
    match command {
        ClientCommand::NativeInventory(command) => command,
        ClientCommand::OpenItem { bag_index, slot } => I::OpenItem {
            bag: bag_index,
            slot,
        },
        ClientCommand::WrapItem {
            gift_bag,
            gift_slot,
            item_bag,
            item_slot,
        } => I::WrapItem {
            gift_bag,
            gift_slot,
            item_bag,
            item_slot,
        },
        ClientCommand::AutoEquipItem { bag_index, slot } => I::AutoEquipItem {
            bag: bag_index,
            slot,
        },
        ClientCommand::SetAmmo { entry } => I::SetAmmo { entry },
        ClientCommand::SwapInvItem { src_slot, dst_slot } => I::SwapInvItem { src_slot, dst_slot },
        ClientCommand::SwapItem {
            dst_bag,
            dst_slot,
            src_bag,
            src_slot,
        } => I::SwapItem {
            dst_bag,
            dst_slot,
            src_bag,
            src_slot,
        },
        ClientCommand::AutoStoreBagItem {
            src_bag,
            src_slot,
            dst_bag,
        } => I::AutoStoreBagItem {
            src_bag,
            src_slot,
            dst_bag,
        },
        ClientCommand::SplitItem {
            src_bag,
            src_slot,
            dst_bag,
            dst_slot,
            count,
        } => I::SplitItem {
            src_bag,
            src_slot,
            dst_bag,
            dst_slot,
            count: count.into(),
        },
        ClientCommand::DestroyItem {
            bag_index,
            slot,
            count,
        } => I::DestroyItem {
            bag: bag_index,
            slot,
            count,
        },
        ClientCommand::BankerActivate { guid } => I::BankerActivate { banker: guid },
        ClientCommand::BuyBankSlot { guid } => I::BuyBankSlot { banker: guid },
        ClientCommand::AutoBankItem { bag, slot } => I::AutoBankItem { bag, slot },
        ClientCommand::AutoStoreBankItem { bag, slot } => I::AutoStoreBankItem { bag, slot },
        ClientCommand::PageTextQuery { page_id, guid } => I::PageTextQuery { page_id, guid },
        other => unreachable!("not an inventory route: {other:?}"),
    }
}

pub(super) fn session_packet(command: ClientCommand) -> (u32, Vec<u8>) {
    use wrathplus_protocol::wrath_group as g;
    use wrathplus_protocol::wrath_npc as n;
    use wrathplus_protocol::wrath_session::{self as s, CastTarget};
    use wrathplus_protocol::wrath_trainer as t;
    use wrathplus_protocol::wrath_vendor as v;
    // The client's cast counter, echoed by the server's cast results.
    static CAST_COUNT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    let cast = |spell_id: u32, target: CastTarget| {
        let count = CAST_COUNT
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1);
        (
            s::CMSG_CAST_SPELL,
            s::encode_cast_spell(count, spell_id, &target),
        )
    };
    match command {
        ClientCommand::NameQuery { guid } => (s::CMSG_NAME_QUERY, s::encode_name_query(guid)),
        ClientCommand::CreatureQuery { entry, guid } => (
            s::CMSG_CREATURE_QUERY,
            s::encode_template_query(entry, guid),
        ),
        ClientCommand::GameObjectQuery { entry, guid } => (
            s::CMSG_GAMEOBJECT_QUERY,
            s::encode_template_query(entry, guid),
        ),
        ClientCommand::SetActionButton { button, packed } => (
            s::CMSG_SET_ACTION_BUTTON,
            s::encode_set_action_button(button, packed),
        ),
        ClientCommand::CastSpell { spell_id, target } => {
            cast(spell_id, target.map_or(CastTarget::None, CastTarget::Unit))
        }
        ClientCommand::CastSpellAtDest { spell_id, dest } => cast(spell_id, CastTarget::Dest(dest)),
        ClientCommand::CastSpellCorpse {
            spell_id,
            target,
            corpse_guid,
        } => cast(
            spell_id,
            match target {
                wrathplus_protocol::types::CorpseTarget::Ally => {
                    CastTarget::CorpseAlly(corpse_guid)
                }
                wrathplus_protocol::types::CorpseTarget::Enemy => {
                    CastTarget::CorpseEnemy(corpse_guid)
                }
            },
        ),
        // One counter with `CMSG_CAST_SPELL`: the server echoes it in the cast results.
        ClientCommand::NativePetCastSpell {
            caster,
            spell_id,
            target,
        } => {
            let count = CAST_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(1);
            (
                s::CMSG_PET_CAST_SPELL,
                s::encode_pet_cast_spell(caster, count, spell_id, &target),
            )
        }
        ClientCommand::CastSpellAtSource { spell_id, src } => {
            cast(spell_id, CastTarget::Source(src))
        }
        ClientCommand::CastSpellGameObject { spell_id, go_guid } => {
            cast(spell_id, CastTarget::GameObject(go_guid))
        }
        ClientCommand::CastSpellItem {
            spell_id,
            item_guid,
        } => cast(spell_id, CastTarget::Item(item_guid)),
        // One cast counter for both opcodes: the server echoes it in either's cast results.
        ClientCommand::UseItem {
            bag_index,
            slot,
            spell_id,
            item_guid,
            glyph_index,
            target,
            ..
        } => {
            let count = CAST_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(1);
            wrathplus_protocol::wrath_inventory::encode_use_item(
                count,
                bag_index,
                slot,
                spell_id,
                item_guid,
                glyph_index,
                &target,
            )
        }
        ClientCommand::CancelCast { spell_id } => {
            (s::CMSG_CANCEL_CAST, s::encode_cancel_cast(spell_id))
        }
        ClientCommand::CancelAura { spell_id } => {
            (s::CMSG_CANCEL_AURA, s::encode_cancel_aura(spell_id))
        }
        ClientCommand::GossipHello { guid } => (n::CMSG_GOSSIP_HELLO, guid.to_le_bytes().to_vec()),
        ClientCommand::GossipSelectOption { guid, option } => (
            n::CMSG_GOSSIP_SELECT_OPTION,
            n::encode_gossip_select_option(guid, option),
        ),
        ClientCommand::NpcTextQuery { text_id, guid } => (
            n::CMSG_NPC_TEXT_QUERY,
            n::encode_npc_text_query(text_id, guid),
        ),
        ClientCommand::QuestgiverStatusQuery { npc } => {
            (n::CMSG_QUESTGIVER_STATUS_QUERY, npc.to_le_bytes().to_vec())
        }
        ClientCommand::QuestgiverHello { npc } => {
            (n::CMSG_QUESTGIVER_HELLO, npc.to_le_bytes().to_vec())
        }
        ClientCommand::QuestgiverQuery { npc, quest } => (
            n::CMSG_QUESTGIVER_QUERY_QUEST,
            n::encode_questgiver_query_quest(npc, quest),
        ),
        ClientCommand::QuestgiverAccept { npc, quest } => (
            n::CMSG_QUESTGIVER_ACCEPT_QUEST,
            n::encode_questgiver_accept_quest(npc, quest),
        ),
        ClientCommand::QuestgiverComplete { npc, quest } => (
            n::CMSG_QUESTGIVER_COMPLETE_QUEST,
            n::encode_guid_quest(npc, quest),
        ),
        ClientCommand::QuestgiverRequestReward { npc, quest } => (
            n::CMSG_QUESTGIVER_REQUEST_REWARD,
            n::encode_guid_quest(npc, quest),
        ),
        ClientCommand::QuestgiverChooseReward { npc, quest, choice } => (
            n::CMSG_QUESTGIVER_CHOOSE_REWARD,
            n::encode_questgiver_choose_reward(npc, quest, choice),
        ),
        ClientCommand::QuestQuery { quest } => (n::CMSG_QUEST_QUERY, quest.to_le_bytes().to_vec()),
        ClientCommand::QuestlogRemove { slot } => (n::CMSG_QUESTLOG_REMOVE_QUEST, vec![slot]),
        ClientCommand::PushQuestToParty { quest } => {
            (n::CMSG_PUSHQUESTTOPARTY, quest.to_le_bytes().to_vec())
        }
        ClientCommand::QuestConfirmAccept { quest } => {
            (n::CMSG_QUEST_CONFIRM_ACCEPT, quest.to_le_bytes().to_vec())
        }
        ClientCommand::ListInventory { guid } => {
            (v::CMSG_LIST_INVENTORY, guid.to_le_bytes().to_vec())
        }
        ClientCommand::BuyItem {
            vendor,
            entry,
            count,
        } => (v::CMSG_BUY_ITEM, v::encode_buy_item(vendor, entry, count)),
        ClientCommand::BuyItemInSlot {
            vendor,
            entry,
            bag_guid,
            bag_slot,
            count,
        } => (
            v::CMSG_BUY_ITEM_IN_SLOT,
            v::encode_buy_item_in_slot(vendor, entry, bag_guid, bag_slot, count),
        ),
        ClientCommand::SellItem {
            vendor,
            item_guid,
            count,
        } => (
            v::CMSG_SELL_ITEM,
            v::encode_sell_item(vendor, item_guid, count),
        ),
        ClientCommand::BuybackItem { vendor, slot } => {
            (v::CMSG_BUYBACK_ITEM, v::encode_buyback_item(vendor, slot))
        }
        ClientCommand::RepairItem { vendor, item_guid } => (
            v::CMSG_REPAIR_ITEM,
            v::encode_repair_item(vendor, item_guid, false),
        ),
        ClientCommand::TaxiNodeStatusQuery { guid } => (
            wrathplus_protocol::wrath_taxi::CMSG_TAXINODE_STATUS_QUERY,
            wrathplus_protocol::wrath_taxi::encode_query(guid),
        ),
        ClientCommand::TaxiQueryNodes { guid } => (
            wrathplus_protocol::wrath_taxi::CMSG_TAXIQUERYAVAILABLENODES,
            wrathplus_protocol::wrath_taxi::encode_query(guid),
        ),
        ClientCommand::ActivateTaxi {
            guid,
            source_node,
            dest_node,
        } => (
            wrathplus_protocol::wrath_taxi::CMSG_ACTIVATETAXI,
            wrathplus_protocol::wrath_taxi::encode_activate(guid, source_node, dest_node),
        ),
        ClientCommand::ActivateTaxiExpress { guid, nodes } => (
            wrathplus_protocol::wrath_taxi::CMSG_ACTIVATETAXIEXPRESS,
            wrathplus_protocol::wrath_taxi::encode_express(guid, &nodes),
        ),
        ClientCommand::GroupInvite { name } => (
            g::CMSG_GROUP_INVITE,
            g::encode_invite(name.split('\0').next().unwrap_or("")).expect("NUL-free name"),
        ),
        ClientCommand::GroupAccept => (g::CMSG_GROUP_ACCEPT, g::encode_accept()),
        ClientCommand::GroupDecline => (g::CMSG_GROUP_DECLINE, g::encode_decline()),
        ClientCommand::PartyAssignment {
            assignment,
            apply,
            guid,
        } => (
            g::MSG_PARTY_ASSIGNMENT,
            g::encode_assignment(assignment, apply, guid),
        ),
        ClientCommand::GroupLeave => (g::CMSG_GROUP_DISBAND, g::encode_leave()),
        ClientCommand::GroupRaidConvert => (g::commands::CMSG_GROUP_RAID_CONVERT, Vec::new()),
        ClientCommand::LootMethod {
            method,
            master,
            threshold,
        } => (
            g::commands::CMSG_LOOT_METHOD,
            g::commands::loot_method(method, master, threshold),
        ),
        ClientCommand::SetRaidTarget { icon, guid } => (
            g::commands::MSG_RAID_TARGET_UPDATE,
            g::commands::raid_target(icon, guid),
        ),
        ClientCommand::GroupChangeSubGroup { name, group } => (
            g::commands::CMSG_GROUP_CHANGE_SUB_GROUP,
            g::commands::change_subgroup(name.split('\0').next().unwrap_or(""), group)
                .expect("NUL-free name"),
        ),
        ClientCommand::GroupSwapSubGroup { name, other } => (
            g::commands::CMSG_GROUP_SWAP_SUB_GROUP,
            g::commands::swap_subgroup(
                name.split('\0').next().unwrap_or(""),
                other.split('\0').next().unwrap_or(""),
            )
            .expect("NUL-free names"),
        ),
        ClientCommand::GroupAssistantLeader { guid, grant } => (
            g::commands::CMSG_GROUP_ASSISTANT_LEADER,
            g::commands::assistant(guid, grant),
        ),
        ClientCommand::ReadyCheckStart => (g::commands::MSG_RAID_READY_CHECK, Vec::new()),
        ClientCommand::ReadyCheckAnswer { ready } => {
            (g::commands::MSG_RAID_READY_CHECK, vec![u8::from(ready)])
        }
        ClientCommand::RequestPartyMemberStats { guid } => (
            g::CMSG_REQUEST_PARTY_MEMBER_STATS,
            g::encode_request_stats(guid),
        ),
        ClientCommand::BinderActivate { binder } => (
            wrathplus_protocol::wrath_bind::CMSG_BINDER_ACTIVATE,
            wrathplus_protocol::wrath_bind::encode_activate(binder),
        ),
        ClientCommand::TrainerList { trainer } => (t::CMSG_TRAINER_LIST, t::encode_list(trainer)),
        ClientCommand::TrainerBuySpell { trainer, spell_id } => {
            (t::CMSG_TRAINER_BUY_SPELL, t::encode_buy(trainer, spell_id))
        }
        ClientCommand::GetMailList { mailbox } => (
            wrathplus_protocol::wrath_mail::CMSG_GET_MAIL_LIST,
            mailbox.to_le_bytes().to_vec(),
        ),
        ClientCommand::QueryNextMailTime => (
            wrathplus_protocol::wrath_mail::MSG_QUERY_NEXT_MAIL_TIME as u32,
            wrathplus_protocol::wrath_mail::encode_query_next_mail_time(),
        ),
        // AzerothCore GroupHandler.cpp::HandleLootRoll: full GUID, u32 slot, u8 vote.
        ClientCommand::LootRoll {
            looted_target,
            item_slot,
            roll_type,
        } => (
            wrathplus_protocol::wrath_roll::CMSG_LOOT_ROLL,
            wrathplus_protocol::wrath_roll::encode_vote(looted_target, item_slot, roll_type)
                .expect("validated native roll vote"),
        ),
        ClientCommand::ListStabledPets { npc } => {
            stable_packet(wrathplus_protocol::wrath_stable::Command::List { npc })
        }
        ClientCommand::StablePet { npc } => {
            stable_packet(wrathplus_protocol::wrath_stable::Command::Stable { npc })
        }
        ClientCommand::UnstablePet { npc, pet_number } => {
            stable_packet(wrathplus_protocol::wrath_stable::Command::Unstable { npc, pet_number })
        }
        ClientCommand::StableSwapPet { npc, pet_number } => {
            stable_packet(wrathplus_protocol::wrath_stable::Command::Swap { npc, pet_number })
        }
        ClientCommand::BuyStableSlot { npc } => {
            stable_packet(wrathplus_protocol::wrath_stable::Command::BuySlot { npc })
        }
        ClientCommand::GuildBankQuery(query) => (query.opcode(), query.encode()),
        ClientCommand::RequestRaidInfo => (
            wrathplus_protocol::wrath_instance::CMSG_REQUEST_RAID_INFO,
            Vec::new(),
        ),
        ClientCommand::LfgRequestPlayerLockInfo => {
            let command = wrathplus_protocol::wrath_lfg::Command::RequestPlayerLockInfo;
            (command.opcode(), command.encode())
        }
        ClientCommand::LfgRequestPartyLockInfo => {
            let command = wrathplus_protocol::wrath_lfg::Command::RequestPartyLockInfo;
            (command.opcode(), command.encode())
        }
        ClientCommand::SetChannelWatch { name } => (
            wrathplus_protocol::wrath_channel::CMSG_SET_CHANNEL_WATCH,
            wrathplus_protocol::wrath_channel::encode_channel_name(&name),
        ),
        ClientCommand::ChannelDisplayList { name } => (
            wrathplus_protocol::wrath_channel::CMSG_CHANNEL_DISPLAY_LIST,
            wrathplus_protocol::wrath_channel::encode_channel_name(&name),
        ),
        other => unreachable!("not a Wrath session command: {other:?}"),
    }
}

fn send_loot(active: &mut WrathWriterSwap, command: ClientCommand) -> Result<()> {
    let (opcode, body) = loot_packet(command)?;
    let authority = active.authority.clone();
    authority
        .with_player_action(|| active.writer.send_raw(opcode, &body))
        .unwrap_or_else(|| {
            Err(anyhow!(
                "Wrath loot action lost session/control authority before send"
            ))
        })
}

fn expire_pre_self_orders(
    authority: &MovementAuthorityHandle,
    epoch: u64,
    pending: usize,
    control: &Sender<WrathWriterControl>,
) {
    if pending == 0 {
        return;
    }
    authority.revoke();
    let _ = control.send(WrathWriterControl::Fatal {
        epoch,
        reason: format!(
            "Wrath self pose did not arrive or teleport snap remained pending within {} ms with {pending} pending movement order(s)",
            PRE_SELF_ORDER_TIMEOUT.as_millis()
        ),
    });
}

/// The vehicle seat a living movement names (`ON_TRANSPORT` with `seat >= 0`).
fn movement_seat(movement: &wrathplus_protocol::wrath_gameplay::MovementInfo) -> Option<(u64, i8)> {
    movement
        .transport
        .as_ref()
        .and_then(|t| wrathplus_protocol::wrath_vehicle::vehicle_seat(t.guid, t.seat))
}

/// AzerothCore's `MOVEMENTFLAG_MASK_MOVING`: what a rooted mover may not report.
const ROOTED_MOVING: u32 = wrath_ground::MOVE_FORWARD
    | wrath_ground::MOVE_BACKWARD
    | wrath_ground::MOVE_STRAFE_LEFT
    | wrath_ground::MOVE_STRAFE_RIGHT
    | wrath_ground::MOVE_PITCH_UP
    | wrath_ground::MOVE_PITCH_DOWN
    | wrath_ground::MOVE_FALLING
    | wrath_ground::MOVE_FALLING_FAR
    | wrath_ground::MOVE_ASCENDING
    | wrath_ground::MOVE_DESCENDING;

/// A create block's living pose as the authority keeps it: ROOT stripped (the root orders own it),
/// and a state the writer does not report (a creature's flight, spline elevation) stood at its
/// world position, so an ACK from it cannot rewind the unit.
fn living_pose(movement: &wrathplus_protocol::wrath_gameplay::MovementInfo) -> OnFootPose {
    let mut movement = movement.clone();
    movement.flags &= !wrath_authority::MOVEMENT_FLAG_ROOT;
    OnFootPose::from_movement(&movement).unwrap_or_else(|error| {
        bevy::log::debug!(
            "net(wrath): living pose not reportable ({error}); standing at its position"
        );
        OnFootPose {
            flags: 0,
            position: [
                movement.position.x,
                movement.position.y,
                movement.position.z,
            ],
            orientation: movement.position.orientation,
            fall_time: 0,
            fall: None,
            pitch: None,
            transport: None,
        }
    })
}

fn accepts_epoch(command_epoch: u64, active_epoch: u64) -> bool {
    command_epoch == active_epoch
}

fn send_ground_movement(active: &mut WrathWriterSwap, command: ClientCommand) -> Result<()> {
    let ClientCommand::Move {
        kind,
        flags,
        pos,
        orientation,
        pitch,
        fall_time,
        jump,
        transport,
    } = command
    else {
        unreachable!()
    };
    // The deck-local tail rides with Wrath ON_TRANSPORT (the controller's flags are already mapped).
    let timestamp = active.clock.now_ms();
    let transport = transport.map(|t| wrath_ground::TransportTail {
        guid: t.guid,
        position: [t.pos.x, t.pos.y, t.pos.z],
        orientation: t.orientation,
        time: timestamp,
    });
    // Pitch rides only with SWIMMING or FLYING (AzerothCore `ReadMovementInfo`); otherwise level.
    let pitched = flags & (wrath_ground::MOVE_SWIMMING | wrath_ground::MOVE_FLYING) != 0;
    if !pitched && pitch != 0.0 {
        bevy::log::warn!("net(wrath): on-foot movement with a pitch outside water or air, dropped");
        return Ok(());
    }
    let pitch = pitched.then_some(pitch);
    let fall = jump.map(|jump| wrath_ground::FallTail {
        vertical_speed: jump.zspeed,
        sin_angle: jump.sin_angle,
        cos_angle: jump.cos_angle,
        horizontal_speed: jump.xy_speed,
    });
    let pose = OnFootPose {
        flags,
        position: pos,
        orientation,
        fall_time,
        fall,
        pitch,
        transport,
    };
    let authority = active.authority.clone();
    authority
        .with_on_foot_send(active.active_mover, pose, |mover, pose| {
            let opcode = u32::from(kind.opcode());
            match encoded_or_dropped("movement", pose.encode(opcode, mover, timestamp)) {
                Some(body) => active.writer.send_raw(opcode, &body),
                None => Ok(()),
            }
        })
        .unwrap_or(Ok(()))
}

/// An outbound body, or `None` with a warning when the client's own pose would not encode: a
/// bad pose is dropped like an invalid chat line, never a session error.
fn encoded_or_dropped<E: std::fmt::Display>(
    what: &str,
    body: Result<Vec<u8>, E>,
) -> Option<Vec<u8>> {
    match body {
        Ok(body) => Some(body),
        Err(error) => {
            bevy::log::warn!("net(wrath): {what} not encodable, dropped: {error}");
            None
        }
    }
}

/// `CMSG_MOVE_NOT_ACTIVE_MOVER`: our own body's parting pose as we hand control to a vehicle or a
/// possessed unit (`Wow.exe` `0x729010` sends it only for the player, before `SET_ACTIVE_MOVER`).
/// AzerothCore reads `packed guid + MovementInfo` and keeps it only while we are still its mover.
fn send_not_active_mover(active: &mut WrathWriterSwap, command: ClientCommand) -> Result<()> {
    let ClientCommand::NotActiveMover {
        guid,
        pos,
        orientation,
        ..
    } = command
    else {
        unreachable!()
    };
    if guid != active.authority.self_guid {
        bevy::log::warn!("net(wrath): NOT_ACTIVE_MOVER for {guid:#018x} is not our body; withheld");
        return Ok(());
    }
    // The parting pose is a standing one: the controller parks the body first
    // (`player::wire_in::yield_own_body`), so no direction bit or arc rides it.
    let pose = OnFootPose {
        flags: 0,
        position: pos,
        orientation,
        fall_time: 0,
        fall: None,
        pitch: None,
        transport: None,
    };
    let body = wrath_authority::encode_mover_pose(guid, pose, active.clock.now_ms());
    let Some(body) = encoded_or_dropped("NOT_ACTIVE_MOVER", body) else {
        return Ok(());
    };
    active
        .writer
        .send_raw(wrath_authority::CMSG_MOVE_NOT_ACTIVE_MOVER, &body)
}

fn send_pending_authority(
    active: &mut WrathWriterSwap,
    pending: PendingAuthorityOrder,
) -> Result<UnsetAckResult> {
    match pending {
        PendingAuthorityOrder::UnsetCanFly(order) => send_unset_can_fly(active, order),
        PendingAuthorityOrder::ForceRoot { order, rooted } => {
            send_force_root(active, order, rooted)
        }
        PendingAuthorityOrder::Mover(order) => send_mover_ack(active, order),
    }
}

fn queue_or_send_authority(
    active: &mut WrathWriterSwap,
    pending: PendingAuthorityOrder,
    queue: &mut std::collections::VecDeque<PendingAuthorityOrder>,
    timeout: &mut Receiver<Instant>,
) -> Result<()> {
    // Once a pose or teleport snap is waiting, preserve receipt order without probing later orders.
    if !queue.is_empty() {
        if queue.len() == MAX_PENDING_PRE_SELF_ORDERS {
            bail!("too many Wrath movement orders before self pose");
        }
        queue.push_back(pending);
        return Ok(());
    }
    match send_pending_authority(active, pending)? {
        UnsetAckResult::PendingSelfPose | UnsetAckResult::PendingTeleport => {
            *timeout = crossbeam_channel::after(PRE_SELF_ORDER_TIMEOUT);
            queue.push_back(pending);
        }
        UnsetAckResult::Sent | UnsetAckResult::Stale => {}
    }
    Ok(())
}

/// Retry only the queued head after a self pose or controller teleport ACK makes progress.
fn drain_authority_queue(
    queue: &mut std::collections::VecDeque<PendingAuthorityOrder>,
    timeout: &mut Receiver<Instant>,
    mut send: impl FnMut(PendingAuthorityOrder) -> Result<UnsetAckResult>,
) -> Result<()> {
    while let Some(&pending) = queue.front() {
        match send(pending)? {
            UnsetAckResult::Sent | UnsetAckResult::Stale => {
                queue.pop_front();
            }
            UnsetAckResult::PendingSelfPose | UnsetAckResult::PendingTeleport => break,
        }
    }
    if queue.is_empty() {
        *timeout = crossbeam_channel::never();
    }
    Ok(())
}

fn send_unset_can_fly(
    active: &mut WrathWriterSwap,
    order: UnsetCanFlyOrder,
) -> Result<UnsetAckResult> {
    let authority = active.authority.clone();
    let timestamp = active.clock.now_ms();
    authority.send_unset_ack(order, timestamp, |body| {
        active
            .writer
            .send_raw(wrath_authority::CMSG_MOVE_SET_CAN_FLY_ACK, body)
    })
}

fn send_force_root(
    active: &mut WrathWriterSwap,
    order: ForceRootOrder,
    rooted: bool,
) -> Result<UnsetAckResult> {
    let authority = active.authority.clone();
    let timestamp = active.clock.now_ms();
    authority.send_root_ack(order, rooted, timestamp, |opcode, body| {
        active.writer.send_raw(opcode, body)
    })
}

fn send_mover_ack(active: &mut WrathWriterSwap, order: MoverOrder) -> Result<UnsetAckResult> {
    let authority = active.authority.clone();
    let timestamp = active.clock.now_ms();
    authority.send_mover_ack(order, timestamp, |opcode, body| {
        active.writer.send_raw(opcode, body)
    })
}

/// An undecodable self order is logged and ignored, not fatal: nothing was queued, so the
/// authority FIFO and counter are untouched (manager rule, 2026-10-16).
fn reject_unsupported_movement(authority: &MovementAuthorityHandle, opcode: u16) -> Result<()> {
    if mandatory_movement_order(opcode) {
        bevy::log::warn!(
            "net(wrath): ignoring unproven movement order {opcode:#05x} for {:#x}",
            authority.self_guid
        );
    }
    Ok(())
}

/// Self orders with no decoder: only the swim/fly transition pair, which AzerothCore registers but
/// never sends. Every other order has a native ACK path (`MoverOrder`, root, unset-can-fly).
fn mandatory_movement_order(opcode: u16) -> bool {
    matches!(opcode, 0x033e | 0x033f)
}

fn client_control_update(body: &[u8]) -> Result<(u64, bool)> {
    let mut rest = body;
    let guid = wrathplus_protocol::wire::read_packed_guid(&mut rest)
        .context("Wrath SMSG_CLIENT_CONTROL_UPDATE packed guid")?;
    if rest.len() != 1 || rest[0] > 1 {
        bail!("Wrath SMSG_CLIENT_CONTROL_UPDATE has invalid allowMove/body length");
    }
    Ok((guid, rest[0] != 0))
}

pub(crate) fn active_mover_packet(guid: u64) -> (u32, Vec<u8>) {
    (CMSG_SET_ACTIVE_MOVER, guid.to_le_bytes().to_vec())
}

pub(crate) fn ping_packet(sequence: u32, last_rtt_ms: u32) -> (u32, Vec<u8>) {
    let mut body = Vec::with_capacity(8);
    body.extend(sequence.to_le_bytes());
    body.extend(last_rtt_ms.to_le_bytes());
    (CMSG_PING, body)
}

pub(crate) fn logout_packet() -> (u32, Vec<u8>) {
    wrathplus_protocol::wrath_logout::encode_request()
}

pub(crate) fn item_query_packet(entry: u32, guid: u64) -> (u32, Vec<u8>) {
    (
        CMSG_ITEM_QUERY_SINGLE,
        wrathplus_protocol::wrath_items::encode_item_query(entry, guid),
    )
}

fn normalize_realms(
    realms: &[wrathplus_protocol::wrath_auth::RealmInfo],
) -> Vec<wrathplus_protocol::RealmInfo> {
    realms
        .iter()
        .map(|realm| wrathplus_protocol::RealmInfo {
            name: realm.name.clone(),
            address: realm.address.clone(),
            population: realm.population,
            characters: realm.characters,
            realm_type: u32::from(realm.icon),
            flags: realm.flags | if realm.locked { 0x02 } else { 0 },
            category: realm.timezone,
            id: realm.id,
        })
        .collect()
}

fn normalize_characters(characters: Vec<WrathCharacter>) -> Vec<Character> {
    characters.into_iter().map(normalize_character).collect()
}

fn normalize_character(character: WrathCharacter) -> Character {
    let mut equipment = [CharEnumItem::default(); 19];
    for (target, source) in equipment.iter_mut().zip(character.equipment.iter()) {
        *target = CharEnumItem {
            display_id: source.display_id,
            inventory_type: source.inventory_type,
        };
    }
    Character {
        guid: character.guid,
        name: character.name,
        race: character.race,
        class: character.class,
        gender: character.gender,
        skin: character.skin,
        face: character.face,
        hair_style: character.hair_style,
        hair_color: character.hair_color,
        facial_hair: character.facial_hair,
        level: character.level,
        zone: character.zone,
        map: character.map,
        position: wrathplus_protocol::wire::Vector3d {
            x: character.position[0],
            y: character.position[1],
            z: character.position[2],
        },
        flags: character.flags,
        customize_flags: character.customization_flags,
        equipment,
        pet_display_id: character.pet_display_id,
        pet_level: character.pet_level,
        pet_family: character.pet_family,
    }
}

fn retryable_timeout(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<WrathPartialFrameRead>().is_some() {
        return false;
    }
    error.downcast_ref::<io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        )
    })
}

fn one_byte(body: &[u8], packet: &str) -> Result<u8> {
    if body.len() != 1 {
        bail!("Wrath {packet} has {} bytes, expected 1", body.len());
    }
    Ok(body[0])
}

fn le_u32_exact(body: &[u8], packet: &str) -> Result<u32> {
    if body.len() != 4 {
        bail!("Wrath {packet} has {} bytes, expected 4", body.len());
    }
    Ok(u32::from_le_bytes(body.try_into().unwrap()))
}

fn send(events: &Sender<SessionEvent>, event: SessionEvent) -> Result<()> {
    events.send(event).map_err(|_| anyhow!("app exited"))
}

fn send_login_failure(events: &Sender<SessionEvent>, reason: &str, terminal: bool) {
    let _ = events.send(SessionEvent::LoginFailed {
        refusal: None,
        reason: reason.into(),
        terminal,
        dial: None,
    });
}

fn drain<T>(receiver: &Receiver<T>) {
    while receiver.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::wrath_world::WrathCharEnumItem;
    /// Movement authority: grants, denials, the self create and the vehicle hand-off.
    #[path = "authority.rs"]
    mod authority;
    /// Client-side encode failures drop the command; socket and authority failures end the session.
    #[path = "encode_drop.rs"]
    mod encode_drop;

    pub(super) fn test_self_position() -> MovementPosition {
        MovementPosition::Stationary(wrathplus_protocol::wrath_gameplay::Position {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            orientation: 0.5,
        })
    }

    #[test]
    fn immutable_endpoints_reject_old_sends_across_the_ecs_connection_barrier() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let old = crate::net::SessionCommandSender::new(tx.clone(), 7);
        let old_clone = old.clone();
        old.send(ClientCommand::Logout).unwrap();
        let new = crate::net::SessionCommandSender::new(tx, 8);
        old_clone.send(ClientCommand::ForceLogout).unwrap();
        new.send(ClientCommand::Logout).unwrap();

        let queued: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            queued
                .iter()
                .map(|command| command.epoch)
                .collect::<Vec<_>>(),
            [7, 7, 8]
        );
        let accepted: Vec<_> = queued
            .into_iter()
            .filter(|command| accepts_epoch(command.epoch, 8))
            .collect();
        assert_eq!(accepted.len(), 1);
        assert!(matches!(accepted[0].command, ClientCommand::Logout));
    }

    #[test]
    fn stale_internal_retire_and_fatal_are_rejected_by_new_epoch() {
        let active = 12;
        assert_ne!(11, active, "stale app envelope is rejected");

        let stale_internal = WrathWriterControl::Internal {
            epoch: 11,
            command: WrathWriterCommand::Reply(WrathReply {
                opcode: 1,
                body: Vec::new(),
            }),
        };
        let (retired, _ack) = crossbeam_channel::bounded(0);
        let stale_retire = WrathWriterControl::Retire { epoch: 11, retired };
        let stale_fatal = WrathWriterControl::Fatal {
            epoch: 11,
            reason: "old failure".into(),
        };
        for control in [stale_internal, stale_retire, stale_fatal] {
            let epoch = match control {
                WrathWriterControl::Internal { epoch, .. }
                | WrathWriterControl::Retire { epoch, .. }
                | WrathWriterControl::Fatal { epoch, .. } => epoch,
                WrathWriterControl::Install { .. } => unreachable!(),
            };
            assert_ne!(epoch, active);
        }
    }

    #[test]
    fn reader_guard_waits_for_synchronous_retirement_acknowledgement() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(23, 0x23);
        authority
            .observe_self_create(0x23, &test_self_position())
            .unwrap();
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
        let observed = authority.clone();
        let writer = std::thread::spawn(move || {
            let WrathWriterControl::Retire { epoch, retired } = rx.recv().unwrap() else {
                panic!("expected retirement");
            };
            assert_eq!(epoch, 23);
            assert!(observed.with_movement_send(|| ()).is_none());
            retired.send(()).unwrap();
        });
        {
            let _guard = WriterRetireGuard {
                epoch: 23,
                control: tx,
                authority: authority.clone(),
            };
        }
        assert!(authority.with_movement_send(|| ()).is_none());
        writer.join().unwrap();
    }

    #[test]
    fn realm_and_roster_normalization_preserve_shared_fields() {
        let realms = normalize_realms(&[wrathplus_protocol::wrath_auth::RealmInfo {
            icon: 1,
            locked: true,
            flags: 4,
            name: "Realm".into(),
            address: "127.0.0.1:8085".into(),
            population: 0.5,
            characters: 2,
            timezone: 8,
            id: 7,
            specified_build: None,
        }]);
        assert_eq!(realms[0].name, "Realm");
        assert_eq!(realms[0].realm_type, 1);
        assert_eq!(realms[0].flags, 6);
        assert_eq!(realms[0].category, 8);

        let mut source_equipment = [WrathCharEnumItem::default(); 23];
        source_equipment[18].display_id = 123;
        source_equipment[19].display_id = 999;
        let roster = normalize_characters(vec![WrathCharacter {
            guid: 5,
            name: "Hero".into(),
            race: 1,
            class: 1,
            gender: 0,
            skin: 2,
            face: 3,
            hair_style: 4,
            hair_color: 5,
            facial_hair: 6,
            level: 7,
            zone: 8,
            map: 9,
            position: [1.0, 2.0, 3.0],
            guild_id: 10,
            flags: 11,
            customization_flags: 12,
            first_login: true,
            pet_display_id: 13,
            pet_level: 14,
            pet_family: 15,
            equipment: source_equipment,
        }]);
        assert_eq!(roster[0].guid, 5);
        assert_eq!(roster[0].position.x, 1.0);
        assert_eq!(roster[0].equipment[18].display_id, 123);
        assert_eq!(roster[0].equipment.len(), 19);
    }

    #[test]
    fn exact_supported_packets_and_disabled_routes_are_explicit() {
        assert_eq!(
            active_mover_packet(0x0102_0304_0506_0708),
            (0x26a, vec![8, 7, 6, 5, 4, 3, 2, 1])
        );
        assert_eq!(ping_packet(3, 40), (0x1dc, vec![3, 0, 0, 0, 40, 0, 0, 0]));
        assert_eq!(logout_packet(), (0x4b, Vec::new()));
        assert_eq!(
            item_query_packet(0x1234_5678, 0x0123_4567_89ab_cdef),
            (
                0x56,
                vec![0x78, 0x56, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01,]
            )
        );
        assert_eq!(
            route(&ClientCommand::ItemQuery { entry: 1, guid: 2 }),
            WrathRoute::NativeItemQuery
        );
        assert_eq!(route(&ClientCommand::Logout), WrathRoute::Logout);
        assert_eq!(
            route(&ClientCommand::WorldportAck),
            WrathRoute::DisabledMovement
        );
        assert_eq!(
            route(&ClientCommand::QueryTime),
            WrathRoute::NativeSessionMisc
        );
    }

    #[test]
    fn quest_turn_in_and_abandon_route_exact_native_commands() {
        let npc = 0xf130_0001_2300_4567u64;
        let quest = 783u32;
        let guid_quest = [npc.to_le_bytes().as_slice(), quest.to_le_bytes().as_slice()].concat();
        for (command, opcode, body) in [
            (
                ClientCommand::QuestgiverComplete { npc, quest },
                0x18a,
                guid_quest.clone(),
            ),
            (
                ClientCommand::QuestgiverRequestReward { npc, quest },
                0x18c,
                guid_quest.clone(),
            ),
            (
                ClientCommand::QuestgiverChooseReward {
                    npc,
                    quest,
                    choice: 2,
                },
                0x18e,
                [guid_quest.as_slice(), 2u32.to_le_bytes().as_slice()].concat(),
            ),
            (ClientCommand::QuestlogRemove { slot: 24 }, 0x194, vec![24]),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeSession);
            assert_eq!(session_packet(command), (opcode, body));
        }
    }

    #[test]
    fn item_commands_route_native_inventory_bodies_and_use_item_shares_the_cast_counter() {
        for (command, opcode, body) in [
            (
                ClientCommand::SwapInvItem {
                    src_slot: 23,
                    dst_slot: 24,
                },
                0x10d,
                vec![24, 23],
            ),
            (
                ClientCommand::SplitItem {
                    src_bag: 255,
                    src_slot: 23,
                    dst_bag: 19,
                    dst_slot: 1,
                    count: 5,
                },
                0x10e,
                vec![255, 23, 19, 1, 5, 0, 0, 0],
            ),
            (
                ClientCommand::DestroyItem {
                    bag_index: 255,
                    slot: 23,
                    count: 0,
                },
                0x111,
                vec![255, 23, 0, 0, 0, 0],
            ),
            (
                ClientCommand::BankerActivate { guid: 7 },
                0x1b7,
                7u64.to_le_bytes().to_vec(),
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeInventory);
            assert_eq!(
                wrathplus_protocol::wrath_inventory::encode(&inventory_command(command)),
                (opcode, body)
            );
        }
        let use_item = || ClientCommand::UseItem {
            bag_index: 255,
            slot: 23,

            spell_id: 8690,
            item_guid: 0x4000_0000_0000_0001,
            glyph_index: 0,
            target: wrathplus_protocol::types::UseItemTarget::SelfImplicit,
        };
        assert_eq!(route(&use_item()), WrathRoute::NativeSession);
        // The cast count (byte 2) comes from the shared counter, which parallel tests also advance.
        let (opcode, body) = session_packet(use_item());
        assert_eq!(opcode, 0x0ab);
        assert_eq!(&body[..2], &[255, 23]);
        assert_eq!(&body[3..7], &8690u32.to_le_bytes());
        assert_eq!(&body[7..15], &0x4000_0000_0000_0001u64.to_le_bytes());
        assert_eq!(&body[15..], &[0; 9]);
    }

    #[test]
    fn trade_auction_and_master_loot_route_native_bodies() {
        for (command, opcode, body) in [
            (ClientCommand::AcceptTrade, 0x11a, vec![1, 0, 0, 0]),
            (
                ClientCommand::SetTradeItem {
                    trade_slot: 6,
                    bag: 255,
                    slot: 23,
                },
                0x11d,
                vec![6, 255, 23],
            ),
            (ClientCommand::CancelTrade, 0x11c, vec![]),
            (
                ClientCommand::AuctionSellItem {
                    auctioneer: 7,
                    items: vec![(9, 20)],
                    bid: 1,
                    buyout: 2,
                    etime_minutes: 720,
                },
                0x256,
                [
                    7u64.to_le_bytes().as_slice(),
                    &1u32.to_le_bytes(),
                    &9u64.to_le_bytes(),
                    &20u32.to_le_bytes(),
                    &1u32.to_le_bytes(),
                    &2u32.to_le_bytes(),
                    &720u32.to_le_bytes(),
                ]
                .concat(),
            ),
            (
                ClientCommand::LootMasterGive {
                    guid: 7,
                    slot: 3,
                    target: 9,
                },
                0x2a3,
                [7u64.to_le_bytes().as_slice(), &[3], &9u64.to_le_bytes()].concat(),
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeExchange);
            assert_eq!(exchange_packet(command), (opcode, body));
        }
    }

    #[test]
    fn chat_lines_speak_the_own_language_and_name_only_whisper_and_channel_targets() {
        use crate::net::ChatKind;
        let line = |kind, target: Option<&str>, language| ClientCommand::Chat {
            kind,
            target: target.map(str::to_string),
            text: "hi".into(),
            language,
        };
        assert_eq!(
            route(&line(ChatKind::Say, None, None)),
            WrathRoute::NativeChatLine
        );
        assert_eq!(
            chat_line(line(ChatKind::Say, None, None), 1).unwrap(),
            (
                0x95,
                [1u32.to_le_bytes(), 1u32.to_le_bytes()]
                    .concat()
                    .into_iter()
                    .chain(*b"hi\0")
                    .collect()
            )
        );
        assert_eq!(
            chat_line(line(ChatKind::Whisper, Some("Bob"), Some(7)), 1)
                .unwrap()
                .1,
            [7u32.to_le_bytes(), 7u32.to_le_bytes()]
                .concat()
                .into_iter()
                .chain(*b"Bob\0hi\0")
                .collect::<Vec<_>>()
        );
        let addon = ClientCommand::AddonMessage {
            distribution: wrathplus_ui::script::AddonDistribution::Guild,
            text: "p\tm".into(),
            target: None,
        };
        assert_eq!(
            chat_line(addon, 1).unwrap().1[..8],
            [4u32.to_le_bytes(), u32::MAX.to_le_bytes()].concat()[..]
        );
        // The 12340 WHISPER lane: chat type 0x07, LANG_ADDON, the target before the text.
        let whisper = ClientCommand::AddonMessage {
            distribution: wrathplus_ui::script::AddonDistribution::Whisper,
            text: "p\tm".into(),
            target: Some("Bob".into()),
        };
        assert_eq!(
            chat_line(whisper, 1).unwrap().1,
            [7u32.to_le_bytes(), u32::MAX.to_le_bytes()]
                .concat()
                .into_iter()
                .chain(*b"Bob\0p\tm\0")
                .collect::<Vec<_>>()
        );
        let embedded = ClientCommand::Chat {
            kind: ChatKind::Say,
            target: None,
            text: "a\0b".into(),
            language: None,
        };
        assert!(
            chat_line(embedded, 1).is_err(),
            "an embedded NUL is refused before send"
        );
        assert_eq!(
            route(&ClientCommand::LeaveChannel { name: "x".into() }),
            WrathRoute::NativeChannel
        );
        assert_eq!(
            wrathplus_protocol::wrath_chat_notices::encode(&channel_command(
                ClientCommand::ChannelKick {
                    name: "x".into(),
                    player: "Bob".into()
                }
            )),
            (0xa4, b"x\0Bob\0".to_vec())
        );
    }

    #[test]
    fn the_world_enter_status_request_is_native_and_other_queries_keep_their_routes() {
        assert_eq!(
            route(&ClientCommand::BattlefieldStatusRequest),
            WrathRoute::NativeBattleground
        );
        assert_eq!(
            battleground_packet(ClientCommand::BattlefieldStatusRequest),
            (0x02d3, Vec::new())
        );
        assert_eq!(
            route(&ClientCommand::QueryTime),
            WrathRoute::NativeSessionMisc
        );
        assert_eq!(
            route(&ClientCommand::WorldportAck),
            WrathRoute::DisabledMovement
        );
    }

    #[test]
    fn native_melee_packet_routing_is_exact_and_never_uses_a_vanilla_writer() {
        for (command, opcode, bytes) in [
            (
                ClientCommand::SetSelection { guid: 0x1234 },
                0x13d,
                0x1234u64.to_le_bytes().to_vec(),
            ),
            (
                ClientCommand::AttackSwing { guid: 0x5678 },
                0x141,
                0x5678u64.to_le_bytes().to_vec(),
            ),
            (ClientCommand::AttackStop, 0x142, Vec::new()),
            (
                ClientCommand::SetSheathed { state: 1 },
                0x1e0,
                1u32.to_le_bytes().to_vec(),
            ),
            (
                ClientCommand::StandStateChange { state: 0 },
                0x101,
                0u32.to_le_bytes().to_vec(),
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeMelee);
            assert_eq!(melee_packet(command).unwrap(), (opcode, bytes));
        }
        assert!(melee_packet(ClientCommand::SetSheathed { state: 3 }).is_err());
        assert!(melee_packet(ClientCommand::StandStateChange { state: 2 }).is_err());
    }

    #[test]
    fn trainer_commands_route_native_with_full_guid() {
        let trainer = 0xf130000000001234;
        for (command, opcode, body) in [
            (
                ClientCommand::TrainerList { trainer },
                0x1b0,
                trainer.to_le_bytes().to_vec(),
            ),
            (
                ClientCommand::TrainerBuySpell {
                    trainer,
                    spell_id: 2018,
                },
                0x1b2,
                [
                    trainer.to_le_bytes().as_slice(),
                    2018u32.to_le_bytes().as_slice(),
                ]
                .concat(),
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeSession);
            assert_eq!(session_packet(command), (opcode, body));
        }
    }

    #[test]
    fn solo_corpse_loot_routes_encode_native_bytes_and_keep_other_services_disabled() {
        for (command, opcode, body) in [
            (
                ClientCommand::Loot { guid: 0x1234 },
                0x15d,
                0x1234u64.to_le_bytes().to_vec(),
            ),
            (
                ClientCommand::AutostoreLootItem { slot: 17 },
                0x108,
                vec![17],
            ),
            (ClientCommand::LootMoney, 0x15e, vec![]),
            (
                ClientCommand::LootRelease { guid: 0x1234 },
                0x15f,
                0x1234u64.to_le_bytes().to_vec(),
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeLoot);
            assert_eq!(loot_packet(command).unwrap(), (opcode, body));
        }
        assert!(loot_packet(ClientCommand::Loot { guid: 0 }).is_err());
        assert!(loot_packet(ClientCommand::LootRelease { guid: 0 }).is_err());
        assert!(loot_packet(ClientCommand::AutostoreLootItem { slot: 18 }).is_err());
        assert_eq!(
            route(&ClientCommand::QueryTime),
            WrathRoute::NativeSessionMisc
        );
    }

    #[test]
    fn revoke_waits_for_in_flight_send_and_prevents_every_later_send() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(7, 0xdd);
        authority
            .observe_self_create(0xdd, &test_self_position())
            .unwrap();
        let sending = authority.clone();
        let revoking = authority.clone();
        let (entered_tx, entered_rx) = crossbeam_channel::bounded(0);
        let (release_tx, release_rx) = crossbeam_channel::bounded(0);
        let sent = Arc::new(AtomicU64::new(0));
        let sent_in_thread = Arc::clone(&sent);
        let writer = std::thread::spawn(move || {
            sending.with_movement_send(|| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                sent_in_thread.fetch_add(1, Ordering::SeqCst);
            })
        });
        entered_rx.recv().unwrap();
        let revoke = std::thread::spawn(move || revoking.revoke());
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        revoke.join().unwrap();

        assert_eq!(sent.load(Ordering::SeqCst), 1);
        assert!(authority
            .with_movement_send(|| sent.fetch_add(1, Ordering::SeqCst))
            .is_none());
        assert_eq!(sent.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unset_barrier_blocks_app_movement_and_releases_only_after_successful_ack() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(30, 0x301);
        authority
            .observe_self_create(0x301, &test_self_position())
            .unwrap();
        let order = UnsetCanFlyOrder {
            mover_guid: 0x301,
            counter: 0x1122_3344,
        };

        authority.raise_unset_barrier(order).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        let failed = authority.send_unset_ack(order, 9, |_| bail!("socket failure"));
        assert!(failed.is_err());
        assert!(authority.with_movement_send(|| ()).is_none());

        let mut ack = Vec::new();
        assert_eq!(
            authority
                .send_unset_ack(order, 10, |body| {
                    ack.extend_from_slice(body);
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        assert_eq!(&ack[3..7], &order.counter.to_le_bytes());
        assert_eq!(&ack[ack.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
    }

    #[test]
    fn pre_self_unset_waits_without_fabricating_pose_then_uses_actual_create() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(31, 0x311);
        let order = UnsetCanFlyOrder {
            mover_guid: 0x311,
            counter: 7,
        };
        authority.raise_unset_barrier(order).unwrap();
        let sends = AtomicU64::new(0);
        assert_eq!(
            authority
                .send_unset_ack(order, 1, |_| {
                    sends.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::PendingSelfPose
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);

        authority
            .observe_self_create(0x311, &test_self_position())
            .unwrap();
        let mut ack = Vec::new();
        assert_eq!(
            authority
                .send_unset_ack(order, 2, |body| {
                    ack.extend_from_slice(body);
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        assert_eq!(&ack[17..21], &1.0f32.to_le_bytes());
        assert_eq!(&ack[21..25], &2.0f32.to_le_bytes());
        assert_eq!(&ack[25..29], &3.0f32.to_le_bytes());
    }

    #[test]
    fn unset_does_not_restore_revoked_authority_and_stale_orders_retire() {
        let shared = Arc::new(MovementAuthority::default());
        let stale = shared.install(32, 0x321);
        stale
            .observe_self_create(0x321, &test_self_position())
            .unwrap();
        stale.revoke();
        let stale_order = UnsetCanFlyOrder {
            mover_guid: 0x321,
            counter: 8,
        };
        stale.raise_unset_barrier(stale_order).unwrap();
        assert_eq!(
            stale.send_unset_ack(stale_order, 3, |_| Ok(())).unwrap(),
            UnsetAckResult::Sent
        );
        assert!(stale.with_movement_send(|| ()).is_none());

        let current = shared.install(33, 0x331);
        assert_eq!(
            stale
                .send_unset_ack(stale_order, 4, |_| panic!("stale ACK sent"))
                .unwrap(),
            UnsetAckResult::Stale
        );
        assert!(current.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn root_then_unroot_keeps_denied_control_and_barrier_until_successful_write() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(40, 0x401);
        authority.observe_control(0x401, false).unwrap();
        authority
            .observe_self_create(0x401, &test_self_position())
            .unwrap();
        let order = ForceRootOrder {
            mover_guid: 0x401,
            counter: 11,
        };
        authority.raise_root_barrier(order, true).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        let mut opcodes = Vec::new();
        assert_eq!(
            authority
                .send_root_ack(order, true, 7, |opcode, _| {
                    opcodes.push(opcode);
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        assert_eq!(opcodes, [wrath_authority::CMSG_FORCE_MOVE_ROOT_ACK]);
        assert!(authority.with_movement_send(|| ()).is_none());

        let unroot = ForceRootOrder {
            mover_guid: 0x401,
            counter: 12,
        };
        authority.raise_root_barrier(unroot, false).unwrap();
        assert!(authority
            .send_root_ack(unroot, false, 8, |_, _| bail!("write failed"))
            .is_err());
        assert!(authority.with_movement_send(|| ()).is_none());
        authority
            .send_root_ack(unroot, false, 9, |_, _| Ok(()))
            .unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn airborne_authority_uses_latest_reported_pose_and_failed_sends_do_not_advance_it() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(70, 0x0500_0001);
        authority
            .observe_self_create(0x0500_0001, &test_self_position())
            .unwrap();
        let pose = OnFootPose {
            flags: wrath_ground::MOVE_FORWARD | wrath_ground::MOVE_FALLING,
            position: [8.0, 9.0, 12.0],
            orientation: 0.75,
            fall_time: 300,
            fall: Some(wrath_ground::FallTail {
                vertical_speed: 7.955547,
                sin_angle: 0.25,
                cos_angle: 0.75,
                horizontal_speed: 7.0,
            }),
            pitch: None,
            transport: None,
        };
        authority.with_self_send(pose, || Ok(())).unwrap().unwrap();
        let newer = OnFootPose {
            position: [9.0, 9.0, 12.5],
            ..pose
        };
        assert!(authority
            .with_self_send(newer, || bail!("write failed"))
            .unwrap()
            .is_err());
        let unset = UnsetCanFlyOrder {
            mover_guid: 0x0500_0001,
            counter: 20,
        };
        authority.raise_unset_barrier(unset).unwrap();
        assert!(authority
            .with_self_send(newer, || panic!("order barrier bypass"))
            .is_none());
        let mut body = Vec::new();
        authority
            .send_unset_ack(unset, 500, |bytes| {
                body = bytes.to_vec();
                Ok(())
            })
            .unwrap();
        assert_eq!(
            body,
            wrath_authority::encode_unset_can_fly_on_foot_ack(0x0500_0001, 20, pose, 500,).unwrap()
        );
        let root = ForceRootOrder {
            mover_guid: 0x0500_0001,
            counter: 21,
        };
        authority.raise_root_barrier(root, true).unwrap();
        let mut packets = Vec::new();
        authority
            .send_root_ack(root, true, 600, |opcode, body| {
                packets.push((opcode, body.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].0, wrath_ground::MSG_MOVE_STOP);
        let stopped =
            wrathplus_protocol::wrath_gameplay::OutboundMovement::from_bytes(&packets[0].1)
                .unwrap();
        assert_eq!(
            [
                stopped.movement.position.x,
                stopped.movement.position.y,
                stopped.movement.position.z
            ],
            pose.position
        );
        assert_eq!(stopped.movement.fall_time, 300);
        assert_eq!(stopped.movement.flags, 0);
        assert!(stopped.movement.jump.is_none());
        assert_eq!(
            packets[1].1,
            wrath_authority::encode_force_root_on_foot_ack(
                0x0500_0001,
                21,
                pose.halted(),
                600,
                true,
            )
            .unwrap()
        );
        assert!(authority
            .with_self_send(newer, || panic!("root bypass"))
            .is_none());
        authority.observe_control(0x0500_0001, false).unwrap();
        let replacement = shared.install(71, 0x0500_0001);
        assert!(authority
            .with_self_send(pose, || panic!("stale epoch bypass"))
            .is_none());
        assert!(replacement
            .with_self_send(pose, || panic!("pre-self bypass"))
            .is_none());
    }

    #[test]
    fn moving_root_stops_before_root_ack() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(41, 0x411);
        authority
            .observe_self_create(0x411, &test_self_position())
            .unwrap();
        authority
            .with_ground_send(
                GroundPose::new(wrath_ground::MOVE_FORWARD, [4.0, 5.0, 6.0], 0.25).unwrap(),
                || Ok(()),
            )
            .unwrap()
            .unwrap();
        let order = ForceRootOrder {
            mover_guid: 0x411,
            counter: 13,
        };
        authority.raise_root_barrier(order, true).unwrap();
        let mut opcodes = Vec::new();
        authority
            .send_root_ack(order, true, 1, |opcode, _| {
                opcodes.push(opcode);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            opcodes,
            [
                wrath_ground::MSG_MOVE_STOP,
                wrath_authority::CMSG_FORCE_MOVE_ROOT_ACK
            ]
        );
        assert!(authority.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn mixed_authority_orders_require_exact_fifo_ack_kind_and_counter() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(43, 0x431);
        authority
            .observe_self_create(0x431, &test_self_position())
            .unwrap();
        let unset = UnsetCanFlyOrder {
            mover_guid: 0x431,
            counter: 20,
        };
        let root = ForceRootOrder {
            mover_guid: 0x431,
            counter: 21,
        };
        authority.raise_unset_barrier(unset).unwrap();
        authority.raise_root_barrier(root, true).unwrap();
        assert!(authority
            .send_root_ack(root, true, 1, |_, _| Ok(()))
            .is_err());
        assert!(authority.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn duplicate_and_stale_root_orders_fail_closed() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(44, 0x441);
        authority
            .observe_self_create(0x441, &test_self_position())
            .unwrap();
        let root = ForceRootOrder {
            mover_guid: 0x441,
            counter: 30,
        };
        let unroot = ForceRootOrder {
            mover_guid: 0x441,
            counter: 31,
        };
        authority.raise_root_barrier(root, true).unwrap();
        authority.raise_root_barrier(unroot, false).unwrap();
        assert!(authority.raise_root_barrier(unroot, false).is_err());
        assert!(authority.raise_root_barrier(root, true).is_err());
        assert!(authority.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn queued_root_unroot_root_never_momentarily_allows_movement() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(45, 0x451);
        authority
            .observe_self_create(0x451, &test_self_position())
            .unwrap();
        let orders = [
            (
                ForceRootOrder {
                    mover_guid: 0x451,
                    counter: 40,
                },
                true,
            ),
            (
                ForceRootOrder {
                    mover_guid: 0x451,
                    counter: 41,
                },
                false,
            ),
            (
                ForceRootOrder {
                    mover_guid: 0x451,
                    counter: 42,
                },
                true,
            ),
        ];
        for (order, rooted) in orders {
            authority.raise_root_barrier(order, rooted).unwrap();
        }
        for (order, rooted) in orders {
            authority
                .send_root_ack(order, rooted, 1, |_, _| Ok(()))
                .unwrap();
            assert!(authority.with_movement_send(|| ()).is_none());
        }
    }

    #[test]
    fn pre_self_root_waits_without_writing() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(42, 0x421);
        let order = ForceRootOrder {
            mover_guid: 0x421,
            counter: 14,
        };
        authority.raise_root_barrier(order, true).unwrap();
        assert_eq!(
            authority
                .send_root_ack(order, true, 1, |_, _| panic!("pre-self write"))
                .unwrap(),
            UnsetAckResult::PendingSelfPose
        );
    }

    #[test]
    fn initial_pre_self_unroot_waits_for_pose_and_never_grants_denied_control() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(46, 0x461);
        authority.observe_control(0x461, false).unwrap();
        let order = ForceRootOrder {
            mover_guid: 0x461,
            counter: 50,
        };
        authority.raise_root_barrier(order, false).unwrap();
        assert_eq!(
            authority
                .send_root_ack(order, false, 1, |_, _| panic!("pre-self write"))
                .unwrap(),
            UnsetAckResult::PendingSelfPose
        );
        authority
            .observe_self_create(0x461, &test_self_position())
            .unwrap();

        let mut ack = Vec::new();
        assert_eq!(
            authority
                .send_root_ack(order, false, 2, |opcode, body| {
                    ack.push((opcode, body.to_vec()));
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        assert_eq!(ack.len(), 1);
        assert_eq!(ack[0].0, wrath_authority::CMSG_FORCE_MOVE_UNROOT_ACK);
        assert_eq!(&ack[0].1[17..21], &1.0f32.to_le_bytes());
        assert_eq!(&ack[0].1[21..25], &2.0f32.to_le_bytes());
        assert_eq!(&ack[0].1[25..29], &3.0f32.to_le_bytes());
        assert!(authority.with_movement_send(|| ()).is_none());
    }

    #[test]
    fn successful_allowed_self_initial_unroot_does_not_fabricate_root_state() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(47, 0x471);
        authority
            .observe_self_create(0x471, &test_self_position())
            .unwrap();
        let order = ForceRootOrder {
            mover_guid: 0x471,
            counter: 51,
        };
        authority.raise_root_barrier(order, false).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        assert_eq!(
            authority
                .send_root_ack(order, false, 3, |opcode, _| {
                    assert_eq!(opcode, wrath_authority::CMSG_FORCE_MOVE_UNROOT_ACK);
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
    }

    #[test]
    fn an_unknown_movers_unset_is_ignored_but_advances_the_counter() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(34, 0x341);
        authority
            .observe_self_create(0x341, &test_self_position())
            .unwrap();
        // AzerothCore would refuse its ACK (`guid != m_mover`): logged and not queued.
        assert!(!authority
            .raise_unset_barrier(UnsetCanFlyOrder {
                mover_guid: 0x999,
                counter: 1,
            })
            .unwrap());
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
        assert_eq!(
            authority.shared.state.lock().unwrap().last_order_counter,
            Some(1)
        );
    }
}

#[cfg(test)]
#[path = "wrath/bind_tests.rs"]
mod bind_tests;

#[cfg(test)]
#[path = "wrath/taxi_tests.rs"]
mod taxi_tests;

#[path = "wrath/taxi_authority.rs"]
mod taxi_authority;

#[cfg(test)]
mod group_tests {
    use super::*;
    #[test]
    fn group_commands_route_and_encode_native_bodies() {
        for (command, opcode, body) in [
            (
                ClientCommand::GroupInvite {
                    name: "Ally".into(),
                },
                0x6e,
                b"Ally\0\0\0\0\0".to_vec(),
            ),
            (ClientCommand::GroupAccept, 0x72, vec![0; 4]),
            (ClientCommand::GroupDecline, 0x73, vec![]),
            (ClientCommand::GroupLeave, 0x7b, vec![]),
            (
                ClientCommand::PartyAssignment {
                    assignment: 1,
                    apply: true,
                    guid: 0x8877665544332211,
                },
                0x38e,
                vec![1, 1, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
            ),
            (
                ClientCommand::PartyAssignment {
                    assignment: 0,
                    apply: false,
                    guid: 0,
                },
                0x38e,
                vec![0; 10],
            ),
            (
                ClientCommand::RequestPartyMemberStats { guid: 7 },
                0x27f,
                vec![7, 0, 0, 0, 0, 0, 0, 0],
            ),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeSession);
            assert_eq!(session_packet(command), (opcode, body));
        }
    }
}

#[cfg(test)]
#[path = "wrath/mail_tests.rs"]
mod mail_tests;

#[cfg(test)]
mod channel_command_tests {
    use super::*;
    #[test]
    fn native_channel_commands_route_and_encode_in_request_order() {
        let commands = [
            ClientCommand::SetChannelWatch {
                name: "General".into(),
            },
            ClientCommand::ChannelDisplayList {
                name: "General".into(),
            },
            ClientCommand::SetChannelWatch {
                name: String::new(),
            },
        ];
        for command in &commands {
            assert_eq!(route(command), WrathRoute::NativeSession);
        }
        let packets: Vec<_> = commands.into_iter().map(session_packet).collect();
        assert_eq!(
            packets,
            vec![
                (0x3ef, b"General\0".to_vec()),
                (0x3d2, b"General\0".to_vec()),
                (0x3ef, vec![0])
            ]
        );
    }
}

#[cfg(test)]
#[path = "wrath/raid_command_tests.rs"]
mod raid_command_tests;

#[cfg(test)]
mod lfg_command_tests {
    use super::*;
    #[test]
    fn native_lock_queries_route_to_exact_empty_packets() {
        for (command, opcode) in [
            (ClientCommand::LfgRequestPlayerLockInfo, 0x036e),
            (ClientCommand::LfgRequestPartyLockInfo, 0x0371),
        ] {
            assert_eq!(route(&command), WrathRoute::NativeSession);
            assert_eq!(session_packet(command), (opcode, Vec::new()));
        }
    }
}

#[cfg(test)]
mod native_loot_vote_command_tests {
    use super::*;
    #[test]
    fn exact_native_vote_address_and_type() {
        for vote in 0..=3 {
            let c = ClientCommand::LootRoll {
                looted_target: 0x0102030405060708,
                item_slot: 0xfedcba98,
                roll_type: vote,
            };
            assert_eq!(route(&c), WrathRoute::NativeSession);
            assert_eq!(
                session_packet(c),
                (
                    0x2a0,
                    vec![8, 7, 6, 5, 4, 3, 2, 1, 0x98, 0xba, 0xdc, 0xfe, vote]
                )
            );
        }
        for vote in 4..=255 {
            assert_eq!(
                route(&ClientCommand::LootRoll {
                    looted_target: 1,
                    item_slot: 2,
                    roll_type: vote
                }),
                WrathRoute::DisabledGameAction
            );
        }
    }
}

#[cfg(test)]
mod saved_instance_command_tests {
    use super::*;
    #[test]
    fn native_request_raid_info_is_empty_and_not_a_legacy_packet() {
        assert_eq!(
            route(&ClientCommand::RequestRaidInfo),
            WrathRoute::NativeSession
        );
        assert_eq!(
            session_packet(ClientCommand::RequestRaidInfo),
            (0x02cd, Vec::new())
        );
    }
}

#[cfg(test)]
#[path = "wrath/lfg_outbound_tests.rs"]
mod lfg_outbound_tests;

#[cfg(test)]
#[path = "wrath/guild_bank_tests.rs"]
mod guild_bank_tests;

#[cfg(test)]
mod repair_funding_tests {
    use super::*;

    #[test]
    fn existing_vendor_repairs_use_personal_money_only() {
        for item_guid in [0, 0x8877665544332211] {
            let command = ClientCommand::RepairItem {
                vendor: 0x0102030405060708,
                item_guid,
            };
            assert_eq!(route(&command), WrathRoute::NativeSession);
            let (opcode, body) = session_packet(command);
            assert_eq!(opcode, wrathplus_protocol::wrath_vendor::CMSG_REPAIR_ITEM);
            let mut expected = 0x0102030405060708u64.to_le_bytes().to_vec();
            expected.extend(item_guid.to_le_bytes());
            expected.push(0);
            assert_eq!(body, expected);
        }
    }
}

fn stable_packet(command: wrathplus_protocol::wrath_stable::Command) -> (u32, Vec<u8>) {
    (command.opcode(), command.encode())
}
#[cfg(test)]
#[path = "wrath/stable_tests.rs"]
mod stable_tests;

fn guild_packet(command: ClientCommand) -> std::io::Result<(u32, Vec<u8>)> {
    use wrathplus_protocol::wrath_guild_commands::Command as G;
    let command = match command {
        ClientCommand::GuildQuery { guild_id } => G::Query { guild_id },
        ClientCommand::GuildInfoRequest => G::Info,
        ClientCommand::GuildRosterRequest => G::Roster,
        ClientCommand::GuildInvite { name } => G::Invite {
            name: name.into_bytes(),
        },
        ClientCommand::GuildAccept => G::Accept,
        ClientCommand::GuildDecline => G::Decline,
        ClientCommand::GuildPromote { name } => G::Promote {
            name: name.into_bytes(),
        },
        ClientCommand::GuildDemote { name } => G::Demote {
            name: name.into_bytes(),
        },
        ClientCommand::GuildRemove { name } => G::Remove {
            name: name.into_bytes(),
        },
        ClientCommand::GuildMotd { motd } => G::Motd {
            text: motd.into_bytes(),
        },
        ClientCommand::GuildCreate { name } => G::Create {
            name: name.into_bytes(),
        },
        ClientCommand::GuildLeave => G::Leave,
        ClientCommand::GuildDisband => G::Disband,
        ClientCommand::GuildLeader { name } => G::Leader {
            name: name.into_bytes(),
        },
        ClientCommand::GuildAddRank { name } => G::AddRank {
            name: name.into_bytes(),
        },
        ClientCommand::GuildDelRank => G::DelRank,
        ClientCommand::GuildSetPublicNote { name, note } => G::SetNote {
            officer: false,
            name: name.into_bytes(),
            note: note.into_bytes(),
        },
        ClientCommand::GuildSetOfficerNote { name, note } => G::SetNote {
            officer: true,
            name: name.into_bytes(),
            note: note.into_bytes(),
        },
        ClientCommand::GuildInfoText { text } => G::InfoText {
            text: text.into_bytes(),
        },
        ClientCommand::SaveGuildEmblem { vendor, design } => G::SaveEmblem { vendor, design },
        ClientCommand::TabardVendorActivate { npc } => G::TabardVendorActivate { vendor: npc },
        ClientCommand::GuildEventLogQuery => G::EventLogQuery,
        // The rank save has its own codec (`wrath_guild_rank`): all six bank rows ride along.
        ClientCommand::NativeGuildRank(rank) => return Ok((rank.opcode(), rank.encode()?)),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a native core guild command",
            ))
        }
    };
    Ok((command.opcode(), command.encode()?))
}
#[cfg(test)]
#[path = "wrath/guild_tests.rs"]
mod guild_tests;

fn arena_team_packet(
    command: &wrathplus_protocol::wrath_arena_team::Command,
) -> std::io::Result<(u32, Vec<u8>)> {
    Ok((command.opcode(), command.encode()?))
}
#[cfg(test)]
#[path = "wrath/arena_team_tests.rs"]
mod arena_team_tests;

fn native_character_action_succeeded(action: wrathplus_protocol::CharAction, code: u8) -> bool {
    use wrathplus_protocol::wrath_world::{CHAR_CREATE_SUCCESS, CHAR_DELETE_SUCCESS};
    match action {
        wrathplus_protocol::CharAction::Create => code == CHAR_CREATE_SUCCESS,
        wrathplus_protocol::CharAction::Delete => code == CHAR_DELETE_SUCCESS,
    }
}

fn finish_character_action(
    session: &mut WrathWorldSession,
    characters: &mut Vec<Character>,
    realm: Option<wrathplus_protocol::RealmInfo>,
    action: wrathplus_protocol::CharAction,
    code: u8,
    publish: impl FnMut(SessionEvent) -> Result<()>,
) -> Result<()> {
    publish_character_action(
        action,
        code,
        || {
            *characters =
                normalize_characters(session.char_enum().context("Wrath refreshed roster")?);
            Ok(SessionEvent::CharacterList {
                characters: characters.clone(),
                realm,
            })
        },
        publish,
    )
}

// Refresh must complete and publish before success. A transport failure never fabricates a result.
fn publish_character_action(
    action: wrathplus_protocol::CharAction,
    code: u8,
    refresh: impl FnOnce() -> Result<SessionEvent>,
    mut publish: impl FnMut(SessionEvent) -> Result<()>,
) -> Result<()> {
    if native_character_action_succeeded(action, code) {
        publish(refresh()?)?;
    }
    publish(SessionEvent::WrathCharActionResult { action, code })
}

#[path = "wrath/char_service.rs"]
mod char_service;
#[cfg(test)]
#[path = "wrath/character_tests.rs"]
mod character_tests;

fn native_mail_packet(
    command: &wrathplus_protocol::wrath_mail_mutations::Command,
) -> std::io::Result<(u32, Vec<u8>)> {
    Ok((command.opcode(), command.encode()?))
}

#[cfg(test)]
#[path = "wrath/mail_mutation_tests.rs"]
mod mail_mutation_tests;
impl MovementAuthorityHandle {
    fn logout_event(&self, opcode: u16, body: &[u8]) -> io::Result<Option<SessionEvent>> {
        use wrathplus_protocol::wrath_logout::{decode, Packet};
        let packet = decode(opcode, body)?;
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != self.epoch || state.self_guid != self.self_guid {
            return Ok(None);
        }
        Ok(match packet {
            Packet::Response { reason, instant } => {
                if reason != 0 {
                    state.logout_pending = false;
                }
                Some(SessionEvent::LogoutResponse {
                    reason,
                    instant: instant != 0,
                })
            }
            Packet::Complete => {
                state.logout_pending = false;
                state.self_control = false;
                state.control_decided = true;
                state.taxi_spline = None;
                state.transfer_ack = None;
                Some(SessionEvent::LoggedOut)
            }
            Packet::CancelAck => {
                let pending = std::mem::take(&mut state.logout_pending);
                pending.then_some(SessionEvent::LogoutCancelled)
            }
        })
    }
}

#[cfg(test)]
#[path = "wrath/logout_tests.rs"]
mod logout_tests;

#[cfg(test)]
#[path = "wrath/pet_name_tests.rs"]
mod pet_name_tests;

#[cfg(test)]
#[path = "wrath/who_tests.rs"]
mod who_tests;

#[cfg(test)]
#[test]
fn completed_quest_query_is_native_and_empty() {
    assert_eq!(
        route(&ClientCommand::NativeQuestsCompletedQuery),
        WrathRoute::NativeQuestsCompletedQuery
    );
    assert_eq!(
        wrathplus_protocol::wrath_quests_completed::encode_query(),
        (0x500, vec![])
    );
}

#[cfg(test)]
#[path = "wrath_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[test]
fn ui_timer_query_is_native_and_empty() {
    assert_eq!(
        route(&ClientCommand::NativeUiTimerQuery),
        WrathRoute::NativeUiTimerQuery
    );
    assert_eq!(
        wrathplus_protocol::wrath_ui_timer::encode_query(),
        (0x4f6, vec![])
    );
}

#[cfg(test)]
mod mover_order_tests {
    use super::tests::test_self_position;
    use super::*;
    use wrathplus_protocol::types::{MoveMode, SpeedKind};
    use wrathplus_protocol::wrath_authority::{ModeOrderKind, SpeedOrderKind};

    fn installed(guid: u64) -> MovementAuthorityHandle {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(90, guid);
        authority
            .observe_self_create(guid, &test_self_position())
            .unwrap();
        authority
    }

    #[test]
    fn a_speed_order_holds_movement_until_its_exact_speed_is_acknowledged() {
        let authority = installed(0x501);
        let order = MoverOrder::Speed {
            kind: SpeedOrderKind::Run,
            mover_guid: 0x501,
            counter: 3,
            speed: 14.0,
        };
        authority.raise_mover_barrier(order).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        let mut sent = Vec::new();
        assert_eq!(
            authority
                .send_mover_ack(order, 7, |opcode, body| {
                    sent.push((opcode, body.to_vec()));
                    Ok(())
                })
                .unwrap(),
            UnsetAckResult::Sent
        );
        let [(opcode, body)] = &sent[..] else {
            panic!()
        };
        assert_eq!(*opcode, 0x00e3);
        assert_eq!(body[body.len() - 4..], 14.0f32.to_le_bytes());
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
    }

    #[test]
    fn a_granted_water_walk_stays_on_later_acks_and_the_land_walk_clears_it() {
        let authority = installed(0x502);
        let walk = MoverOrder::Mode {
            kind: ModeOrderKind::WaterWalk,
            apply: true,
            mover_guid: 0x502,
            counter: 4,
        };
        authority.raise_mover_barrier(walk).unwrap();
        authority.send_mover_ack(walk, 1, |_, _| Ok(())).unwrap();
        let speed = MoverOrder::Speed {
            kind: SpeedOrderKind::Swim,
            mover_guid: 0x502,
            counter: 5,
            speed: 4.7,
        };
        authority.raise_mover_barrier(speed).unwrap();
        let mut flags = 0;
        authority
            .send_mover_ack(speed, 2, |_, body| {
                flags = u32::from_le_bytes(body[7..11].try_into().unwrap());
                Ok(())
            })
            .unwrap();
        assert_eq!(flags, wrath_authority::MOVEMENT_FLAG_WATER_WALKING);
        let land = MoverOrder::Mode {
            kind: ModeOrderKind::WaterWalk,
            apply: false,
            mover_guid: 0x502,
            counter: 6,
        };
        authority.raise_mover_barrier(land).unwrap();
        let mut opcode = 0;
        authority
            .send_mover_ack(land, 3, |sent, body| {
                opcode = sent;
                flags = u32::from_le_bytes(body[7..11].try_into().unwrap());
                Ok(())
            })
            .unwrap();
        assert_eq!((opcode, flags), (0x02d0, 0));
    }

    #[test]
    fn a_near_teleport_waits_for_the_controllers_snap_and_moves_the_reported_pose() {
        let authority = installed(0x503);
        let order = MoverOrder::TeleportNear {
            mover_guid: 0x503,
            counter: 8,
            position: [50.0, 60.0, 70.0],
            orientation: 2.0,
            transport: None,
        };
        authority.raise_mover_barrier(order).unwrap();
        assert!(matches!(
            mover_order_event(&order),
            Some(SessionEvent::Teleport { counter: 8, .. })
        ));
        // Never acknowledged by the writer itself, and movement stays held.
        assert!(authority.send_mover_ack(order, 1, |_, _| Ok(())).is_err());
        let authority = installed(0x504);
        let order = MoverOrder::TeleportNear {
            mover_guid: 0x504,
            counter: 8,
            position: [50.0, 60.0, 70.0],
            orientation: 2.0,
            transport: None,
        };
        authority.raise_mover_barrier(order).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        // A wrong counter is not the head order and sends nothing.
        assert!(!authority
            .send_teleport_ack(0x504, 9, 1, |_| panic!("wrong counter"))
            .unwrap());
        let mut body = Vec::new();
        assert!(authority
            .send_teleport_ack(0x504, 8, 0x55, |sent| {
                body = sent.to_vec();
                Ok(())
            })
            .unwrap());
        assert_eq!(body, wrath_authority::encode_teleport_ack(0x504, 8, 0x55));
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
        // Later ACKs report from the destination.
        let state = authority.shared.state.lock().unwrap();
        assert_eq!(state.ground_pose.unwrap().position, [50.0, 60.0, 70.0]);
        drop(state);
        assert_eq!(
            route(&ClientCommand::TeleportAck {
                guid: 0x504,
                counter: 8
            }),
            WrathRoute::NativeTeleportAck
        );
    }

    #[test]
    fn transport_near_teleport_preserves_tail_after_single_use_ack_not_before() {
        let authority = installed(0x504);
        let tail = wrath_ground::TransportTail {
            guid: 42,
            position: [1.0, 2.0, 3.0],
            orientation: 0.4,
            time: 456,
        };
        let order = MoverOrder::TeleportNear {
            mover_guid: 0x504,
            counter: 8,
            position: [500.0, 600.0, 700.0],
            orientation: 2.0,
            transport: Some(tail),
        };
        authority.raise_mover_barrier(order).unwrap();
        assert!(authority.with_movement_send(|| ()).is_none());
        assert!(!authority
            .send_teleport_ack(0x504, 9, 55, |_| panic!())
            .unwrap());
        // A failed write does not retire the order or publish its destination.
        assert!(authority
            .send_teleport_ack(0x504, 8, 55, |_| anyhow::bail!("synthetic write failure"))
            .is_err());
        assert!(authority.with_movement_send(|| ()).is_none());
        assert!(authority
            .send_teleport_ack(0x504, 8, 55, |body| {
                assert_eq!(body, wrath_authority::encode_teleport_ack(0x504, 8, 55));
                Ok(())
            })
            .unwrap());
        assert!(!authority
            .send_teleport_ack(0x504, 8, 55, |_| panic!("duplicate"))
            .unwrap());
        let pose = authority.shared.state.lock().unwrap().ground_pose.unwrap();
        assert_eq!(pose.transport, Some(tail));
        assert_eq!(pose.flags, wrath_ground::MOVE_ON_TRANSPORT);
        assert_eq!(pose.position, [500.0, 600.0, 700.0]);
        // A later forced-speed ACK carries the destination and transport tail together.
        let speed = MoverOrder::Speed {
            kind: SpeedOrderKind::Run,
            mover_guid: 0x504,
            counter: 9,
            speed: 7.0,
        };
        authority.raise_mover_barrier(speed).unwrap();
        authority
            .send_mover_ack(speed, 56, |_, body| {
                let mut input = body;
                let guid = wrathplus_protocol::wire::read_packed_guid(&mut input).unwrap();
                let mut framed = Vec::new();
                wrathplus_protocol::wire::write_packed_guid(guid, &mut framed).unwrap();
                framed.extend(&input[4..input.len() - 4]); // counter and acknowledged speed
                let movement =
                    wrathplus_protocol::wrath_gameplay::OutboundMovement::from_bytes(&framed)
                        .unwrap()
                        .movement;
                assert_eq!(movement.transport.unwrap().guid, 42);
                assert_eq!(movement.position.x, 500.0);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn knockback_and_flight_orders_map_to_the_controller_only_where_it_models_them() {
        let knock = MoverOrder::KnockBack {
            mover_guid: 1,
            counter: 2,
            cos_angle: 0.6,
            sin_angle: 0.8,
            horizontal_speed: 12.0,
            vertical_speed: -9.0,
        };
        let Some(SessionEvent::KnockBack { launch, .. }) = mover_order_event(&knock) else {
            panic!()
        };
        assert_eq!(
            (
                launch.zspeed,
                launch.cos_angle,
                launch.sin_angle,
                launch.xy_speed
            ),
            (-9.0, 0.6, 0.8, 12.0)
        );
        assert!(matches!(
            mover_order_event(&MoverOrder::Speed {
                kind: SpeedOrderKind::Flight,
                mover_guid: 1,
                counter: 2,
                speed: 7.0,
            }),
            Some(SessionEvent::ForceSpeedChange { kind: SpeedKind::Flight, speed, .. }) if speed == 7.0
        ));
        assert!(matches!(
            mover_order_event(&MoverOrder::Mode {
                kind: ModeOrderKind::CanFly,
                apply: true,
                mover_guid: 1,
                counter: 2,
            }),
            Some(SessionEvent::MoveMode {
                mode: MoveMode::CanFly,
                apply: true,
                ..
            })
        ));
        for order in [
            MoverOrder::Mode {
                kind: ModeOrderKind::DisableGravity,
                apply: true,
                mover_guid: 1,
                counter: 2,
            },
            MoverOrder::CollisionHeight {
                mover_guid: 1,
                counter: 2,
                height: 2.0,
            },
        ] {
            assert!(mover_order_event(&order).is_none(), "{order:?}");
        }
    }

    #[test]
    fn a_swimming_pose_is_reported_and_a_root_keeps_the_swimmer_in_the_water() {
        let authority = installed(0x508);
        let swimming = OnFootPose {
            flags: wrathplus_protocol::wrath_ground::MOVE_SWIMMING,
            position: [1.0, 2.0, -3.0],
            orientation: 0.5,
            fall_time: 0,
            fall: None,
            pitch: Some(-0.3),
            transport: None,
        };
        assert!(authority
            .with_self_send(swimming, || Ok(()))
            .unwrap()
            .is_ok());
        let root = ForceRootOrder {
            mover_guid: 0x508,
            counter: 20,
        };
        authority.raise_root_barrier(root, true).unwrap();
        let mut sent = Vec::new();
        authority
            .send_root_ack(root, true, 3, |opcode, body| {
                sent.push((opcode, body.to_vec()));
                Ok(())
            })
            .unwrap();
        // No STOP: a still swimmer is not in motion.
        let [(opcode, body)] = &sent[..] else {
            panic!("{sent:?}")
        };
        assert_eq!(*opcode, wrath_authority::CMSG_FORCE_MOVE_ROOT_ACK);
        let flags = u32::from_le_bytes(body[7..11].try_into().unwrap());
        assert_eq!(
            flags,
            wrath_authority::MOVEMENT_FLAG_ROOT | wrathplus_protocol::wrath_ground::MOVE_SWIMMING
        );
        // The pitch rides after the position, before the fall time.
        let pitch_at = 7 + 4 + 2 + 4 + 16;
        assert_eq!(
            f32::from_le_bytes(body[pitch_at..pitch_at + 4].try_into().unwrap()),
            -0.3
        );
    }

    #[test]
    fn a_stale_counter_fails_closed_and_an_unknown_mover_is_ignored() {
        let authority = installed(0x505);
        let order = MoverOrder::Speed {
            kind: SpeedOrderKind::Walk,
            mover_guid: 0x505,
            counter: 10,
            speed: 2.5,
        };
        authority.raise_mover_barrier(order).unwrap();
        assert!(authority
            .raise_mover_barrier(MoverOrder::Speed {
                kind: SpeedOrderKind::Walk,
                mover_guid: 0x505,
                counter: 10,
                speed: 2.5,
            })
            .is_err());
        let authority = installed(0x506);
        assert!(!authority
            .raise_mover_barrier(MoverOrder::Speed {
                kind: SpeedOrderKind::Walk,
                mover_guid: 0x999,
                counter: 10,
                speed: 2.5,
            })
            .unwrap());
        assert_eq!(authority.with_movement_send(|| 1), Some(1));
    }
}

#[cfg(test)]
mod observer_teleport_tests {
    use super::tests::test_self_position;
    use super::*;
    use wrathplus_protocol::wrath_gameplay::{MovementInfo, OutboundMovement, Position};

    fn foreign_teleport(guid: u64, flags: u32) -> Vec<u8> {
        OutboundMovement {
            mover_guid: guid,
            movement: MovementInfo {
                flags,
                flags2: 0,
                timestamp: 1,
                position: Position {
                    x: 900.0,
                    y: 800.0,
                    z: 70.0,
                    orientation: 1.0,
                },
                transport: None,
                // Swimming and flying carry their pitch.
                pitch: (flags & 0x0220_0000 != 0).then_some(0.25),
                fall_time: 0,
                jump: None,
                spline_elevation: (flags & 0x0400_0000 != 0).then_some(1.0),
            },
        }
        .to_bytes()
        .unwrap()
    }

    type Snapshot = (
        Option<OnFootPose>,
        Vec<PendingAuthorityOrder>,
        Option<u32>,
        bool,
        bool,
    );

    fn snapshot(authority: &MovementAuthorityHandle) -> Snapshot {
        let state = authority.shared.state.lock().unwrap();
        (
            state.ground_pose,
            state.pending_orders.iter().copied().collect(),
            state.last_order_counter,
            state.self_control,
            state.rooted,
        )
    }

    /// AzerothCore sends `MSG_MOVE_TELEPORT` to the set without the teleporter: for us it is
    /// always another unit, a relay, never an order on our mover.
    #[test]
    fn an_observed_teleport_for_another_guid_never_touches_our_authority() {
        let shared = Arc::new(MovementAuthority::default());
        let authority = shared.install(91, 0x601);
        authority
            .observe_self_create(0x601, &test_self_position())
            .unwrap();
        let before = snapshot(&authority);
        let (events_tx, events_rx) = crossbeam_channel::unbounded();
        let (control_tx, control_rx) = crossbeam_channel::unbounded();
        let mut adapter = WrathEventAdapter::new();
        let clock = SessionClock::new();
        // An on-foot teleport relays as the observed unit's move; one with spline elevation is outside
        // the relay subset and drops. Neither is fatal, and neither moves or blocks our mover.
        for (flags, dropped) in [(0, false), (0x0400_0000, true)] {
            let packet = wrathplus_protocol::wrath_world::WrathServerPacket {
                opcode: 0x00c5,
                body: foreign_teleport(0x0777, flags),
            };
            dispatch_packet(
                &events_tx,
                &control_tx,
                91,
                &mut adapter,
                &packet,
                &clock,
                &authority,
            )
            .unwrap();
            let event = events_rx.try_recv().unwrap();
            if dropped {
                assert!(matches!(
                    event,
                    SessionEvent::PacketDropped {
                        opcode: 0x00c5,
                        unparseable: true
                    }
                ));
            } else {
                assert!(matches!(
                    event,
                    SessionEvent::UnitMove {
                        guid: 0x0777,
                        verb: wrathplus_protocol::types::RelayVerb::Teleport,
                        ..
                    }
                ));
            }
            assert!(control_rx.try_recv().is_err(), "no writer command");
            assert_eq!(snapshot(&authority), before);
            assert_eq!(authority.with_movement_send(|| 1), Some(1));
        }
        assert!(!mandatory_movement_order(0x00c5));
        assert!(!wrath_authority::is_mover_order(0x00c5));
    }
}

#[cfg(test)]
mod release_spirit_tests;
