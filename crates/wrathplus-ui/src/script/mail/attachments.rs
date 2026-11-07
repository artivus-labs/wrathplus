//! Staged-item source reservations and mail pre-staging eligibility.

use super::{cursor, Model, SEND_SLOTS};
use mlua::{Lua, Value};

/// Compose and the auction sell slot own their reservations, not the app's replaceable
/// container snapshot. Clearing or taking a staged item releases its reservation; a cursor
/// pickup then owns its own source lock.
pub(in crate::script) fn source_reserved(model: &Model, bag: i64, slot: u32) -> bool {
    model
        .mail_send_items
        .iter()
        .flatten()
        .any(|it| it.bag == bag && it.slot == slot)
        || model
            .auction_sell_item
            .as_ref()
            .is_some_and(|it| it.bag == bag && it.slot == slot)
}

/// The common bag-use/cursor-drop gate (`0x56fff0`), before mutating the compose slots.
/// Bound quest items fail before the account-bound exception (template Flags `0x8000000`).
pub(in crate::script) fn can_attach(model: &mut Model, item: &cursor::CursorItem) -> bool {
    let error = if model.wrath_mail_family.send_locked {
        Some("ERR_CLIENT_LOCKED_OUT")
    } else if model
        .mail_send_items
        .iter()
        .flatten()
        .any(|it| it.bag == item.bag && it.slot == item.slot)
    {
        // This gate rejects duplicate compose sources, not other staging owners. A transfer
        // from the auction slot must not fail mail eligibility merely for its old owner.
        return false;
    } else {
        if !model.item_templates.contains_key(&item.item_id) {
            model.item_stat_asks.insert(item.item_id);
        }
        let source = model
            .containers
            .get(&item.bag)
            .and_then(|c| c.slots.get(&item.slot));
        let bound = source.is_some_and(|s| s.flags & 1 != 0 || s.already_bound);
        let template = model.item_templates.get(&item.item_id);
        if bound {
            match template {
                Some(t) if t.bonding == 4 => Some("ERR_MAIL_QUEST_ITEM"),
                Some(t) if t.flags & 0x0800_0000 == 0 => Some("ERR_MAIL_BOUND_ITEM"),
                // Without the template the account-bound exception cannot be decided safely.
                None => Some("ERR_ITEM_NOT_FOUND"),
                _ => None,
            }
        } else {
            None
        }
        .or_else(|| {
            template
                .filter(|t| t.flags & 2 != 0)
                .map(|_| "ERR_MAIL_CONJURED_ITEM")
        })
    };
    if let Some(error) = error {
        model.ui_errors.push(error);
        false
    } else {
        true
    }
}

/// A send-slot argument as the 12340 getters read it (`0x570750`): none is slot 0, a number is
/// rounded after subtracting 1; `None` past the slots.
pub(super) fn send_slot_arg(lua: &Lua, v: Option<&Value>) -> mlua::Result<Option<usize>> {
    let Some(n) = lua.coerce_number(v.cloned().unwrap_or(Value::Nil))? else {
        return Ok(Some(0));
    };
    Ok(usize::try_from((n - 1.0).round_ties_even() as i64)
        .ok()
        .filter(|i| *i < SEND_SLOTS))
}
