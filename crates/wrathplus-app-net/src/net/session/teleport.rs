use bevy::prelude::MessageWriter;

use crate::net::{SelfGuid, TeleportMessage};

/// `MSG_MOVE_TELEPORT_ACK`: only our own is forwarded to the controller, whose ack resumes our
/// movement.
pub(super) fn teleport(
    guid: u64,
    counter: u32,
    position: [f32; 3],
    orientation: f32,
    transport: Option<wrathplus_protocol::TransportPose>,
    self_guid: &SelfGuid,
    teleports: &mut MessageWriter<TeleportMessage>,
) {
    if self_guid.0 == Some(guid) {
        teleports.write(TeleportMessage {
            guid,
            counter,
            position,
            orientation,
            transport,
        });
    }
}
