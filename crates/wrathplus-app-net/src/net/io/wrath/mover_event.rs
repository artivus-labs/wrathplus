use wrathplus_protocol::wrath_authority::{self, MoverOrder};
use wrathplus_protocol::SessionEvent;

/// The shared event the controller applies for a mover order, where it models the state: every
/// speed, water walk, feather fall, hover, can-fly, knockback and the near-teleport snap. Gravity
/// and collision height are acknowledged without one.
pub(crate) fn mover_order_event(order: &MoverOrder) -> Option<SessionEvent> {
    use wrath_authority::{ModeOrderKind, SpeedOrderKind};
    use wrathplus_protocol::types::{JumpInfo, MoveMode, SpeedKind};
    Some(match *order {
        MoverOrder::Speed {
            kind,
            mover_guid,
            counter,
            speed,
        } => SessionEvent::ForceSpeedChange {
            guid: mover_guid,
            kind: match kind {
                SpeedOrderKind::Walk => SpeedKind::Walk,
                SpeedOrderKind::Run => SpeedKind::Run,
                SpeedOrderKind::RunBack => SpeedKind::RunBack,
                SpeedOrderKind::Swim => SpeedKind::Swim,
                SpeedOrderKind::SwimBack => SpeedKind::SwimBack,
                SpeedOrderKind::TurnRate => SpeedKind::TurnRate,
                SpeedOrderKind::Flight => SpeedKind::Flight,
                SpeedOrderKind::FlightBack => SpeedKind::FlightBack,
                SpeedOrderKind::PitchRate => SpeedKind::PitchRate,
            },
            counter,
            speed,
        },
        MoverOrder::Mode {
            kind,
            apply,
            mover_guid,
            counter,
        } => SessionEvent::MoveMode {
            guid: mover_guid,
            counter,
            mode: match kind {
                ModeOrderKind::WaterWalk => MoveMode::WaterWalk,
                ModeOrderKind::FeatherFall => MoveMode::FeatherFall,
                ModeOrderKind::Hover => MoveMode::Hover,
                ModeOrderKind::CanFly => MoveMode::CanFly,
                ModeOrderKind::DisableGravity => return None,
            },
            apply,
        },
        MoverOrder::KnockBack {
            mover_guid,
            counter,
            cos_angle,
            sin_angle,
            horizontal_speed,
            vertical_speed,
        } => SessionEvent::KnockBack {
            guid: mover_guid,
            counter,
            launch: JumpInfo {
                zspeed: vertical_speed,
                cos_angle,
                sin_angle,
                xy_speed: horizontal_speed,
            },
        },
        MoverOrder::TeleportNear {
            mover_guid,
            counter,
            position,
            orientation,
            transport,
        } => SessionEvent::Teleport {
            guid: mover_guid,
            counter,
            position,
            orientation,
            transport: transport.map(|t| wrathplus_protocol::TransportPose {
                guid: t.guid,
                pos: wrathplus_protocol::wire::Vector3d {
                    x: t.position[0],
                    y: t.position[1],
                    z: t.position[2],
                },
                orientation: t.orientation,
            }),
        },
        MoverOrder::CollisionHeight { .. } => return None,
    })
}
