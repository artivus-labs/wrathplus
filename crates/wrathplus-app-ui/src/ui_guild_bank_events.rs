//! The guild bank's FrameScript events: the frame's, off a change of view, and the queued
//! `GUILDBANK_TEXT_CHANGED`s.
use super::GuildBank;
use wrathplus_ui::script::{ScriptValue, UiScript, WrathGuildBankView};

/// Fire the queued `GUILDBANK_TEXT_CHANGED(tab)`, 1-based (`"%d"`, `0x9e289c`).
pub(super) fn fire_text_changed(bank: &mut GuildBank, script: &mut UiScript) {
    for tab in std::mem::take(&mut bank.text_changed) {
        script.queue_event(
            "GUILDBANK_TEXT_CHANGED",
            vec![ScriptValue::Int(i64::from(tab) + 1)],
        );
    }
}

/// The frame's events for a change of view, in the order the reference's reply handler raises
/// them; the frame opens and closes with the view's `open`, the bank guid.
pub(super) fn events(
    before: Option<&WrathGuildBankView>,
    after: &WrathGuildBankView,
) -> Vec<(&'static str, Vec<ScriptValue>)> {
    let closed = WrathGuildBankView::default();
    let was_open = before.is_some_and(|b| b.open);
    let is_open = after.open;
    // An opening diffs against nothing, so it announces the state it opens on.
    let before = before.filter(|_| was_open).unwrap_or(&closed);
    let mut out = Vec::new();
    match (was_open, is_open) {
        (false, true) => out.push(("GUILDBANKFRAME_OPENED", Vec::new())),
        (true, false) => {
            out.push(("GUILDBANKFRAME_CLOSED", Vec::new()));
            return out;
        }
        (false, false) => return out,
        (true, true) => {}
    }
    if before.tabs != after.tabs {
        out.push(("GUILDBANK_UPDATE_TABS", Vec::new()));
    }
    if before.money != after.money {
        out.push(("GUILDBANK_UPDATE_MONEY", Vec::new()));
    }
    if before.withdraw_money_remaining != after.withdraw_money_remaining {
        out.push(("GUILDBANK_UPDATE_WITHDRAWMONEY", Vec::new()));
    }
    if before.slots != after.slots {
        out.push(("GUILDBANKBAGSLOTS_CHANGED", Vec::new()));
    }
    for (i, text) in after.tab_text.iter().enumerate() {
        if before.tab_text.get(i) != Some(text) {
            out.push((
                "GUILDBANK_UPDATE_TEXT",
                vec![ScriptValue::Int(i as i64 + 1)],
            ));
        }
    }
    if before.money_log != after.money_log || before.tab_logs != after.tab_logs {
        out.push(("GUILDBANKLOG_UPDATE", Vec::new()));
    }
    out
}
