//! `InitiateTrade`'s local gates (`0x522600`), which refuse before `CMSG_INITIATE_TRADE` goes out.

use crate::app_core::group::GroupState;
use crate::app_core::session::ObjectStore;
use crate::app_core::state::Selection;
use crate::net::Objects;

/// The partner `token` names, or the refusal `0x522600` raises (`None`: nothing, silently).
/// Labelled: the faction gate (`0x729b30`, `ERR_PLAYER_WRONG_FACTION`) and the send's in-progress
/// and one-second gates (`0x703cf0`) are left to the server's `TradeStatus`.
pub(super) fn initiate_target(
    token: &str,
    me: Option<u64>,
    own: Option<&ObjectStore>,
    objects: &Objects,
    selection: &Selection,
    group: &GroupState,
) -> Result<u64, Option<&'static str>> {
    // "player" names us; the UnitPopup resolver the other verbs share answers only others.
    let guid = if token == "player" {
        me
    } else {
        crate::target::unit_tokens::player_token_guid(token, selection, group)
    };
    let guid = guid.ok_or(None)?;
    // Both sides must stream as player objects (`0x4d4db0`, mask `0x10`).
    let own = own.ok_or(None)?;
    let streamed = wrathplus_protocol::guid::is_player(guid) && objects.object(guid).is_some();
    if Some(guid) != me && !streamed {
        return Err(None);
    }
    // `0x87` at `0x522683`, from the player's own state (inferred: dead or a ghost).
    if own.0.is_dead_or_ghost() {
        return Err(Some("ERR_PLAYER_DEAD"));
    }
    // `0x270` at `0x5226a6`.
    if Some(guid) == me {
        return Err(Some("ERR_TRADE_SELF"));
    }
    Ok(guid)
}
