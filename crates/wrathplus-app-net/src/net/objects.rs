//! The bridge's object layer: the streamed world's creates, deltas and destroys, movers' speeds,
//! GameObject templates and anims, and items. Each packet is one handler over [`Scene`] whose
//! commands apply before the next packet's, so a create's store and speeds are live components by
//! the time the next packet reads them. The movement packets' halves are `crate::motion`'s.
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use wrathplus_protocol::{guid, EntityKind, MoveSpeeds, ObjectFields, SpeedKind};
use wrathplus_protocol::{SessionEvent, SessionEventKind};

use crate::app_core::death::DeathNet;
use crate::app_core::movement::{gameobject_rotation, pose_transform, wire_yaw};
use crate::go_templates::GameObjectTemplates;
use crate::items::Items;
use crate::names::NameCache;
use wrathplus_world::model_fade::DespawnFade;
use wrathplus_world::vis_chain::VisChainOnly;

mod speed;
mod transport_seed;
use super::{
    merge_store_fields, FieldChanged, Guid, GuidIndex, NetCommands, NetEntity, NetHandlerApp,
    ObjectStore, SelfGuid, SpeedChangeMessage, UnitSpeeds,
};
use speed::set_speed;

/// `SMSG_GAMEOBJECT_CUSTOM_ANIM`, bridged to [`crate::go_anim`], which rejects `anim_id >= 4` and
/// maps the rest to AnimationData 153..156; the fishing bobber's splash is `anim_id` 0.
fn gameobject_custom_anim(
    guid: u64,
    anim_id: u32,
    plays: &mut MessageWriter<crate::app_core::anim::GoCustomAnim>,
) {
    debug!("net: gameobject {guid:#x} custom anim {anim_id}");
    plays.write(crate::app_core::anim::GoCustomAnim {
        go_guid: guid,
        anim_id,
    });
}

/// `SMSG_GAMEOBJECT_DESPAWN_ANIM`: AnimationData 157 on the one-shot channel (`0x5f8c50`).
/// vmangos sends `SMSG_DESTROY_OBJECT` in the same tick, so the mark is a component queued now,
/// ahead of the closure [`object_destroyed`] queues, not a message.
fn gameobject_despawn_anim(guid: u64, commands: &mut Commands, index: &GuidIndex) {
    debug!("net: gameobject {guid:#x} despawn anim");
    if let Some(&e) = index.0.get(&guid) {
        commands
            .entity(e)
            .insert(crate::app_core::anim::DespawnAnimAnnounced);
    }
}

/// Registers the object layer's handlers. The movement halves follow on the same events in
/// `crate::motion`, registered after these.
pub(super) fn register(app: &mut App) {
    use SessionEventKind as K;
    app.net_handler(K::ObjectCreate, on_object_create)
        .net_handler(K::ItemCreate, on_item)
        .net_handler(K::ItemTemplate, on_item)
        .net_handler(K::ItemTime, on_item)
        .net_handler(K::ItemEnchantTime, on_item)
        .net_handler(K::UnitMove, on_unit_move)
        .net_handler(K::ObjectValues, on_object_values)
        .net_handler(K::ObjectDestroyed, on_object_destroyed)
        .net_handler(K::ObjectsRemoved, on_objects_removed)
        .net_handler(K::GameObjectInfo, on_gameobject_info)
        .net_handler(K::GameObjectCustomAnim, on_gameobject_anim)
        .net_handler(K::GameObjectDespawnAnim, on_gameobject_anim)
        .net_handler(K::ForceSpeedChange, on_speed)
        .net_handler(K::SpeedChanged, on_speed);
}

/// The streamed world as one parameter: the components, caches, lifecycle hooks and edges the
/// object packets touch.
#[derive(SystemParam)]
pub(crate) struct Scene<'w, 's> {
    commands: Commands<'w, 's>,
    net: Res<'w, NetCommands>,
    index: ResMut<'w, GuidIndex>,
    self_guid: Res<'w, SelfGuid>,
    transforms: Query<'w, 's, &'static mut Transform>,
    stores: Query<'w, 's, &'static mut ObjectStore>,
    countdowns: Query<'w, 's, &'static mut crate::items::Countdowns>,
    speeds: Query<'w, 's, &'static mut UnitSpeeds>,
    field_changes: MessageWriter<'w, FieldChanged>,
    speed_changes: MessageWriter<'w, SpeedChangeMessage>,
    self_moves: MessageWriter<'w, super::SelfMoveMessage>,
    go_custom_anims: MessageWriter<'w, crate::app_core::anim::GoCustomAnim>,
    names: Res<'w, NameCache>,
    items: ResMut<'w, Items>,
    go_templates: ResMut<'w, GameObjectTemplates>,
    death_net: ResMut<'w, DeathNet>,
    group: Res<'w, crate::app_core::group::GroupState>,
    deactivated: MessageWriter<'w, super::MemberDeactivated>,
}

fn on_object_create(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::ObjectCreate {
        guid,
        kind,
        display_id,
        position,
        orientation,
        rotation,
        scale,
        speeds,
        mover,
        transport_progress,
        transport,
        spline: _,
        fields,
    } = ev
    {
        note_corpse(guid, kind, &fields, &sc.self_guid, &mut sc.death_net);
        object_create(
            guid,
            kind,
            display_id,
            position,
            orientation,
            rotation,
            scale,
            speeds,
            mover,
            transport_progress,
            transport,
            fields,
            &mut sc.commands,
            &mut sc.index,
            &mut sc.transforms,
            &mut sc.stores,
            &mut sc.field_changes,
            &sc.names,
            &sc.go_templates,
            &sc.net,
        );
    }
}

/// Items: a descriptor-only create, `SMSG_ITEM_QUERY_SINGLE_RESPONSE` (a miss is cached as `None`
/// and never re-asked), and the lifetime and enchant countdowns, written into the item's own
/// [`crate::items::Countdowns`].
fn on_item(In(ev): In<SessionEvent>, mut sc: Scene) {
    match ev {
        SessionEvent::ItemCreate {
            guid,
            container,
            fields,
        } => item_create(
            guid,
            container,
            fields,
            &mut sc.commands,
            &mut sc.index,
            &mut sc.stores,
            &mut sc.field_changes,
            &mut sc.items,
        ),
        SessionEvent::ItemTemplate { entry, info } => {
            let info = info.map(|b| *b);
            debug!("net: item template {entry} → {info:?}");
            sc.items.insert_template(entry, info);
        }
        // `0x5e4f30`'s two arms share the item lookup (`0x468460`, `TYPEMASK_ITEM`) and differ on
        // a miss.
        SessionEvent::ItemTime { item_guid, seconds } => {
            // An unheld item drops the update, with no queue (`0x1EA`'s arm returns), so an item
            // created after the login re-send (`Player::SendItemDurations`) waits for the next.
            match sc.index.0.get(&item_guid).copied() {
                Some(e) if sc.countdowns.contains(e) => {
                    if let Ok(mut c) = sc.countdowns.get_mut(e) {
                        c.set_lifetime(seconds);
                    }
                    debug!("item duration: item {item_guid:#x} → {seconds}s");
                }
                _ => debug!("item duration: {item_guid:#x} names an item we do not hold — dropped"),
            }
        }
        SessionEvent::ItemEnchantTime {
            item_guid,
            slot,
            seconds,
        } => match sc.index.0.get(&item_guid).copied() {
            Some(e) if sc.countdowns.contains(e) => {
                if let Ok(mut c) = sc.countdowns.get_mut(e) {
                    if c.set_enchant(slot, seconds) {
                        debug!("enchant timer: item {item_guid:#x} slot {slot} → {seconds}s");
                    } else {
                        debug!("enchant timer: item {item_guid:#x} slot {slot} is past the seventh — refused");
                    }
                }
            }
            // `0x1EB`'s miss arm queues on a resolving player's list (`0x5ebd40`), and only the
            // active player's is replayed (`0x5d8440` → `0x5ebde0`). vmangos names the owner, us
            // (`Player.cpp:11883`, `:12077`), so it queues on the active player.
            _ if sc.self_guid.0.is_some_and(|g| sc.index.0.contains_key(&g)) => {
                sc.items.queue_enchant_time(item_guid, slot, seconds)
            }
            _ => debug!("enchant timer: {item_guid:#x} unheld and no player to queue on — dropped"),
        },
        _ => {}
    }
}

/// Our own guid's relayed move: the server writing our pose (`SetAsServerSide`, `ctime = 0`),
/// which the reference applies with no mover gate (`0x603bb0`); it goes to `player::wire_in`. An
/// observed unit's is `crate::motion`'s.
fn on_unit_move(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::UnitMove {
        guid,
        position,
        orientation,
        flags,
        pitch,
        fall_time,
        jump,
        transport,
        ..
    } = ev
    {
        if sc.self_guid.0 == Some(guid) {
            sc.self_moves.write(super::SelfMoveMessage {
                position,
                orientation,
                flags,
                pitch,
                fall_time,
                jump,
                transport,
            });
        }
    }
}

/// Our corpse's `CORPSE_FIELD_FLAGS` can flip to BONES under a live guid; the reclaim latch is
/// re-asked on that edge, as the reference's `FLAGS` handler does (`0x5d6d60`).
fn on_object_values(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::ObjectValues { guid, fields } = ev {
        recheck_corpse(guid, &fields, &sc.self_guid, &mut sc.death_net);
        object_values(
            guid,
            fields,
            &mut sc.commands,
            &sc.index,
            &mut sc.stores,
            &mut sc.field_changes,
        );
    }
}

/// The party hook runs first: the reference's deactivate reads the descriptor about to go.
fn on_object_destroyed(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::ObjectDestroyed(guid) = ev {
        forget_corpse(guid, &mut sc.death_net);
        let held = |g: u64| sc.index.0.get(&g).and_then(|e| sc.stores.get(*e).ok());
        sc.deactivated
            .write_batch(member_deactivated(guid, &sc.group, held));
        object_destroyed(guid, &mut sc.commands, &mut sc.index);
    }
}

/// The deactivate hook's input for `guid`, a roster member we hold an object for; `None` off the
/// roster or without an object, since the reference's hook is a virtual on the object.
pub(super) fn member_deactivated<'a>(
    guid: u64,
    group: &crate::app_core::group::GroupState,
    held: impl Fn(u64) -> Option<&'a ObjectStore>,
) -> Option<super::MemberDeactivated> {
    if !group.members.iter().any(|m| m.guid == guid) {
        return None;
    }
    let member = held(guid)?.clone();
    let pet = member
        .0
        .unit_pet_guid()
        .and_then(|pet| Some((pet, held(pet)?.clone())));
    Some(super::MemberDeactivated { guid, member, pet })
}

/// OUT_OF_RANGE takes the same deactivate as DESTROY in the reference, so a member walking out of
/// range also gets the snapshot and `CMSG_REQUEST_PARTY_MEMBER_STATS`.
fn on_objects_removed(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::ObjectsRemoved(guids) = ev {
        for guid in &guids {
            let held = |g: u64| sc.index.0.get(&g).and_then(|e| sc.stores.get(*e).ok());
            sc.deactivated
                .write_batch(member_deactivated(*guid, &sc.group, held));
        }
        objects_removed(guids, &mut sc.commands, &mut sc.index);
    }
}

fn on_gameobject_info(In(ev): In<SessionEvent>, mut sc: Scene) {
    if let SessionEvent::GameObjectInfo {
        entry,
        type_id,
        display_id,
        name,
        data,
    } = ev
    {
        gameobject_info(
            entry,
            type_id,
            display_id,
            name,
            &data,
            &mut sc.go_templates,
        );
    }
}

fn on_gameobject_anim(In(ev): In<SessionEvent>, mut sc: Scene) {
    match ev {
        SessionEvent::GameObjectCustomAnim { guid, anim_id } => {
            gameobject_custom_anim(guid, anim_id, &mut sc.go_custom_anims)
        }
        SessionEvent::GameObjectDespawnAnim { guid } => {
            gameobject_despawn_anim(guid, &mut sc.commands, &sc.index)
        }
        _ => {}
    }
}

fn on_speed(In(ev): In<SessionEvent>, mut sc: Scene) {
    match ev {
        SessionEvent::ForceSpeedChange {
            guid,
            kind,
            counter,
            speed,
        } => force_speed_change(
            guid,
            kind,
            counter,
            speed,
            &sc.index,
            &mut sc.speeds,
            &sc.self_guid,
            &mut sc.speed_changes,
        ),
        SessionEvent::SpeedChanged { guid, kind, speed } => {
            speed_changed(guid, kind, speed, &sc.index, &mut sc.speeds)
        }
        _ => {}
    }
}

/// An `SMSG_UPDATE_OBJECT` create block: spawns or refreshes the entity, warms the ask-once
/// caches and seeds its descriptor store.
fn object_create(
    guid: u64,
    kind: EntityKind,
    display_id: Option<u32>,
    position: [f32; 3],
    orientation: f32,
    rotation: Option<[f32; 4]>,
    scale: f32,
    speeds: Option<MoveSpeeds>,
    mover: Option<wrathplus_protocol::MoverState>,
    transport_progress: Option<u32>,
    transport: Option<wrathplus_protocol::TransportPose>,
    fields: ObjectFields,
    commands: &mut Commands,
    index: &mut GuidIndex,
    transforms: &mut Query<&mut Transform>,
    stores: &mut Query<&mut ObjectStore>,
    edges: &mut MessageWriter<FieldChanged>,
    names: &NameCache,
    go_templates: &GameObjectTemplates,
    net_commands: &NetCommands,
) {
    let net = NetEntity {
        kind,
        display_id,
        scale,
    };
    // A transport's cycle anchor: the create block's `UPDATE_FLAG_TRANSPORT` u32 and when it
    // landed; a re-create re-anchors. The reference's per-frame tick (`0x630970`) runs types 15
    // (boats, a path-progress clock) and 11 (lifts, `time-since-create % period`,
    // `GameObject.cpp:246`).
    let go_type = (kind == EntityKind::GameObject).then(|| fields.gameobject_type_id());
    let transport_anchor = matches!(go_type, Some(11 | 15))
        .then_some(transport_progress)
        .flatten()
        .map(|progress_ms| crate::app_core::movement::TransportAnchor {
            progress_ms,
            at: std::time::Instant::now(),
        });
    // UPDATEFLAG_ROTATION is model placement, not a type-11 path's descriptor quaternion.
    let go_quat = rotation;
    // AC StaticTransport::RelocateToProgress uses parent rotation and stationary position.
    // A create without a progress clock stays frozen.
    let elevator_seed = (matches!(go_type, Some(11)) && transport_anchor.is_some())
        .then(|| transport_seed::from_create(&fields, position, orientation))
        .flatten();
    // A GameObject is placed by its quaternion. A mover's create block also seeds its flags
    // (`0x618c30`, merge mask `0x75a07dff`) and pitch (`0x7c6420`) before the model is built
    // (`0x613e10`), so a swimmer that streams in nose-down is drawn nose-down from the first frame.
    let placement = match kind {
        EntityKind::GameObject => gameobject_rotation(go_quat, orientation),
        _ => mover.map_or_else(
            || wire_yaw(orientation),
            |m| crate::app_core::movement::swim_body_rotation(orientation, m.flags, m.pitch),
        ),
    };
    // A living rider's tail is deck-local; keep its world position for a removed-deck fallback.
    let rider = matches!(kind, EntityKind::Unit | EntityKind::Player)
        .then_some(transport)
        .flatten()
        .map(|t| crate::app_core::movement::TransportRider {
            transport_guid: t.guid,
            local_pos: [t.pos.x, t.pos.y, t.pos.z],
            local_orientation: t.orientation,
        });
    // Warm the name cache as a unit streams in, the reference's timing: its create path
    // (`0x5fb880`) queries every creature (`0x60afb0`, at `0x60b157`), whose name is the record's
    // field 0 (`0x60934b`). Its `CreatureCache.wdb` answers without the wire; ours lasts a session.
    if matches!(kind, EntityKind::Unit | EntityKind::Player) {
        let _ = names.resolve(guid, net_commands);
        // A pet guid carries a pet number, so `resolve` asks the pet-name query; the template is
        // asked by the descriptor's entry as well, the reference's cache key (`[[unit+8]+0xc]`).
        if let Some(entry) = fields.object_entry().filter(|&e| e != 0) {
            let _ = names.resolve_creature(entry, guid, net_commands);
        }
    }
    // Warm the template cache as a GameObject streams in: only the query carries its lockId,
    // which decides use-vs-cast on a right-click.
    if matches!(kind, EntityKind::GameObject) {
        // AC MotionTransport::CreateMoTrans uses the spawn counter, not entry, in its GUID.
        if let Some(entry) = fields.object_entry() {
            go_templates.request_entry(entry, guid, net_commands);
        }
        // Where and how big, under `RUST_LOG=wrathplus_app::net::objects=debug`.
        debug!(
            "gameobject spawn: entry {:?} display {display_id:?} type {go_type:?} \
             pos [{:.2}, {:.2}, {:.2}] scale {scale}",
            fields.object_entry(),
            position[0],
            position[1],
            position[2],
        );
    }
    if let Some(&e) = index.0.get(&guid) {
        // A re-create is a fresh snapshot: refresh identity and pose; `crate::motion` drops any
        // extrapolation and replaces the path.
        commands.entity(e).insert(net);
        // The set is replaced whole; a `SMSG_FORCE_*_SPEED_CHANGE` in the same tick lands on top,
        // since this insert applies before the next packet's handler (`apply::seam_tests`).
        if let Some(s) = speeds {
            commands.entity(e).insert(UnitSpeeds(s));
        }
        if let Some(anchor) = transport_anchor {
            commands.entity(e).insert(anchor);
        }
        if let Some(seed) = elevator_seed {
            commands.entity(e).insert(seed);
        }
        match rider {
            Some(r) => {
                commands.entity(e).insert(r);
            }
            None => {
                commands
                    .entity(e)
                    .remove::<crate::app_core::movement::TransportRider>();
            }
        }
        crate::app_core::movement::set_pose(transforms, e, position, placement);
        // The reference refreshes a live guid in place, notifying field watchers like any delta.
        merge_fields(commands, stores, edges, e, guid, fields);
    } else {
        // A transport spawns hidden: its create pose is the spawn point, not where it is in its
        // cycle; the transport tick unhides it at the first sampled pose.
        let visibility = if transport_anchor.is_some() {
            Visibility::Hidden
        } else {
            Visibility::default()
        };
        let mut entity = commands.spawn((
            Guid(guid),
            net,
            pose_transform(position, placement),
            visibility,
        ));
        // Chain-only visibility: the root renders nothing itself, and culling flips its
        // `Visibility` through `Mut` writes, which do not re-add the sweep row.
        entity.vis_chain_only();
        if let Some(s) = speeds {
            entity.insert(UnitSpeeds(s));
        }
        if let Some(anchor) = transport_anchor {
            entity.insert(anchor);
        }
        if let Some(seed) = elevator_seed {
            entity.insert(seed);
        }
        if let Some(r) = rider {
            entity.insert(r);
        }
        // The seed is not merged: the reference does not notify on create.
        entity.insert(ObjectStore(fields));
        index.0.insert(guid, entity.id());
    }
    if transport.is_some() && matches!(kind, EntityKind::Unit | EntityKind::Player) {
        if let Some(&entity) = index.0.get(&guid) {
            transport_seed::remember_world_pose(commands, entity, position, orientation);
        }
    }
}

/// A descriptor-only item or container create: an indexed entity with a store and no pose or
/// model; a re-create merges like a values delta.
fn item_create(
    guid: u64,
    container: bool,
    fields: ObjectFields,
    commands: &mut Commands,
    index: &mut GuidIndex,
    stores: &mut Query<&mut ObjectStore>,
    edges: &mut MessageWriter<FieldChanged>,
    items: &mut Items,
) {
    debug!("net: item create {guid:#x} (container: {container})");
    if let Some(&e) = index.0.get(&guid) {
        merge_fields(commands, stores, edges, e, guid, fields);
    } else {
        let e = crate::items::spawn_item(commands, index, guid, fields, container);
        // Replay the enchant times queued while it was not held (`0x5ebde0`), as of now.
        let queued = items.take_enchant_times(guid);
        if !queued.is_empty() {
            let mut countdowns = crate::items::Countdowns::default();
            for (slot, seconds) in queued {
                if !countdowns.set_enchant(slot, seconds) {
                    debug!("enchant timer: queued slot {slot} for {guid:#x} is past the seventh — refused");
                }
            }
            commands.entity(e).insert(countdowns);
        }
    }
}

/// An `SMSG_UPDATE_OBJECT` values block, merged into any object's store; an unknown guid drops.
fn object_values(
    guid: u64,
    fields: ObjectFields,
    commands: &mut Commands,
    index: &GuidIndex,
    stores: &mut Query<&mut ObjectStore>,
    edges: &mut MessageWriter<FieldChanged>,
) {
    if let Some(&e) = index.0.get(&guid) {
        merge_fields(commands, stores, edges, e, guid, fields);
    }
}

/// `SMSG_DESTROY_OBJECT`: corpse decay ahead of respawn, or a despawn.
fn object_destroyed(guid: u64, commands: &mut Commands, index: &mut GuidIndex) {
    transport_seed::detach_removed_riders(commands, guid);
    // The object is freed now and its model fades out, as the reference's OnDeactivate
    // (`0x6145e0`, called by `0x464920` on DESTROY and OUT_OF_RANGE alike) hands the model to the
    // `SWModelFadeout` scheduler (`0x672df0`). The guid leaves the index at once ([`tear_down`]).
    //
    // A pinned object only gets its pending-destroy bit and is freed when the last pin drops
    // (`0x464920`, `0x468410`). Our one pin is an announced `SMSG_GAMEOBJECT_DESPAWN_ANIM`, whose
    // end runs the deferred teardown (`go_anim`'s `release_despawn_pin`).
    if let Some(e) = index.0.remove(&guid) {
        // An item has no model to fade, so it goes now with its countdown cells.
        if guid::is_item(guid) {
            commands.entity(e).despawn();
            return;
        }
        commands.queue(move |world: &mut bevy::ecs::world::World| {
            if world
                .get::<crate::app_core::anim::DespawnAnimAnnounced>(e)
                .is_some()
            {
                if let Ok(mut ent) = world.get_entity_mut(e) {
                    ent.insert(crate::app_core::anim::PendingDestroy);
                }
            } else if let Ok(ent) = world.get_entity_mut(e) {
                tear_down(ent);
            }
        });
    }
}

/// The `OutOfRange` block: the object still exists, out of our range.
fn objects_removed(guids: Vec<u64>, commands: &mut Commands, index: &mut GuidIndex) {
    // Faded, not popped: OUT_OF_RANGE reaches `0x464920` like DESTROY, and the object ends now
    // while its model fades ([`tear_down`]).
    for g in guids {
        transport_seed::detach_removed_riders(commands, g);
        if let Some(e) = index.0.remove(&g) {
            commands.entity(e).queue(tear_down);
        }
    }
}

/// An object's teardown, the reference's `0x464920`: the object is freed and its detached model
/// fades out ([`DespawnFade`]); its OnDeactivate (`0x5fbb60` → `0x493910`) clears the selection
/// and sends `CMSG_SET_SELECTION 0` at once.
///
/// The model stays on the same entity, so removing its [`Guid`] is what ends the object for every
/// consumer; the [`NetEntity`] and [`ObjectStore`] stay so the model draws out (a decaying corpse
/// stays lying down). The caller has already removed the guid from the [`GuidIndex`], so a
/// same-tick re-create spawns a fresh object beside the fading model.
pub fn tear_down(mut ent: EntityWorldMut) {
    ent.remove::<Guid>();
    ent.insert(DespawnFade::default());
}

/// `SMSG_GAMEOBJECT_QUERY_RESPONSE`: cached, with the lockId resolved from its type's `data[]`
/// slot for use-vs-cast.
fn gameobject_info(
    entry: u32,
    type_id: u32,
    display_id: u32,
    name: String,
    data: &[i32; 24],
    go_templates: &mut GameObjectTemplates,
) {
    debug!("net: gameobject template {entry} type {type_id} display {display_id} {name:?}");
    go_templates.insert(entry, type_id, name, data);
}

/// Merges a descriptor delta into an object's store and reports its [`FieldChanged`] edges. An
/// indexed entity with no store should not happen, and is seeded from the delta.
fn merge_fields(
    commands: &mut Commands,
    stores: &mut Query<&mut ObjectStore>,
    edges: &mut MessageWriter<FieldChanged>,
    entity: Entity,
    guid: u64,
    delta: ObjectFields,
) {
    if let Ok(mut s) = stores.get_mut(entity) {
        merge_store_fields(&mut s.0, delta, entity, guid, |e| {
            edges.write(e);
        });
    } else {
        warn!(
            "net: values delta for {guid:#x} found no store on its entity — seeding from the delta"
        );
        commands.entity(entity).insert(ObjectStore(delta));
    }
}

/// `SMSG_FORCE_*_SPEED_CHANGE`: applied to the mover's speeds, and for our own guid forwarded to
/// the controller, which owes the ack. A foreign mover is never acked, which the server treats as
/// an error.
fn force_speed_change(
    guid: u64,
    kind: SpeedKind,
    counter: u32,
    speed: f32,
    index: &GuidIndex,
    speeds: &mut Query<&mut UnitSpeeds>,
    self_guid: &SelfGuid,
    speed_changes: &mut MessageWriter<SpeedChangeMessage>,
) {
    let applied = set_speed(guid, kind, speed, index, speeds);
    if self_guid.0 == Some(guid) {
        info!("net: force {kind:?} speed change -> {speed} yd/s (counter {counter})");
        // Should be unreachable; the ack still goes, so it warns.
        if !applied {
            warn!(
                "net: force {kind:?} speed change for OUR mover {guid:#x} landed nowhere \
                 (no live speed set) — we still ack, so the server and we now disagree"
            );
        }
        speed_changes.write(SpeedChangeMessage);
    } else {
        debug!("net: force speed change for foreign mover {guid:#x} — applied {applied}, no ack");
    }
}

/// The SPLINE_SET and MOVE_SET families: an observed unit's speed, no ack. MOVE_SET's pose
/// arrives as its own `UnitMove`.
fn speed_changed(
    guid: u64,
    kind: SpeedKind,
    speed: f32,
    index: &GuidIndex,
    speeds: &mut Query<&mut UnitSpeeds>,
) {
    if !set_speed(guid, kind, speed, index, speeds) {
        // A speed broadcast can outrun the create block, which carries the same set.
        debug!("net: {kind:?} speed change for untracked mover {guid:#x} — nothing to write");
    }
}

/// Our corpse streaming in: latch its guid for the reclaim send. The reference's latch
/// `[0xb4e328]` has one writer, `0x4920d0`, gated on `CORPSE_FIELD_OWNER == me` and
/// `CORPSE_FIELD_FLAGS` bit 0 clear, so bones never arm it; `RetrieveCorpse` sends whatever is in
/// the latch.
fn note_corpse(
    guid: u64,
    kind: EntityKind,
    fields: &ObjectFields,
    self_guid: &SelfGuid,
    death_net: &mut DeathNet,
) {
    if kind == EntityKind::Corpse
        && fields.corpse_owner() == self_guid.0
        && !fields.corpse_is_bones()
    {
        death_net.corpse_guid = Some(guid);
    }
}

/// The same latch, re-asked when `CORPSE_FIELD_FLAGS` changes under a live guid, as the
/// reference's `FLAGS` mirror handler `0x5d6d60` does, so an in-place flip to bones drops it.
fn recheck_corpse(
    guid: u64,
    fields: &ObjectFields,
    self_guid: &SelfGuid,
    death_net: &mut DeathNet,
) {
    // A delta carries only what changed: an absent FLAGS field is not a clear bit.
    let Some(flags) = fields.corpse_flags_present() else {
        return;
    };
    let mine = fields.corpse_owner() == self_guid.0 || death_net.corpse_guid == Some(guid);
    if !mine {
        return;
    }
    if flags & 0x01 != 0 {
        if death_net.corpse_guid == Some(guid) {
            death_net.corpse_guid = None;
        }
    } else if fields.corpse_owner() == self_guid.0 {
        death_net.corpse_guid = Some(guid);
    }
}

/// The corpse-to-bones swap destroys the corpse object; a stale guid must not ride a reclaim.
fn forget_corpse(guid: u64, death_net: &mut DeathNet) {
    if death_net.corpse_guid == Some(guid) {
        death_net.corpse_guid = None;
    }
}
