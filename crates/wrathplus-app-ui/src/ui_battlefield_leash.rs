//! The battlemaster window's leash: both list windows make the battlemaster 12340's interaction
//! target (`0x51fce0`), and the per-frame check `0x51fde0` closes it (`0x518d50`) once the NPC is
//! gone or the player is farther from it than the leash. Labelled: `0x51fde0`'s object-flag and
//! reaction tests (`[obj+0xbc] & 0x10000`, `0x729530`, `0x729b30`) are not modelled.

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

use crate::app_core::player::Player;
use crate::net::{GuidIndex, NetEntity, Objects};
use crate::ui_battlefield::Battlefield;

/// A player target's squared leash, stored whole (`0x51fd71`, `0xac8a68`).
const PLAYER_LEASH_SQ: f32 = 123.456_78;

/// `0x51fd8a`-`0x51fd98`: a unit's squared leash, its `UNIT_FIELD_COMBATREACH` plus 4.0
/// (`0x9e8d2c`), squared into the f32 at `0xbd0798`.
fn leash_sq(combat_reach: f32, is_player: bool) -> f32 {
    if is_player {
        PLAYER_LEASH_SQ
    } else {
        (combat_reach + 4.0) * (combat_reach + 4.0)
    }
}

/// `0x51fde0`: the window closes when the target is gone or beyond its leash, measured from the
/// target's position each frame (`0x4f61d0`).
fn breaks(player: Vec3, target: Option<(Vec3, f32)>) -> bool {
    target.is_none_or(|(at, leash_sq)| player.distance_squared(at) > leash_sq)
}

pub(crate) fn leash_battlemaster(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<Battlefield>,
    player: Res<Player>,
    objects: Objects,
    guids: Res<GuidIndex>,
    bodies: Query<&GlobalTransform, With<NetEntity>>,
) {
    let (Some(mut script), Some(guid)) = (script, state.leashed()) else {
        return;
    };
    let target = guids
        .0
        .get(&guid)
        .and_then(|e| bodies.get(*e).ok())
        .map(|tf| {
            let reach = objects.object(guid).map_or(1.5, |o| o.unit_combat_reach());
            let is_player = wrathplus_protocol::guid::is_player(guid);
            (tf.translation(), leash_sq(reach, is_player))
        });
    if breaks(player.pos, target) {
        state.close();
        script.fire_event("BATTLEFIELDS_CLOSED", vec![]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leash_is_the_npcs_combat_reach_plus_four_from_where_it_stands() {
        // Reach 1.5: 5.5 yd; reach 3.0: 7.0 yd, past the old fixed 5.5556.
        assert_eq!(leash_sq(1.5, false), 30.25);
        let npc = Vec3::new(10.0, 0.0, 0.0);
        assert!(!breaks(
            Vec3::new(16.9, 0.0, 0.0),
            Some((npc, leash_sq(3.0, false)))
        ));
        assert!(breaks(
            Vec3::new(17.1, 0.0, 0.0),
            Some((npc, leash_sq(3.0, false)))
        ));
        assert!(
            breaks(Vec3::ZERO, None),
            "a target no longer streamed closes it"
        );
        assert!(!breaks(
            Vec3::new(21.0, 0.0, 0.0),
            Some((npc, leash_sq(1.5, true)))
        ));
    }
}
