//! The social session: the friend list, the ignore list, `/who`, and the system lines they print.
//! The reference's `FriendList` (`0xc28168`) holds guids, not names: a row's name comes from the
//! name cache (`0x55f080`) when it is formatted, and the selection is a guid too.
//!
//! Every friend and ignore result prints to the chat frame, not the red error line: `HandleResult`
//! (`0x5acab0`) shows catalog ids `0x104..0x114` through `DisplayError` (`0x496720`), all of kind 0
//! in the catalog table (`0xb4b498`), the chat composer (`0x49a870`). Which result prints which key
//! is matched by name, vmangos `FriendsResult` to `ERR_FRIEND_*`/`ERR_IGNORE_*`; the reference's
//! own mapping is untraced.

use bevy::prelude::*;
use wrathplus_protocol::types::{friend_result, friend_status, FriendEntry, FriendStatusUpdate};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;

pub mod feed;
pub mod native_who;
pub mod player_names;
mod query;

pub(crate) use crate::app_core::interaction::SocialState;

/// The `GlobalStrings.lua` key a friend or ignore result prints, resolved at the feed. The
/// reference composes these lines engine-side; FrameXML never names the keys.
fn result_key(result: u8) -> Option<&'static str> {
    Some(match result {
        friend_result::DB_ERROR => "ERR_FRIEND_DB_ERROR",
        friend_result::LIST_FULL => "ERR_FRIEND_LIST_FULL",
        friend_result::ONLINE => "ERR_FRIEND_ONLINE_SS",
        friend_result::OFFLINE => "ERR_FRIEND_OFFLINE_S",
        friend_result::NOT_FOUND => "ERR_FRIEND_NOT_FOUND",
        friend_result::REMOVED => "ERR_FRIEND_REMOVED_S",
        friend_result::ADDED_ONLINE | friend_result::ADDED_OFFLINE => "ERR_FRIEND_ADDED_S",
        friend_result::ALREADY => "ERR_FRIEND_ALREADY_S",
        friend_result::SELF => "ERR_FRIEND_SELF",
        friend_result::ENEMY => "ERR_FRIEND_WRONG_FACTION",
        friend_result::IGNORE_FULL => "ERR_IGNORE_FULL",
        friend_result::IGNORE_SELF => "ERR_IGNORE_SELF",
        friend_result::IGNORE_NOT_FOUND => "ERR_IGNORE_NOT_FOUND",
        friend_result::IGNORE_ALREADY => "ERR_IGNORE_ALREADY_S",
        friend_result::IGNORE_ADDED => "ERR_IGNORE_ADDED_S",
        friend_result::IGNORE_REMOVED => "ERR_IGNORE_REMOVED_S",
        friend_result::IGNORE_AMBIGUOUS => "ERR_IGNORE_AMBIGUOUS",
        friend_result::UNKNOWN => "ERR_FRIEND_ERROR",
        // An unknown code shows nothing.
        _ => return None,
    })
}

/// How many times the subject's name fills a result's message, as the key's suffix names it: none
/// for a bare key, one for `_S`, two for `_SS` (the `|Hplayer:%s|h[%s]|h` link). Read off the key,
/// not the text, so a string not yet resolved never passes for one that takes no name.
fn name_pushes(key: &str) -> usize {
    if key.ends_with("_SS") {
        2
    } else if key.ends_with("_S") {
        1
    } else {
        0
    }
}

/// The key a friend row's away tag resolves through, the pair the chat frame prefixes a speaker's
/// name with: the status bits, DND tested before AFK (12340's `0x6b4307`, `0x6b4325`).
fn status_flag_key(status: u8) -> Option<&'static str> {
    if status & friend_status::DND != 0 {
        Some("CHAT_FLAG_DND")
    } else if status & friend_status::AFK != 0 {
        Some("CHAT_FLAG_AFK")
    } else {
        None
    }
}

/// The social packet handlers: the friend and ignore lists, the `/who` answer and the result
/// codes. The lines and the list events fire from the feed, which resolves the names they need.
pub(crate) mod net {
    use super::*;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use crate::net::NetHandlerApp;

    /// Register the handlers, one per kind, and the session-end listener.
    pub(crate) fn register(app: &mut App) {
        use SessionEventKind as K;
        app.net_handler(K::FriendList, on_friend_list)
            .net_handler(K::IgnoreList, on_ignore_list)
            .net_handler(K::FriendStatus, on_friend_status)
            .net_handler(K::WrathContactNotes, on_contact_notes)
            .net_handler(K::WhoResults, on_who)
            .net_handler(K::Disconnected, on_session_end)
            .net_handler(K::Connected, on_connected);
    }

    fn on_friend_list(In(ev): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if let SessionEvent::FriendList { friends } = ev {
            friend_list(&mut social, friends);
        }
    }

    fn on_ignore_list(In(ev): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if let SessionEvent::IgnoreList { guids } = ev {
            ignore_list(&mut social, guids);
        }
    }

    fn on_friend_status(In(ev): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if let SessionEvent::FriendStatus(update) = ev {
            friend_status(&mut social, update);
        }
    }

    /// The contact list's notes replace the set; an added friend's note patches it.
    fn on_contact_notes(In(ev): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if let SessionEvent::WrathContactNotes { notes, replace } = ev {
            if replace {
                social.notes.clear();
            }
            social.notes.extend(notes);
            social.friends_dirty = true;
        }
    }

    fn on_who(In(ev): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if let SessionEvent::WhoResults(results) = ev {
            who(&mut social, results);
        }
    }

    fn on_connected(In(event): In<SessionEvent>, mut social: ResMut<SocialState>) {
        if matches!(event, SessionEvent::Connected { .. }) {
            social.clear_session();
        }
    }

    /// A second handler on the session end, after the bridge's own teardown; `clear_session`, not
    /// `default()`, so the per-process sort chain survives.
    fn on_session_end(In(_): In<SessionEvent>, mut social: ResMut<SocialState>) {
        social.clear_session();
    }

    /// `SMSG_FRIEND_LIST`.
    pub(crate) fn friend_list(social: &mut SocialState, friends: Vec<FriendEntry>) {
        social.apply_friend_list(friends);
    }

    /// `SMSG_IGNORE_LIST`.
    pub(crate) fn ignore_list(social: &mut SocialState, guids: Vec<u64>) {
        social.apply_ignore_list(guids);
    }

    /// `SMSG_FRIEND_STATUS`.
    pub(crate) fn friend_status(social: &mut SocialState, update: FriendStatusUpdate) {
        social.apply_friend_status(update);
    }

    /// `SMSG_WHO`.
    pub(crate) fn who(social: &mut SocialState, results: wrathplus_protocol::types::WhoResults) {
        social.apply_who(results);
    }
}

/// The social window's session: the wire mirror, the VM feed, and the outbound intents.
pub struct UiSocialPlugin;

impl Plugin for UiSocialPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        app.init_resource::<SocialState>().add_systems(
            Update,
            (
                native_who::drain
                    .after(wrathplus_world::schedule::WorldStage::Net)
                    .before(UiFeed),
                feed::feed_social.in_set(UiFeed),
                player_names::feed_player_names.in_set(UiFeed),
                feed::drain_social.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::FriendOnline;

    fn status(result: u8, guid: u64) -> FriendStatusUpdate {
        FriendStatusUpdate {
            result,
            guid,
            online: None,
        }
    }

    #[test]
    fn add_and_remove_results_maintain_the_list() {
        let mut social = SocialState::default();
        social.apply_friend_status(FriendStatusUpdate {
            result: friend_result::ADDED_ONLINE,
            guid: 7,
            online: Some(FriendOnline {
                status: friend_status::ONLINE,
                area: 12,
                level: 60,
                class: 4,
            }),
        });
        assert_eq!(social.friends.len(), 1);
        assert_eq!(social.friends[0].level, 60);

        // The same add arriving twice must not duplicate the row.
        social.apply_friend_status(status(friend_result::ADDED_OFFLINE, 7));
        assert_eq!(social.friends.len(), 1);

        social.apply_friend_status(status(friend_result::REMOVED, 7));
        assert!(social.friends.is_empty());
    }

    /// Going offline writes only the online byte (12340's `0x6b761a` → `0x6b3d10`): the level, zone
    /// and class stay as last sent.
    #[test]
    fn presence_broadcasts_patch_the_row() {
        let mut social = SocialState::default();
        social.apply_friend_list(vec![FriendEntry {
            guid: 7,
            ..Default::default()
        }]);

        social.apply_friend_status(FriendStatusUpdate {
            result: friend_result::ONLINE,
            guid: 7,
            online: Some(FriendOnline {
                status: friend_status::AFK | 0x08,
                area: 1519,
                level: 42,
                class: 8,
            }),
        });
        assert_eq!(social.friends[0].status, friend_status::AFK | 0x08);
        assert_eq!(social.friends[0].area, 1519);

        social.apply_friend_status(status(friend_result::OFFLINE, 7));
        assert!(!social.friends[0].is_online());
        let f = &social.friends[0];
        assert_eq!((f.area, f.level, f.class), (1519, 42, 8));
        assert_eq!(f.status & 0x08, 0x08, "the RAF bit outlives the logout");
    }

    #[test]
    fn removing_the_selected_friend_clears_the_selection() {
        let mut social = SocialState::default();
        social.apply_friend_list(vec![FriendEntry {
            guid: 7,
            ..Default::default()
        }]);
        social.selected_friend = 7;
        social.apply_friend_status(status(friend_result::REMOVED, 7));
        assert_eq!(social.selected_friend, 0);
    }

    #[test]
    fn ignore_results_maintain_the_ignore_list() {
        let mut social = SocialState::default();
        assert!(!social.is_ignored(9));
        social.apply_friend_status(status(friend_result::IGNORE_ADDED, 9));
        assert!(social.is_ignored(9));
        assert!(!social.is_ignored(0), "guid 0 is never ignored");
        social.apply_friend_status(status(friend_result::IGNORE_REMOVED, 9));
        assert!(!social.is_ignored(9));
    }

    #[test]
    fn a_refusal_changes_no_state_but_prints() {
        let mut social = SocialState::default();
        social.apply_friend_status(status(friend_result::ALREADY, 7));
        assert!(social.friends.is_empty());
        assert_eq!(social.pending_lines.len(), 1);
    }

    /// The two ADDED codes share a key, so vmangos's 18 results fit the 17 catalog ids
    /// `0x104..0x114`.
    #[test]
    fn every_result_code_maps_to_a_line() {
        for result in 0x00..=0x11u8 {
            assert!(
                result_key(result).is_some(),
                "result {result:#04x} has no line"
            );
        }
        assert_eq!(
            result_key(friend_result::ADDED_ONLINE),
            result_key(friend_result::ADDED_OFFLINE),
        );
        assert_eq!(result_key(friend_result::UNKNOWN), Some("ERR_FRIEND_ERROR"));
        assert_eq!(result_key(0x77), None, "an unknown code shows nothing");
        // Both read "Player not found." in enUS and differ in other locales.
        assert_eq!(
            result_key(friend_result::NOT_FOUND),
            Some("ERR_FRIEND_NOT_FOUND")
        );
        assert_eq!(
            result_key(friend_result::IGNORE_NOT_FOUND),
            Some("ERR_IGNORE_NOT_FOUND")
        );
    }

    #[test]
    fn a_logout_keeps_the_sort_chain_and_drops_everything_else() {
        let mut social = SocialState::default();
        social.apply_friend_list(vec![FriendEntry {
            guid: 7,
            ..Default::default()
        }]);
        social.apply_ignore_list(vec![9]);
        social.who_to_ui = true;
        social.who_sort.promote("level");
        social.who_sort.promote("level"); // descending
        let chain = social.who_sort.clone();

        social.clear_session();
        assert!(social.friends.is_empty());
        assert!(social.ignores.is_empty());
        assert!(social.who.is_empty());
        assert!(!social.who_to_ui, "the frame is closed after a logout");
        assert_eq!(social.who_sort, chain, "the sort chain is process state");
        assert_ne!(
            social.who_sort,
            wrathplus_ui::script::WhoSortChain::default(),
            "and the assertion above only means something if it is not the seeded chain"
        );
    }

    #[test]
    fn the_key_suffix_is_the_name_arity() {
        assert_eq!(name_pushes("ERR_FRIEND_ONLINE_SS"), 2);
        assert_eq!(name_pushes("ERR_FRIEND_ADDED_S"), 1);
        assert_eq!(name_pushes("ERR_FRIEND_SELF"), 0, "not an _S despite the S");
    }

    #[test]
    fn every_key_resolves_and_its_arity_matches_the_string() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");

        let mut keys: Vec<&str> = (0x00..=0x11u8).filter_map(result_key).collect();
        keys.extend(
            [
                status_flag_key(friend_status::AFK),
                status_flag_key(friend_status::DND),
            ]
            .map(Option::unwrap),
        );
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 19, "17 result rows plus the two away tags");

        for key in keys {
            let text: String = s.lua().globals().get(key).expect(key);
            assert!(!text.is_empty(), "{key} resolves empty");
            // The away tags are a row field, not a `DisplayError` message: no catalog row.
            if !key.starts_with("CHAT_FLAG_") {
                assert!(
                    wrathplus_ui::messages::by_key(key).is_some(),
                    "{key} is not a catalog row, so its surface and sound would be a guess"
                );
                assert_eq!(
                    text.matches("%s").count(),
                    name_pushes(key),
                    "{key}: the string's holes vs the arity its suffix promises"
                );
            }
        }
        assert_eq!(
            wrathplus_ui::messages::by_key("ERR_FRIEND_ONLINE_SS").and_then(|r| r.sound),
            Some("FRIENDJOINGAME"),
            "the cue the chat-log path was dropping"
        );
    }
}
