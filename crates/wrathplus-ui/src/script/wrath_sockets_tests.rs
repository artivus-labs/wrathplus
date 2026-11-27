use super::*;
use crate::script::cursor::{CursorItem, CursorPayload};
use crate::script::UiScript;

fn script() -> UiScript {
    UiScript::new().unwrap()
}

fn gem(name: &str, matches: bool) -> SocketGemView {
    SocketGemView {
        name: Some(name.into()),
        icon: Some(format!("Interface\\Icons\\{name}")),
        link: Some(format!(
            "|cff1eff00|Hitem:40111:0:0:0:0:0:0:0:80|h[{name}]|h|r"
        )),
        matches,
    }
}

fn session() -> SocketSessionView {
    SocketSessionView {
        name: "Helm".into(),
        icon: Some("Interface\\Icons\\INV_Helmet_01".into()),
        quality: 4,
        link: Some("|cffa335ee|Hitem:100:0:0:0:0:0:0:0:80|h[Helm]|h|r".into()),
        refundable: false,
        bound_tradeable: true,
        sockets: vec![
            SocketSlotView {
                color: "Meta".into(),
                existing: Some(gem("Old", true)),
                new: None,
            },
            SocketSlotView {
                color: "Red".into(),
                existing: None,
                new: Some(gem("Ruby", false)),
            },
        ],
    }
}

#[test]
fn a_published_session_opens_answers_and_its_end_closes() {
    let mut ui = script();
    ui.run(
        r#"
        seen = {}
        local f = CreateFrame('Frame')
        f:RegisterEvent('SOCKET_INFO_UPDATE')
        f:RegisterEvent('SOCKET_INFO_CLOSE')
        f:SetScript('OnEvent', function(_, e) seen[#seen+1] = e end)
    "#,
    )
    .unwrap();
    ui.set_socket_session(Some(session()));
    ui.set_socket_session(Some(session()));
    ui.tick(0.0);
    ui.run(
        r#"
        assert(#seen == 1 and seen[1] == 'SOCKET_INFO_UPDATE', #seen)
        local n, i, q = GetSocketItemInfo()
        assert(n == 'Helm' and i == 'Interface\\Icons\\INV_Helmet_01' and q == 4)
        assert(GetNumSockets() == 2)
        assert(GetSocketTypes(1) == 'Meta' and GetSocketTypes(2) == 'Red')
        assert(select('#', GetSocketTypes(3)) == 0)
        local name, icon, matches = GetExistingSocketInfo(1)
        assert(name == 'Old' and matches == 1)
        assert(GetNewSocketInfo(1) == nil and select('#', GetNewSocketInfo(1)) == 3)
        name, icon, matches = GetNewSocketInfo(2)
        assert(name == 'Ruby' and matches == nil)
        assert(GetNewSocketLink(2):find('Ruby') and select('#', GetExistingSocketLink(2)) == 0)
        assert(GetSocketItemBoundTradeable() == 1 and GetSocketItemRefundable() == nil)
    "#,
    )
    .unwrap();
    ui.set_socket_session(None);
    ui.tick(0.0);
    ui.run("assert(#seen == 2 and seen[2] == 'SOCKET_INFO_CLOSE') assert(GetNumSockets() == 0)")
        .unwrap();
}

#[test]
fn clicks_take_the_cursor_item_and_close_ends_the_session_quietly() {
    let mut ui = script();
    ui.set_socket_session(Some(session()));
    ui.set_cursor_for_test(CursorPayload::Item(CursorItem {
        bag: 2,
        slot: 7,
        item_id: 40_111,
        texture: None,
        link: None,
        count: None,
        quality: None,
        equip_slots: Vec::new(),
        bar_placeable: false,
    }));
    ui.run(
        r#"
        ClickSocketButton(2)
        ClickSocketButton(2)
        ClickSocketButton(9)
        AcceptSockets()
        CloseSocketInfo()
        CloseSocketInfo()
        AcceptSockets()
        assert(GetNumSockets() == 0)
    "#,
    )
    .unwrap();
    assert!(
        ui.cursor_item().is_none(),
        "the gem left the cursor for the socket"
    );
    assert_eq!(
        ui.take_socket_requests(),
        [
            SocketRequest::Click {
                index: 1,
                held: Some((2, 7)),
            },
            SocketRequest::Click {
                index: 1,
                held: None,
            },
            SocketRequest::AcceptSockets,
            SocketRequest::Close,
        ]
    );
}
