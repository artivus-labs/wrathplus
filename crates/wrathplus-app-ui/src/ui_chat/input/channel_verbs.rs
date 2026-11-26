//! The channel verbs the VM queued, as the `CMSG_*` each becomes.

use bevy::prelude::*;

use crate::net::ClientCommand;

/// One queued [`wrathplus_ui::script::ChannelCommand`] as its client command, `None` when the
/// verb sends nothing.
pub(super) fn command(
    cmd: wrathplus_ui::script::ChannelCommand,
    channels: &mut super::super::edit::ChannelState,
    script: &mut wrathplus_ui::script::UiScript,
    native: Option<&mut super::super::native_membership::NativeMembership>,
    chat_log: &mut super::super::feed::ChatLog,
) -> Option<ClientCommand> {
    use wrathplus_ui::script::ChannelCommand as C;
    Some(match cmd {
        C::Join { name, password } => {
            super::manual_join_or_leave(channels, script, &name);
            ClientCommand::JoinChannel {
                name,
                password,
                channel_id: 0,
            }
        }
        C::Leave { name } => {
            // `LeaveChannelByName` (`0x4a0000` → `0x49ee70`): a number names a
            // confirmed slot or the call does nothing. Only this path clears the zone
            // mask and the custom re-join list.
            let name = channels.leave_target(&name)?;
            super::manual_join_or_leave(channels, script, &name);
            channels.note_zone_channel_left(&name);
            channels.note_custom_channel_left(&name);
            // Wrath 0x5093f0 writes +0xa0=1 on the explicit local intent,
            // before its acknowledgment. Automatic walk leaves do not use this path.
            if let Some(native) = native {
                native.explicit_leave(&name);
            }
            ClientCommand::LeaveChannel { name }
        }
        C::List { name } => ClientCommand::ChannelList { name },
        // `ListChannels()`: the joined roster, numbered as `/N` addresses it.
        C::ListAll => {
            let roster: Vec<String> = channels
                .joined
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.as_ref().map(|c| format!("{}. {}", i + 1, c.name)))
                .collect();
            let text = if roster.is_empty() {
                "You are not in any channels.".to_string()
            } else {
                format!("Channels: {}", roster.join(", "))
            };
            chat_log.push_event(super::super::event::ChatEvent::text_only(
                super::super::event::ChatEventKind::System,
                text,
            ));
            return None;
        }
        C::DisplayOwner { name } => ClientCommand::ChannelOwner { name },
        C::SetOwner { name, player } => ClientCommand::ChannelSetOwner { name, player },
        C::SetPassword { name, password } => ClientCommand::ChannelPassword { name, password },
        C::Ban { name, player } => ClientCommand::ChannelBan { name, player },
        C::Invite { name, player } => ClientCommand::ChannelInvite { name, player },
        C::Kick { name, player } => ClientCommand::ChannelKick { name, player },
        C::Moderator { name, player } => ClientCommand::ChannelModerator { name, player },
        C::Unmoderator { name, player } => ClientCommand::ChannelUnmoderator { name, player },
        C::Mute { name, player } => ClientCommand::ChannelMute { name, player },
        C::Unmute { name, player } => ClientCommand::ChannelUnmute { name, player },
        C::Unban { name, player } => ClientCommand::ChannelUnban { name, player },
        C::Moderate { name } => ClientCommand::ChannelModerate { name },
        C::ToggleAnnouncements { name } => ClientCommand::ChannelAnnouncements { name },
    })
}
