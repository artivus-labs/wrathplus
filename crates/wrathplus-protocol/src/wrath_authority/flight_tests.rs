use super::*;

// Login mid-air on a flying mount: the self create already carries FLYING|CAN_FLY (+pitch), then
// SMSG_MOVE_SET_CAN_FLY is ACKed from that pose; the ACK keeps the bit and the flier hovers.
#[test]
fn a_login_airborne_flier_acks_the_can_fly_grant_and_keeps_flying() {
    use crate::wrath_ground::{MOVE_CAN_FLY, MOVE_FLYING};
    let flier = OnFootPose {
        flags: MOVE_CAN_FLY | MOVE_FLYING,
        position: [1.0, 2.0, 30.0],
        orientation: 0.5,
        fall_time: 0,
        fall: None,
        pitch: Some(0.0),
        transport: None,
    };
    let grant = encode_mode_ack(0x0500_0001, 1, flier, 9, ModeOrderKind::CanFly, true).unwrap();
    let mut rest = &grant[..grant.len() - 4];
    let _ = read_packed_guid(&mut rest).unwrap();
    let mut framed = Vec::new();
    write_packed_guid(0x0500_0001, &mut framed).unwrap();
    framed.extend(&rest[4..]);
    let info = OutboundMovement::from_bytes(&framed).unwrap().movement;
    assert_eq!(info.flags, MOVE_CAN_FLY | MOVE_FLYING);
    assert_eq!(info.pitch, Some(0.0));
    // A FLYING pose without the grant still cannot be reported (ground checks stay).
    let no_grant = OnFootPose {
        flags: MOVE_FLYING,
        ..flier
    };
    assert!(encode_force_root_on_foot_ack(0x0500_0001, 2, no_grant.halted(), 9, true).is_err());
}
