use super::*;

#[cfg(test)]
use crate::net::wrath_who_capture::NativeWhoPackets;
use wrathplus_protocol::wrath_who::Entry;
#[cfg(test)]
use wrathplus_protocol::wrath_who::Packet;
pub fn entries(name: &str, race: u32) -> Vec<Entry> {
    vec![Entry {
        name: name.as_bytes().to_vec(),
        guild: vec![],
        level: 80,
        class: 6,
        race,
        gender: 1,
        zone: 12,
    }]
}
#[test]
fn native_feed_publishes_each_receipt_before_its_event_and_clears_on_reset() {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.init_resource::<SocialState>()
        .init_resource::<NativeWhoPackets>()
        .init_resource::<NameCache>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::sound::MessageSounds>()
        .insert_resource(NetCommands(tx))
        .add_systems(
            Update,
            (super::super::native_who::drain, feed_social).chain(),
        );
    let script = UiScript::new().unwrap();
    script
        .run(
            r#"
        received = {}
        local f = CreateFrame('Frame')
        f:RegisterEvent('WHO_LIST_UPDATE')
        f:SetScript('OnEvent', function()
            local name, guild, level, race, class = GetWhoInfo(1)
            local count, total = GetNumWhoResults()
            table.insert(received, name .. ':' .. race .. ':' .. class .. ':' .. total)
        end)
    "#,
        )
        .unwrap();
    app.insert_non_send_resource(script);
    app.world_mut().resource_mut::<SocialState>().who_to_ui = true;
    for (name, race, total) in [("First", 10, 123), ("Second", 11, 456)] {
        app.world_mut()
            .resource_mut::<NativeWhoPackets>()
            .0
            .push_back(Packet::Who {
                total,
                entries: entries(name, race),
            });
    }
    app.update();
    let vm = app.world().non_send_resource::<UiScript>();
    let got: String = vm
        .lua()
        .load("return table.concat(received, '|')")
        .eval()
        .unwrap();
    assert_eq!(
        got,
        "First:Blood Elf:Death Knight:123|Second:Draenei:Death Knight:456"
    );
    assert!(app
        .world()
        .resource::<SocialState>()
        .pending_native_who
        .is_empty());
    app.world_mut()
        .resource_mut::<SocialState>()
        .clear_session();
    app.update();
    let vm = app.world().non_send_resource::<UiScript>();
    let count: u32 = vm.lua().load("return GetNumWhoResults()").eval().unwrap();
    assert_eq!(count, 0);
}
#[test]
fn native_chat_rows_and_summary_use_raw_count_not_total() {
    let mut social = SocialState::default();
    super::super::native_who::apply(&mut social, 999, entries("Synthetic", 10));
    let rows = who_wire_rows(&social, None);
    assert_eq!(rows[0].class, "Death Knight");
    assert_eq!(rows[0].race, "Blood Elf");
    assert_eq!(who_summary_count(&social), 1);
    let lines = who_lines(&rows, who_summary_count(&social), &|key| match key {
        "WHO_LIST_FORMAT" => Some("%s %d %s %s %s".into()),
        "WHO_NUM_RESULTS" => Some("%d player total".into()),
        _ => None,
    });
    assert_eq!(lines.last().unwrap().text(), "1 player total");
    assert!(lines[0].text().contains("Death Knight"));
    assert!(lines[0].text().contains("Blood Elf"));
}
