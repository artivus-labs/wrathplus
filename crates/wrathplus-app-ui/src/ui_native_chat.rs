//! Sole lossless native chat capture owner: every `SMSG_MESSAGECHAT`/`SMSG_GM_MESSAGECHAT`, addon
//! traffic included, with raw bytes, sender GUID and chat type. It is the only input of the native
//! chat feed (`ui_chat::native_message`); the wire emits no shared `SessionEvent::Chat` beside it.
//! This FIFO does not infer permissions, emit UI events, or initiate queries; a system line is also
//! sent on as a [`ServerSaidMessage`], the only tell of server state such as god mode.
use crate::app_core::session::ServerSaidMessage;
use crate::net::NetHandlerApp;
use bevy::prelude::*;
use std::collections::VecDeque;
use wrathplus_protocol::{wrath_chat::Message, SessionEvent, SessionEventKind};

#[derive(Resource, Default)]
pub(crate) struct NativeChatPackets(pub(crate) VecDeque<Message>);

pub struct UiNativeChatPlugin;
impl Plugin for UiNativeChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeChatPackets>()
            .add_message::<ServerSaidMessage>()
            .net_handler(SessionEventKind::WrathChat, receive)
            .net_handler(SessionEventKind::Disconnected, clear);
    }
}
fn receive(
    In(event): In<SessionEvent>,
    mut packets: ResMut<NativeChatPackets>,
    mut server_said: MessageWriter<ServerSaidMessage>,
) {
    if let SessionEvent::WrathChat { message } = event {
        // `CHAT_MSG_SYSTEM` is 0 in 3.3.5 (AzerothCore `SharedDefines.h:3385`).
        if message.chat_type == 0 {
            let text = String::from_utf8_lossy(&message.text).into_owned();
            info!("net: server says — {text}");
            server_said.write(ServerSaidMessage { text });
        }
        packets.0.push_back(message);
    }
}
fn clear(In(_): In<SessionEvent>, mut packets: ResMut<NativeChatPackets>) {
    packets.0.clear();
}

#[cfg(test)]
#[path = "ui_native_chat_tests.rs"]
mod tests;
