//! Shared mover speed-slot projection.
use super::*;

/// Writes one slot of a mover's [`UnitSpeeds`]; `false` when there is none (an untracked guid,
/// or a create with no movement block). vmangos sends a self create and a speed change back to
/// back (`HandleMoveWorldportAckOpcode`), and wire order holds as each handler applies in turn.
pub(super) fn set_speed(
    guid: u64,
    kind: SpeedKind,
    speed: f32,
    index: &GuidIndex,
    speeds: &mut Query<&mut UnitSpeeds>,
) -> bool {
    let Some(mut s) = index.0.get(&guid).and_then(|&e| speeds.get_mut(e).ok()) else {
        return false;
    };
    let s = &mut s.0;
    match kind {
        SpeedKind::Walk => s.walk = speed,
        SpeedKind::Run => s.run = speed,
        SpeedKind::RunBack => s.run_back = speed,
        SpeedKind::Swim => s.swim = speed,
        SpeedKind::SwimBack => s.swim_back = speed,
        SpeedKind::TurnRate => s.turn_rate = speed,
        SpeedKind::Flight => s.flight = speed,
        SpeedKind::FlightBack => s.flight_back = speed,
        SpeedKind::PitchRate => s.pitch_rate = speed,
    }
    true
}
