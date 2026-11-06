//! The unit a character-sheet verb's token names, through build 12340's resolver (`0x60c1f0`):
//! the active player, our pet, another held unit, or nobody.

use super::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatUnit {
    Player,
    Pet,
    /// A held unit that is neither; `is_player` is the PLAYER typemask some verbs test.
    Other {
        is_player: bool,
    },
    Nobody,
}

impl Model {
    /// `0x60c1f0`'s unit (the resolver's guid, then the held object), sorted as the verbs test it:
    /// the active player (`0x4cee50`), our pet, another held unit, or nobody.
    pub(crate) fn stat_unit(&self, token: &str) -> StatUnit {
        let Some(guid) = self.unit_guid(token) else {
            return StatUnit::Nobody;
        };
        if guid == self.local_guid() {
            StatUnit::Player
        } else if guid == self.unit_guids.pet {
            StatUnit::Pet
        } else {
            match self.unit_by_guid(guid) {
                Some(u) if u.has_object => StatUnit::Other {
                    is_player: u.is_player,
                },
                _ => StatUnit::Nobody,
            }
        }
    }
}
