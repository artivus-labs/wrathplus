use super::*;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
fn body(gm: bool, kind: u8, text: &[u8]) -> Vec<u8> {
    let mut b = vec![kind];
    b.extend(u32::MAX.to_le_bytes());
    b.extend(42u64.to_le_bytes());
    b.extend(0x12345678u32.to_le_bytes());
    if gm {
        b.extend(2u32.to_le_bytes());
        b.extend([255, 0]);
    }
    b.extend(100u64.to_le_bytes());
    b.extend(((text.len() + 1) as u32).to_le_bytes());
    b.extend(text);
    b.extend([0, 0x83]);
    b
}
#[test]
fn adapter_fifo_preserves_raw_gm_order_duplicates_and_clears_on_disconnect() {
    let mut app = App::new();
    app.add_plugins(UiNativeChatPlugin);
    let mut adapter = WrathEventAdapter::new();
    let mut expected = Vec::new();
    for (gm, kind, text) in [
        (false, 1, b"\xff".as_slice()),
        (true, 1, b"".as_slice()),
        (false, 1, b"\xff".as_slice()),
        (
            false,
            0,
            b"Godmode is ON. You won't take damage.".as_slice(),
        ),
    ] {
        let opcode = if gm { 0x3b3 } else { 0x96 };
        let bytes = body(gm, kind, text);
        expected.push(wrathplus_protocol::wrath_chat::decode(opcode, &bytes).unwrap());
        let WrathDispatch::Handled { session_events, .. } =
            adapter.dispatch(opcode, &bytes, 0).unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            &session_events[..],
            [SessionEvent::WrathChat { .. }]
        ));
        crate::net::handlers::dispatch(app.world_mut(), session_events);
        for n in 0..bytes.len() {
            assert!(matches!(
                adapter.dispatch(opcode, &bytes[..n], 0).unwrap(),
                WrathDispatch::Unhandled { .. }
            ));
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(matches!(
            adapter.dispatch(opcode, &extra, 0).unwrap(),
            WrathDispatch::Unhandled { .. }
        ));
    }
    assert_eq!(
        app.world()
            .resource::<NativeChatPackets>()
            .0
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        expected
    );
    let said = app.world().resource::<Messages<ServerSaidMessage>>();
    let said: Vec<_> = said
        .iter_current_update_messages()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(said, ["Godmode is ON. You won't take damage."]);
    crate::net::handlers::dispatch(
        app.world_mut(),
        vec![SessionEvent::Disconnected {
            reason: "test".into(),
            end: wrathplus_protocol::SessionEnd::Lost,
        }],
    );
    assert!(app.world().resource::<NativeChatPackets>().0.is_empty());
}
