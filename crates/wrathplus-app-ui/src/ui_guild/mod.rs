//! The guild windows: [`feed`] turns the guild session ([`crate::guild`], which mirrors the guild
//! packets) into the snapshot the guild windows read, and drains their outbound verbs. The roster
//! carries no guild or rank names: those come only from `SMSG_GUILD_QUERY_RESPONSE`, cached by
//! guild id and filled lazily, as in the reference.

use bevy::prelude::*;

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;

mod feed;
pub(crate) mod wrath;
pub(crate) use wrath::NativeGuild;

pub(crate) use crate::guild::{
    last_online, net, GuildMemberNotify, GuildShowOffline, GuildState, Identity, RosterRow,
    RosterUpdate, ROSTER_REQUEST_THROTTLE_SECS,
};
#[cfg(test)]
pub(crate) use crate::guild::{unit_guild, unit_guild_name};

/// The guild feed's system set, so the chat drain can run after the guild events fire.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GuildFeed;

/// The guild windows' session: the wire mirror, the VM feed, and the outbound intents.
pub struct UiGuildPlugin;

/// `guildMemberNotify`'s change callback, conjunct 2 of the sign-on/sign-off line's condition, and
/// `guildShowOffline`'s (`0x5ca870`): store, re-sort and repaint the roster.
pub fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut notify: ResMut<GuildMemberNotify>,
    mut offline: ResMut<GuildShowOffline>,
    mut guild: ResMut<GuildState>,
) {
    if ev.is("guildMemberNotify") {
        notify.0 = ev.flag();
    } else if ev.is("guildShowOffline") {
        offline.0 = ev.flag();
        guild.note_roster_update(RosterUpdate::Applied);
    }
}

/// The CVar rows this feature owns ([`crate::cvars::RegisterCvars`]), each with the
/// reference's register site and its default's standing ([`crate::cvars::Registered`]).
pub(crate) const CVARS: &[crate::cvars::Registered] = {
    use crate::cvars::same;
    &[
        // `guildMemberNotify` (`0x5e24c7`, "0" `0x82e570`, record `0xc4d3c4`): guildmate log on/off
        // lines, read only in `SMSG_GUILD_EVENT`'s handler. The knob is
        // [`GuildMemberNotify`].
        same("guildMemberNotify", "0"),
        // `guildShowOffline` (`0x51e1e2`, "1"): `Get/SetGuildRosterShowOffline`; [`GuildShowOffline`].
        same("guildShowOffline", "1"),
    ]
};

impl Plugin for UiGuildPlugin {
    fn build(&self, app: &mut App) {
        crate::cvars::RegisterCvars::register_cvars(app, CVARS);
        net::register(app);
        wrath::register(app);
        app.add_observer(on_cvar);
        crate::query_cache::register::<GuildState>(app);
        app.init_resource::<GuildState>()
            .init_resource::<GuildMemberNotify>()
            .init_resource::<GuildShowOffline>()
            .add_systems(
                Update,
                (
                    feed::feed_guild.in_set(UiFeed).in_set(GuildFeed),
                    feed::drain_guild.after(UiInput),
                )
                    // Never against the boot VM: the feed's edge memo survives the entry load, so
                    // an edge spent on a VM with no frames, such as the login MOTD, is lost.
                    .run_if(crate::app_core::ui_state::ingame_ui_up),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::interaction::SocialState;
    use crate::net::{ClientCommand, NetCommands};
    use wrathplus_formats::GuildEmblem;
    use wrathplus_protocol::types::{guild_command, guild_command_error, guild_presence};
    use wrathplus_protocol::types::{
        guild_event, GuildCommandResult, GuildEventNotice, GuildQueryResponse, GuildRoster,
        GuildRosterMember, GUILD_RANKS_MAX_COUNT,
    };
    use wrathplus_protocol::ObjectFields;

    fn identity(name: &str, ranks: &[&str]) -> Identity {
        let mut rank_names: [String; GUILD_RANKS_MAX_COUNT] = Default::default();
        for (slot, name) in rank_names.iter_mut().zip(ranks) {
            *slot = (*name).to_string();
        }
        Identity {
            name: name.to_string(),
            rank_names,
            emblem: GuildEmblem::default(),
        }
    }

    fn member(guid: u64, name: &str, presence: u8) -> GuildRosterMember {
        GuildRosterMember {
            guid,
            presence,
            name: name.to_string(),
            level: 60,
            ..Default::default()
        }
    }

    fn event(event: u8, params: &[&str], guid: Option<u64>) -> GuildEventNotice {
        GuildEventNotice {
            event,
            params: params.iter().map(|p| (*p).to_string()).collect(),
            guid,
        }
    }

    #[test]
    fn the_signon_condition_is_all_four_conjuncts() {
        let mut social = SocialState::default();
        crate::ui_social::net::friend_list(
            &mut social,
            vec![wrathplus_protocol::types::FriendEntry {
                guid: 7,
                ..Default::default()
            }],
        );
        crate::ui_social::net::ignore_list(&mut social, vec![9]);
        let on = GuildMemberNotify(true);
        let off = GuildMemberNotify(false);
        let me = Some(1);

        // 2: the CVar, off by default.
        assert!(
            !net::announce_signon(&social, &off, me, Some(5)),
            "guildMemberNotify off silences the whole family"
        );
        assert!(net::announce_signon(&social, &on, me, Some(5)));

        // 3: not us; vmangos sends the sign-on to the signer too (`Guild.cpp:651-656`).
        assert!(
            !net::announce_signon(&social, &on, me, Some(1)),
            "your own sign-on is not announced to you"
        );

        // 4: not a friend, whose `SMSG_FRIEND_STATUS` prints the same line.
        assert!(
            !net::announce_signon(&social, &on, me, Some(7)),
            "a guildmate who is also a friend is announced by the friend path, not twice"
        );

        // An ignored guildmate is announced: the ignore list plays no part.
        assert!(
            social.is_ignored(9),
            "the fixture's ignore really is an ignore"
        );
        assert!(
            net::announce_signon(&social, &on, me, Some(9)),
            "the reference announces an ignored guildmate — 0x5ae810 is not the ignore check"
        );

        // 1: a local player, and the pair's guid; either missing is the silent exit.
        assert!(!net::announce_signon(&social, &on, None, Some(5)));
        assert!(!net::announce_signon(&social, &on, me, None));
    }

    #[test]
    fn show_offline_changes_the_order_not_the_membership() {
        let mut guild = GuildState::default();
        guild.apply_roster(GuildRoster {
            members: vec![
                member(1, "Zed", guild_presence::ONLINE),
                member(2, "Gone", guild_presence::OFFLINE),
                member(3, "Away", guild_presence::ONLINE | guild_presence::AFK),
            ],
            ..Default::default()
        });

        let rows = feed::display_rows(&guild, false, None, None, &|_| String::new());
        assert_eq!(rows.len(), 3, "every member, always");
        assert!(
            rows[..2].iter().all(|r| r.info.online),
            "the online tally's rows lead"
        );
        assert!(!rows[2].info.online);
        let rows = feed::display_rows(&guild, true, None, None, &|_| String::new());
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn wrath_identity_misses_send_native_query_once_and_mark_pending() {
        let (commands, rx) = net_commands();
        let guild = GuildState::default();

        assert!(guild.resolve_identity(7, &commands).is_none());
        assert!(guild.resolve_identity(7, &commands).is_none());
        assert!(matches!(
            rx.try_recv(),
            Ok(ClientCommand::GuildQuery { guild_id: 7 })
        ));
        assert!(
            rx.try_recv().is_err(),
            "pending identity is queried only once"
        );
        assert!(guild.identities.is_pending(7));
    }

    #[test]
    fn an_empty_query_name_is_a_negative_not_a_blank_guild() {
        let (commands, rx) = net_commands();
        let mut guild = GuildState::default();

        assert!(guild.resolve_identity(7, &commands).is_none());
        assert!(guild.resolve_identity(7, &commands).is_none());
        assert_eq!(rx.try_iter().count(), 1, "asked exactly once");

        guild.apply_query_response(GuildQueryResponse {
            guild_id: 7,
            name: String::new(),
            ..Default::default()
        });
        assert!(guild.identities.answered(7), "cached as a negative");
        assert!(!guild.identities.is_pending(7), "and no longer in flight");
        assert!(
            guild.resolve_identity(7, &commands).is_none(),
            "an empty name is not a guild"
        );
        assert_eq!(rx.try_iter().count(), 0, "and is never re-asked");
    }

    #[test]
    fn a_guild_event_asks_again_and_a_local_resort_does_not() {
        let mut guild = GuildState::default();

        guild.apply_event(&event(guild_event::JOINED, &["Furor"], None));
        assert_eq!(guild.roster_event.take(), Some(RosterUpdate::Stale));
        assert!(RosterUpdate::Stale.arg1(), "the pane re-requests");

        guild.sort_by("level");
        assert_eq!(guild.roster_event.take(), Some(RosterUpdate::Applied));
        assert!(
            !RosterUpdate::Applied.arg1(),
            "a column click must never ask the server for a roster"
        );

        guild.apply_roster(GuildRoster::default());
        assert_eq!(
            guild.roster_event.take(),
            Some(RosterUpdate::Applied),
            "the roster we just applied is the one we have — asking for it again is the loop"
        );

        // Both in one frame: the re-request subsumes the repaint.
        guild.apply_roster(GuildRoster::default());
        guild.apply_event(&event(guild_event::PROMOTION, &["A", "B", "Officer"], None));
        assert_eq!(guild.roster_event.take(), Some(RosterUpdate::Stale));
    }

    #[test]
    fn signing_on_and_off_does_not_re_request() {
        let mut guild = GuildState::default();
        for ev in [guild_event::SIGNED_ON, guild_event::SIGNED_OFF] {
            guild.roster_event = None;
            guild.apply_event(&event(ev, &["Tigole"], Some(9)));
            assert_eq!(guild.roster_event, None, "event {ev:#04x}");
        }
        // The case just below them does.
        guild.apply_event(&event(guild_event::UPDATE_ROSTER, &[], None));
        assert_eq!(guild.roster_event, Some(RosterUpdate::Stale));
    }

    #[test]
    fn the_stale_edge_clears_the_request_throttle() {
        let mut guild = GuildState {
            roster_allowed_at: 12_345.0,
            ..Default::default()
        };
        guild.note_roster_update(RosterUpdate::Applied);
        assert_eq!(guild.roster_allowed_at, 12_345.0, "a repaint does not");
        guild.note_roster_update(RosterUpdate::Stale);
        assert_eq!(guild.roster_allowed_at, 0.0);
    }

    #[test]
    fn most_command_results_do_not_move_the_roster() {
        let mut guild = GuildState::default();
        guild.apply_command_result(&GuildCommandResult {
            command: guild_command::INVITE,
            name: "Kaplan".into(),
            result: guild_command_error::PERMISSIONS,
        });
        assert_eq!(guild.roster_event, None);

        guild.apply_command_result(&GuildCommandResult {
            command: guild_command::UNK19,
            name: String::new(),
            result: guild_command_error::PLAYER_NO_MORE_IN_GUILD,
        });
        assert_eq!(guild.roster_event, Some(RosterUpdate::Stale));
    }

    #[test]
    fn leaving_the_guild_drops_the_roster_but_not_the_identities() {
        let mut guild = GuildState::default();
        guild
            .identities
            .insert(7, Some(identity("Legacy", &["GM"])));
        assert!(guild.mirror_self(7, 0), "joined");
        guild.apply_roster(GuildRoster {
            motd: "Raid at eight".into(),
            rank_rights: vec![0xffff, 0x3],
            members: vec![member(1, "Tigole", guild_presence::ONLINE)],
            ..Default::default()
        });
        guild.selection = 1;
        assert!(guild.in_guild());

        assert!(guild.mirror_self(0, 0), "left");
        assert!(!guild.in_guild());
        assert!(guild.members.is_empty());
        assert!(guild.motd.is_empty());
        assert_eq!(guild.selection, 0);
        assert!(guild.identities.answered(7), "identities survive");

        assert!(!guild.mirror_self(0, 0), "no edge when nothing moved");
    }

    /// At login the MOTD event arrives before our descriptor names the guild.
    #[test]
    fn learning_our_own_guild_id_is_not_leaving_a_guild() {
        let mut guild = GuildState::default();
        guild.apply_event(&event(
            guild_event::MOTD,
            &["Raid Wednesday at eight."],
            None,
        ));
        assert!(!guild.in_guild(), "the descriptor has not said yet");

        assert!(
            guild.mirror_self(1, 3),
            "the descriptor finally names the guild"
        );
        assert_eq!(
            guild.motd, "Raid Wednesday at eight.",
            "the MOTD is about the guild we have just been told we are in"
        );
        assert_eq!(
            guild.roster_event,
            Some(RosterUpdate::Stale),
            "the pane still learns the snapshot moved — the MOTD packet's own stale signal, \
             which outranks the `Applied` this edge notes"
        );
    }

    #[test]
    fn own_rights_index_through_the_descriptor_rank() {
        let mut guild = GuildState::default();
        guild.mirror_self(7, 1);
        assert_eq!(guild.own_rights(), 0, "no roster yet");
        guild.apply_roster(GuildRoster {
            rank_rights: vec![0xffff, 0x00ff, 0x000f],
            ..Default::default()
        });
        assert_eq!(guild.own_rights(), 0x00ff);
        assert_eq!(guild.rank_rights.len(), 3);
        guild.mirror_self(7, 9); // a rank past the array
        assert_eq!(guild.own_rights(), 0);
    }

    #[test]
    fn an_over_long_rank_array_is_clamped() {
        let mut guild = GuildState::default();
        guild.apply_roster(GuildRoster {
            rank_rights: vec![1; 14],
            ..Default::default()
        });
        assert_eq!(guild.rank_rights.len(), GUILD_RANKS_MAX_COUNT);
    }

    #[test]
    fn a_rank_rename_patches_the_identity_cache() {
        let mut guild = GuildState::default();
        guild.mirror_self(7, 0);
        guild
            .identities
            .insert(7, Some(identity("Legacy", &["GM", "Off"])));
        guild.apply_event(&event(
            guild_event::UPDATE_RANK_NAME,
            &["1", "Officer"],
            None,
        ));
        assert_eq!(guild.identities.get(7).unwrap().rank_name(1), "Officer");
        assert!(guild.dirty);
    }

    #[test]
    fn selection_follows_the_player_not_the_row() {
        let mut guild = GuildState::default();
        guild.apply_roster(GuildRoster {
            members: vec![
                member(11, "Alice", guild_presence::ONLINE),
                member(22, "Bob", guild_presence::ONLINE),
            ],
            ..Default::default()
        });
        guild.display_order = vec![11, 22];
        guild.select(2);
        assert_eq!(guild.selection, 22);
        assert_eq!(
            guild.member_at_mut(2).map(|m| m.name.clone()),
            Some("Bob".to_string()),
            "a display row resolves to the member the wire wants by name"
        );

        // Re-ordered: the same player stays selected.
        guild.display_order = vec![22, 11];
        assert_eq!(feed::index_of(&guild.display_order, guild.selection), 1);

        guild.select(0);
        assert_eq!(guild.selection, 0, "0 = nothing selected");
        guild.select(9);
        assert_eq!(guild.selection, 0, "past the end selects nothing");

        // Bob leaves: the stored guid stops resolving.
        guild.selection = 22;
        guild.apply_roster(GuildRoster {
            members: vec![member(11, "Alice", guild_presence::ONLINE)],
            ..Default::default()
        });
        guild.display_order = vec![11];
        assert_eq!(feed::index_of(&guild.display_order, guild.selection), 0);
    }

    #[test]
    fn an_invitation_arms_and_answering_clears_it() {
        let mut guild = GuildState::default();
        guild.apply_invite("Tigole".into(), "Legacy of Steel".into());
        assert_eq!(
            guild.pending_invite,
            Some(("Tigole".into(), "Legacy of Steel".into()))
        );
        guild.clear_invite();
        assert_eq!(guild.pending_invite, None);
    }

    #[test]
    fn unit_guild_asks_on_a_miss_and_never_names_a_blank_guild() {
        let (commands, rx) = net_commands();
        let mut guild = GuildState::default();
        guild
            .identities
            .insert(7, Some(identity("Legacy", &["GM", "Off"])));

        // A creature or a guildless player: no `PLAYER_GUILDID`.
        let guildless = ObjectFields::from_fields(&[]);
        assert!(unit_guild(&guildless, &mut guild, &commands).is_none());
        assert_eq!(rx.try_iter().count(), 0, "guild id 0 asks nothing");

        // Fields 191/192 = PLAYER_GUILDID/PLAYER_GUILDRANK.
        let unknown = ObjectFields::from_fields(&[
            (wrathplus_protocol::types::Field::PlayerGuildid, 9),
            (wrathplus_protocol::types::Field::PlayerGuildrank, 1),
        ]);
        assert!(
            unit_guild(&unknown, &mut guild, &commands).is_none(),
            "a query in flight is not a blank-named guild"
        );
        assert_eq!(rx.try_iter().count(), 1, "and the miss asked for it");

        let member = ObjectFields::from_fields(&[
            (wrathplus_protocol::types::Field::PlayerGuildid, 7),
            (wrathplus_protocol::types::Field::PlayerGuildrank, 1),
        ]);
        let resolved = unit_guild(&member, &mut guild, &commands).expect("cached");
        assert_eq!(resolved.name, "Legacy");
        assert_eq!(resolved.rank_name, "Off");
        assert_eq!(resolved.rank_index, 1);

        // A rank past the ten slots names nothing.
        let odd_rank = ObjectFields::from_fields(&[
            (wrathplus_protocol::types::Field::PlayerGuildid, 7),
            (wrathplus_protocol::types::Field::PlayerGuildrank, 99),
        ]);
        assert_eq!(
            unit_guild(&odd_rank, &mut guild, &commands)
                .unwrap()
                .rank_name,
            ""
        );
    }

    #[test]
    fn unit_guild_name_is_unit_guilds_name_on_every_leg() {
        let (commands, rx) = net_commands();
        let mut guild = GuildState::default();
        guild
            .identities
            .insert(7, Some(identity("Legacy", &["GM", "Off"])));
        // The negative cache: an empty answer is no such guild.
        guild.identities.insert(8, Some(identity("", &[])));

        let guildless = ObjectFields::from_fields(&[]);
        assert_eq!(unit_guild_name(&guildless, &mut guild, &commands), None);
        assert_eq!(rx.try_iter().count(), 0, "guild id 0 asks nothing");

        let unknown =
            ObjectFields::from_fields(&[(wrathplus_protocol::types::Field::PlayerGuildid, 9)]);
        assert_eq!(
            unit_guild_name(&unknown, &mut guild, &commands),
            None,
            "a query in flight draws no line"
        );
        assert_eq!(rx.try_iter().count(), 1, "and the miss asked for it");

        let blank =
            ObjectFields::from_fields(&[(wrathplus_protocol::types::Field::PlayerGuildid, 8)]);
        assert_eq!(unit_guild_name(&blank, &mut guild, &commands), None);
        assert_eq!(
            rx.try_iter().count(),
            0,
            "the negative cache re-asks nothing"
        );

        let member = ObjectFields::from_fields(&[
            (wrathplus_protocol::types::Field::PlayerGuildid, 7),
            (wrathplus_protocol::types::Field::PlayerGuildrank, 1),
        ]);
        assert_eq!(
            unit_guild_name(&member, &mut guild, &commands),
            Some("Legacy")
        );
        let via_line = unit_guild_name(&member, &mut guild, &commands).map(str::to_owned);
        let via_api = unit_guild(&member, &mut guild, &commands).map(|g| g.name);
        assert_eq!(via_line, via_api, "the two readers of one cache disagree");
    }

    /// A command channel whose receiver stays alive, so a send neither blocks nor is dropped.
    fn net_commands() -> (NetCommands, crossbeam_channel::Receiver<ClientCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (NetCommands(tx), rx)
    }
}
