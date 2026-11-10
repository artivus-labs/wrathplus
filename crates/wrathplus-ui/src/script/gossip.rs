//! The gossip bindings: the app pushes the open menu ([`UiScript::set_gossip`]), and
//! `SelectGossipOption` and `CloseGossip` queue intents it drains.
//!
//! `GetGossipOptions()` returns flat `(text, type)` pairs, `type` the lowercase icon name the app
//! maps from the wire `GOSSIP_ICON`. `SelectGossipOption` asks the stock popups first, for a boxed
//! option's confirm or a coded one's code, as 12340's `0x58af10` does.
//!
//! A menu is pushed only once its greeting (`SMSG_NPC_TEXT_UPDATE`) has arrived; until then the
//! VM keeps its last menu, as the reference's frame keeps its last paint: its handler returns on a
//! cache miss without an event, and the greeting write and `GOSSIP_SHOW` share one success path
//! (`0x4e2010`, `0x4e22b0`).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, flag, number_arg};
use super::{Model, ScriptValue};

/// One gossip option; its 1-based index is its place in [`GossipMenu::options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GossipOptionView {
    /// The option's label.
    pub label: String,
    /// The lowercase icon type (`"gossip"`, `"vendor"`, `"taxi"`, …); `GossipFrame.lua:123` draws
    /// `Interface\GossipFrame\<Type>GossipIcon`.
    pub icon_type: String,
    /// The confirm box's text, empty for none, and its price in copper (record `+0x4`, `+0x10`).
    pub box_text: String,
    pub box_money: u32,
    /// The option asks for a code (record `+0xc` bit 0).
    pub coded: bool,
}

/// One quest row of `SMSG_GOSSIP_MESSAGE`; a click sends `CMSG_QUESTGIVER_QUERY_QUEST`. `active`,
/// from the wire icon, puts it under current quests rather than available ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GossipQuestRow {
    pub title: String,
    /// The row's third wire field, signed: a level -1 quest answers -1 (`fild` at `0x58b3e8`).
    pub level: i32,
    pub active: bool,
    pub daily: bool,
    pub repeatable: bool,
    pub complete: bool,
}

/// One open gossip menu, pushed whole by the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GossipMenu {
    /// The NPC greeting, always resolved.
    pub greeting: String,
    /// Quest rows from the same packet.
    pub quests: Vec<GossipQuestRow>,
    pub options: Vec<GossipOptionView>,
}

impl super::UiScript {
    /// Push the open gossip menu; `None` is a close, after which the getters keep answering the
    /// last menu, as 12340's close (`CloseGossip` `0x58aa40`, `SMSG_GOSSIP_COMPLETE` `0x58a840`)
    /// zeroes only the NPC guid (`0x58a550`).
    pub fn set_gossip(&mut self, menu: Option<GossipMenu>) {
        if let Some(menu) = menu {
            self.model_mut().gossip = Some(menu);
        }
    }

    /// Drain the selects `SelectGossipOption` sent since the last call: the 1-based option
    /// position and, for a coded option, the code.
    pub fn take_gossip_selects(&mut self) -> Vec<(u32, Option<String>)> {
        std::mem::take(&mut self.model_mut().gossip_selects)
    }

    /// Whether `CloseGossip` was called since the last drain; the 1.12 close sends no packet.
    pub fn take_gossip_close(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().gossip_close)
    }

    /// Drain the queued 1-based whole-menu quest-row positions.
    pub fn take_gossip_quest_selects(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().gossip_quest_selects)
    }
}

/// Register the gossip globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // The greeting buffer `0xc00ef0`, pushed as-is (`0x58a900`): "" before any menu.
    g.set(
        "GetGossipText",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let text = model.gossip.as_ref().map_or("", |m| m.greeting.as_str());
            lua.create_string(text)
        })?,
    )?;

    g.set(
        "GetGossipOptions",
        lua.create_function(|lua, ()| {
            // Copy out under one short borrow; build the Lua values with none held.
            let pairs: Vec<(String, String)> = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model.gossip.as_ref().map_or_else(Vec::new, |m| {
                    m.options
                        .iter()
                        .map(|o| (o.label.clone(), o.icon_type.clone()))
                        .collect()
                })
            };
            let mut out: Vec<Value> = Vec::with_capacity(pairs.len() * 2);
            for (label, icon) in pairs {
                out.push(Value::String(lua.create_string(&label)?));
                out.push(Value::String(lua.create_string(&icon)?));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // SelectGossipOption(index, text, confirmed) (`0x58b590` → `0x58af10`): the index truncates
    // and a non-number raises. A boxed option fires GOSSIP_CONFIRM (index, box text, price) and
    // waits for `confirmed`; a coded one with no text fires GOSSIP_ENTER_CODE (index); a price
    // above the purse is ERR_NOT_ENOUGH_MONEY (`0x5216f0(0x28)`). Else the select goes out, with
    // the text as the code of a coded option, and GOSSIP_CONFIRM_CANCEL fires. The ids 0x119,
    // 0x11b and 0x11a are those names' slots in the event table (`0xc25314`, `0xc2531c`).
    g.set(
        "SelectGossipOption",
        lua.create_function(|lua, (i, text, confirmed): (Value, Value, Value)| {
            let i = number_arg(lua, i, "Usage: SelectGossipOption(index)")?;
            let text = lua.coerce_string(text)?.map(|s| s.to_string_lossy());
            let confirmed = bool_or_default(Some(&confirmed), false);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(option) = model.gossip.as_ref().and_then(|m| {
                let n = usize::try_from(i).ok()?.checked_sub(1)?;
                m.options.get(n).cloned()
            }) else {
                return Ok(());
            };
            let index = ScriptValue::Int(i64::from(i));
            let code = text.filter(|t| !t.is_empty());
            let event = if !option.box_text.is_empty() && !confirmed {
                let money = ScriptValue::Int(i64::from(option.box_money));
                (
                    "GOSSIP_CONFIRM",
                    vec![index, ScriptValue::Str(option.box_text), money],
                )
            } else if option.coded && code.is_none() {
                ("GOSSIP_ENTER_CODE", vec![index])
            } else if u64::from(option.box_money) > model.money {
                model.ui_errors.push("ERR_NOT_ENOUGH_MONEY");
                return Ok(());
            } else {
                let code = code.filter(|_| option.coded);
                model.gossip_selects.push((i as u32, code));
                ("GOSSIP_CONFIRM_CANCEL", Vec::new())
            };
            model.pending_events.push((event.0.to_string(), event.1));
            Ok(())
        })?,
    )?;

    g.set(
        "CloseGossip",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.gossip_close = true;
            Ok(())
        })?,
    )?;

    // ══ The two quest lists ══
    // The reference splits the quest rows by wire icon, 3 and 4 active and the rest available,
    // behind two vararg verbs (`0x58b3a0`, `0x58b490`); the app splits them at parse time. Each row
    // is title, level, then 1 or nil: trivial by the player's level now (`0x58aba0`), then daily
    // and repeatable, or complete.
    fn quest_rows(lua: &Lua, active: bool) -> mlua::Result<MultiValue> {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let rows: Vec<(GossipQuestRow, bool)> = model.gossip.as_ref().map_or_else(Vec::new, |m| {
            m.quests
                .iter()
                .filter(|q| q.active == active)
                .map(|q| (q.clone(), super::quest_log::quest_trivial(&model, q.level)))
                .collect()
        });
        drop(model);
        let mut out = Vec::new();
        for (q, trivial) in rows {
            out.push(Value::String(lua.create_string(&q.title)?));
            out.push(Value::Integer(i64::from(q.level)));
            out.push(flag(trivial));
            if active {
                out.push(flag(q.complete));
            } else {
                out.push(flag(q.daily));
                out.push(flag(q.repeatable));
            }
        }
        Ok(MultiValue::from_vec(out))
    }

    g.set(
        "GetGossipAvailableQuests",
        lua.create_function(|lua, ()| quest_rows(lua, false))?,
    )?;
    g.set(
        "GetGossipActiveQuests",
        lua.create_function(|lua, ()| quest_rows(lua, true))?,
    )?;

    for (name, active) in [
        ("GetNumGossipAvailableQuests", false),
        ("GetNumGossipActiveQuests", true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                Ok(model.gossip.as_ref().map_or(0, |m| {
                    m.quests.iter().filter(|q| q.active == active).count()
                }))
            })?,
        )?;
    }
    g.set(
        "GetNumGossipOptions",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.gossip.as_ref().map_or(0, |m| m.options.len()))
        })?,
    )?;

    // A select's index is 1-based within its own list, as `GossipFrame.lua:73` numbers the
    // buttons; it maps back to the whole-menu position the app's queue holds.
    fn select_quest(lua: &Lua, active: bool, index: usize) {
        let whole = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            model.gossip.as_ref().and_then(|m| {
                m.quests
                    .iter()
                    .enumerate()
                    .filter(|(_, q)| q.active == active)
                    .nth(index.wrapping_sub(1))
                    .map(|(n, _)| n as u32 + 1)
            })
        };
        if let Some(n) = whole {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .gossip_quest_selects
                .push(n);
        }
    }

    g.set(
        "SelectGossipAvailableQuest",
        lua.create_function(|lua, i: usize| {
            select_quest(lua, false, i);
            Ok(())
        })?,
    )?;
    g.set(
        "SelectGossipActiveQuest",
        lua.create_function(|lua, i: usize| {
            select_quest(lua, true, i);
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GossipMenu, GossipOptionView};
    use crate::script::UiScript;

    fn menu() -> GossipMenu {
        GossipMenu {
            greeting: "Greetings, traveler.".into(),
            quests: Vec::new(),
            options: vec![
                GossipOptionView {
                    label: "Let me browse your goods.".into(),
                    icon_type: "vendor".into(),
                    ..Default::default()
                },
                GossipOptionView {
                    label: "I would like to sign the petition.".into(),
                    icon_type: "gossip".into(),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn gossip_snapshot_reads_and_selects_queue() {
        let mut s = UiScript::new().unwrap();
        assert!(s.eval::<bool>("return GetGossipText() == ''").unwrap());
        assert_eq!(s.arity("GetGossipOptions()").unwrap(), 0);

        s.set_gossip(Some(menu()));
        assert_eq!(
            s.eval::<String>("return GetGossipText()").unwrap(),
            "Greetings, traveler."
        );
        let (l1, t1, l2, t2) = s
            .eval::<(String, String, String, String)>(
                "local a,b,c,d = GetGossipOptions()\n\
                 return a, b, c, d",
            )
            .unwrap();
        assert_eq!(s.arity("GetGossipOptions()").unwrap(), 4);
        assert_eq!(
            (l1.as_str(), t1.as_str()),
            ("Let me browse your goods.", "vendor")
        );
        assert_eq!(l2, "I would like to sign the petition.");
        assert_eq!(t2, "gossip");

        s.run("SelectGossipOption(1)").unwrap();
        s.run("SelectGossipOption(2, 'unused-code')").unwrap(); // no code for an uncoded option
        assert_eq!(s.take_gossip_selects(), vec![(1, None), (2, None)]);
        assert!(s.take_gossip_selects().is_empty(), "drained");

        // `0x58af10`: a boxed option asks first, a coded one wants its code, a price above the
        // purse refuses, and each select that goes out fires GOSSIP_CONFIRM_CANCEL.
        s.tick(0.0);
        let mut m = menu();
        (m.options[0].box_text, m.options[0].box_money) = ("Are you sure?".into(), 500);
        m.options[1].coded = true;
        s.set_gossip(Some(m));
        s.run(
            "E = {} local f = CreateFrame('Frame') f:SetScript('OnEvent', function(_, e, ...) \
             E[#E + 1] = table.concat({e, ...}, ' ') end) f:RegisterEvent('GOSSIP_CONFIRM') \
             f:RegisterEvent('GOSSIP_ENTER_CODE') f:RegisterEvent('GOSSIP_CONFIRM_CANCEL') \
             SelectGossipOption(1.9) SelectGossipOption(2, '') SelectGossipOption(1, nil, 1)",
        )
        .unwrap();
        assert!(s.take_gossip_selects().is_empty(), "nothing sent yet");
        assert_eq!(s.take_ui_errors(), vec!["ERR_NOT_ENOUGH_MONEY"]);
        s.set_money(600);
        s.run("SelectGossipOption(1, nil, true) SelectGossipOption(2, 1234)")
            .unwrap();
        assert_eq!(
            s.take_gossip_selects(),
            vec![(1, None), (2, Some("1234".into()))]
        );
        s.tick(0.0);
        assert_eq!(
            s.eval::<String>("return table.concat(E, ',')").unwrap(),
            "GOSSIP_CONFIRM 1 Are you sure? 500,GOSSIP_ENTER_CODE 2,GOSSIP_CONFIRM_CANCEL,\
             GOSSIP_CONFIRM_CANCEL"
        );
        let err = s.run("SelectGossipOption('x')").unwrap_err().to_string();
        assert!(err.contains("Usage: SelectGossipOption(index)"), "{err}");

        assert!(!s.take_gossip_close());
        s.run("CloseGossip()").unwrap();
        assert!(s.take_gossip_close());
        assert!(!s.take_gossip_close(), "drained");
    }

    #[test]
    fn the_two_gossip_quest_lists_split_by_active_and_select_within_themselves() {
        use super::GossipQuestRow;
        let mut s = UiScript::new().unwrap();
        // No menu: `arg.n == 0`, not a nil.
        assert_eq!(s.arity("GetGossipAvailableQuests()").unwrap(), 0);
        assert_eq!(s.arity("GetGossipActiveQuests()").unwrap(), 0);

        let mut m = menu();
        m.quests = vec![
            GossipQuestRow {
                title: "Report to Goldshire".into(),
                level: -1,
                active: true,
                ..Default::default()
            },
            GossipQuestRow {
                title: "A Threat Within".into(),
                level: 7,
                active: false,
                ..Default::default()
            },
            GossipQuestRow {
                title: "Kobold Camp Cleanup".into(),
                level: 9,
                active: false,
                ..Default::default()
            },
        ];
        s.set_gossip(Some(m));

        assert_eq!(
            // Build 12340: title, level, trivial, daily, repeatable per available row.
            s.eval::<(String, i64, String, i64)>(
                "local t1, l1, _, _, _, t2, l2 = GetGossipAvailableQuests() return t1, l1, t2, l2"
            )
            .unwrap(),
            (
                "A Threat Within".to_string(),
                7,
                "Kobold Camp Cleanup".to_string(),
                9
            )
        );
        // The active row comes first in the menu but splits by icon, not position.
        assert_eq!(
            s.eval::<(String, i64)>("return GetGossipActiveQuests()")
                .unwrap(),
            ("Report to Goldshire".to_string(), -1)
        );

        // Available #2 is the menu's third row.
        s.run("SelectGossipAvailableQuest(2)").unwrap();
        assert_eq!(s.take_gossip_quest_selects(), vec![3]);
        s.run("SelectGossipActiveQuest(1)").unwrap();
        assert_eq!(s.take_gossip_quest_selects(), vec![1]);
        s.run("SelectGossipAvailableQuest(9) SelectGossipActiveQuest(0)")
            .unwrap();
        assert!(s.take_gossip_quest_selects().is_empty(), "drained");
    }

    #[test]
    fn wrath_gossip_rows_are_native_and_keep_all_columns() {
        let mut s = UiScript::new().unwrap();
        let mut m = menu();
        m.quests = vec![
            super::GossipQuestRow {
                title: "Daily".into(),
                level: 80,
                daily: true,
                repeatable: true,
                ..Default::default()
            },
            super::GossipQuestRow {
                title: "Done".into(),
                level: 70,
                active: true,
                complete: true,
                ..Default::default()
            },
            super::GossipQuestRow {
                title: "Other".into(),
                level: 80,
                ..Default::default()
            },
        ];
        s.set_gossip(Some(m));
        // At level 80 the margin is 8 (`0xa12fd0`): 70 is trivial, 80 is not. Flags are 1 or nil.
        s.set_unit(
            "player",
            Some(crate::script::UnitState {
                level: 80,
                ..Default::default()
            }),
        );
        assert_eq!(s.arity("GetGossipAvailableQuests()").unwrap(), 10);
        assert_eq!(s.arity("GetGossipActiveQuests()").unwrap(), 4);
        assert!(s.eval::<bool>("local t,l,tr,d,r,t2,l2,tr2,d2,r2 = GetGossipAvailableQuests(); return t == 'Daily' and l == 80 and tr == nil and d == 1 and r == 1 and t2 == 'Other' and tr2 == nil and d2 == nil and r2 == nil").unwrap());
        assert!(s.eval::<bool>("local t,l,tr,c = GetGossipActiveQuests(); return t == 'Done' and l == 70 and tr == 1 and c == 1").unwrap());
        assert_eq!(s.eval::<(u32,u32,u32)>("return GetNumGossipAvailableQuests(), GetNumGossipActiveQuests(), GetNumGossipOptions()").unwrap(), (2,1,2));
    }

    /// 12340's close zeroes only the NPC guid (`0x58a550`): the option rows, their count
    /// `[0xc016fc]` and the greeting buffer stay until the next menu.
    #[test]
    fn a_close_keeps_the_last_menu() {
        let mut s = UiScript::new().unwrap();
        s.set_gossip(Some(menu()));
        s.set_gossip(None);
        assert!(s
            .eval::<bool>("return GetGossipText() == 'Greetings, traveler.'")
            .unwrap());
        assert_eq!(s.arity("GetGossipOptions()").unwrap(), 4);
    }
}
