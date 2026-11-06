//! Build-12340 `UnitCastingInfo` (`0x611df0`) and `UnitChannelInfo` (`0x612090`) over the cast
//! snapshot App publishes.
//!
//! Native reads the unit's cast slot (spell `+0xa6c`, start `+0xa78`, end `+0xa7c`; channel
//! `+0xa80`/`+0xa84`/`+0xa88`, all `GetTickCount` ms) and answers only while now < end. App
//! resolves names, icons and flags from its spell catalog and stamps start/end on this VM's
//! `GetTime()` clock times 1000, republishing on every edge and to each new VM, before it fires
//! the `UNIT_SPELLCAST_*` event (native order: slot first, then event). isTradeSkill and
//! notInterruptible are Lua booleans (`lua_pushboolean`, `0x84e4d0`); notInterruptible is the
//! unit's `+0xa34` bit `0x8` (`0x71ab20`), which App derives at cast start as `0x7262e0` does.
//! Labelled: the open-lock displayName override (the game object's cached strings through
//! `0x70ba70`/`0x70ba90`).

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// One unit's cast or channel, as App resolves it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathUnitCast {
    pub spell_id: u32,
    pub name: String,
    pub rank: String,
    pub icon: String,
    /// Cast: the spell attribute's `0x4`, which blanks displayName.
    pub hide_display_name: bool,
    /// Channel: attributesEx `0x20000000`, which keeps the name on the player's own bar.
    pub channel_shows_name: bool,
    pub trade_skill: bool,
    /// castID, the cast count byte `+0xa5c`; 0 for channels.
    pub cast_count: u8,
    pub start_ms: u32,
    pub end_ms: u32,
    /// `0x71ab20`: the unit's `+0xa34` bit `0x8`, set at cast start (`0x7262e0`); App derives it.
    pub not_interruptible: bool,
}

/// The published casts and channels by full GUID.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    casts: HashMap<u64, WrathUnitCast>,
    channels: HashMap<u64, WrathUnitCast>,
}

impl super::UiScript {
    /// Replace the cast and channel snapshots whole, keyed by full GUID. No event.
    pub fn set_wrath_unit_casts(
        &mut self,
        casts: HashMap<u64, WrathUnitCast>,
        channels: HashMap<u64, WrathUnitCast>,
    ) {
        self.model_mut().wrath_unit_casts = State { casts, channels };
    }
}

/// The live entry for `token` in `pick`'s table, with the token's lower-cased form.
fn live(
    lua: &Lua,
    token: &str,
    pick: impl Fn(&State) -> &HashMap<u64, WrathUnitCast>,
) -> mlua::Result<Option<WrathUnitCast>> {
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    let Some(guid) = model.unit_guid(token) else {
        return Ok(None);
    };
    let now_ms = model.now * 1000.0;
    Ok(pick(&model.wrath_unit_casts)
        .get(&guid)
        .filter(|c| now_ms < f64::from(c.end_ms))
        .cloned())
}

fn text(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // UnitCastingInfo(unit) → name, rank, displayName, icon, startTime, endTime, isTradeSkill,
    // castID, notInterruptible; nothing when the unit casts nothing or the cast has ended.
    g.set(
        "UnitCastingInfo",
        lua.create_function(|lua, token: Value| {
            let usage = r#"Usage: UnitCastingInfo("unit")"#;
            let token = super::binding_abi::string_arg(lua, token, usage)?;
            let Some(c) = live(lua, &token, |s| &s.casts)? else {
                return Ok(MultiValue::new());
            };
            let display = if c.hide_display_name {
                ""
            } else {
                c.name.as_str()
            };
            Ok(MultiValue::from_vec(vec![
                text(lua, &c.name)?,
                text(lua, &c.rank)?,
                text(lua, display)?,
                text(lua, &c.icon)?,
                Value::Number(f64::from(c.start_ms)),
                Value::Number(f64::from(c.end_ms)),
                Value::Boolean(c.trade_skill),
                Value::Integer(i64::from(c.cast_count)),
                Value::Boolean(c.not_interruptible),
            ]))
        })?,
    )?;

    // UnitChannelInfo(unit) → name, rank, displayName, icon, startTime, endTime, isTradeSkill,
    // notInterruptible. displayName is the localized CHANNELING when the token is "player" whole
    // (`0x612148`) and the spell lacks attributesEx 0x20000000 (`0x612151`), else the name.
    g.set(
        "UnitChannelInfo",
        lua.create_function(|lua, token: Value| {
            let usage = r#"Usage: UnitChannelInfo("unit")"#;
            let token = super::binding_abi::string_arg(lua, token, usage)?;
            let Some(c) = live(lua, &token, |s| &s.channels)? else {
                return Ok(MultiValue::new());
            };
            let display = if token.eq_ignore_ascii_case("player") && !c.channel_shows_name {
                match lua.globals().get::<Value>("CHANNELING")? {
                    v @ Value::String(_) => v,
                    _ => text(lua, "CHANNELING")?,
                }
            } else {
                text(lua, &c.name)?
            };
            Ok(MultiValue::from_vec(vec![
                text(lua, &c.name)?,
                text(lua, &c.rank)?,
                display,
                text(lua, &c.icon)?,
                Value::Number(f64::from(c.start_ms)),
                Value::Number(f64::from(c.end_ms)),
                Value::Boolean(c.trade_skill),
                Value::Boolean(c.not_interruptible),
            ]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mlua::Value;

    use super::WrathUnitCast;
    use crate::script::UiScript;

    fn vm_with_player(guid: u64) -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.model_mut().unit_guids.player = guid;
        s.model_mut().unit_guids.target = guid + 1;
        s.set_now(100.0);
        s
    }

    fn cast(name: &str, end_ms: u32) -> WrathUnitCast {
        WrathUnitCast {
            spell_id: 133,
            name: name.into(),
            rank: "Rank 1".into(),
            icon: "Interface\\Icons\\Spell_Fire_FlameBolt".into(),
            cast_count: 7,
            start_ms: 99_000,
            end_ms,
            ..Default::default()
        }
    }

    #[test]
    fn casting_info_answers_while_the_cast_runs() {
        let mut s = vm_with_player(10);
        assert_eq!(
            s.eval::<i64>("return select('#', UnitCastingInfo('player'))")
                .unwrap(),
            0
        );
        let mut hidden = cast("Hidden", 102_000);
        hidden.hide_display_name = true;
        hidden.trade_skill = true;
        s.set_wrath_unit_casts(
            HashMap::from([(10, cast("Fireball", 101_500)), (11, hidden)]),
            HashMap::new(),
        );
        assert_eq!(
            s.eval::<(String, String, String, String, f64, f64, Value, i64, Value)>(
                "return UnitCastingInfo('player')"
            )
            .unwrap(),
            (
                "Fireball".into(),
                "Rank 1".into(),
                "Fireball".into(),
                "Interface\\Icons\\Spell_Fire_FlameBolt".into(),
                99_000.0,
                101_500.0,
                Value::Boolean(false),
                7,
                Value::Boolean(false)
            )
        );
        assert_eq!(
            s.eval::<(String, Value)>(
                "local _, _, d, _, _, _, t = UnitCastingInfo('target') return d, t"
            )
            .unwrap(),
            (String::new(), Value::Boolean(true)),
            "attr 0x4 blanks displayName; trade skill is true (0x84e4d0)"
        );
        for call in ["UnitCastingInfo()", "UnitChannelInfo({})"] {
            let err = s.eval::<()>(call).unwrap_err().to_string();
            assert!(err.contains("Usage: Unit"), "{call}: {err}");
        }
        assert_eq!(
            s.eval::<i64>("return select('#', UnitCastingInfo('focus'))")
                .unwrap(),
            0
        );
        s.set_now(101.5);
        assert_eq!(
            s.eval::<i64>("return select('#', UnitCastingInfo('player'))")
                .unwrap(),
            0,
            "now >= end answers nothing"
        );
        // The resolver never raises (`0x60abf0`): a token nothing parses answers nothing.
        let n = s.eval::<i64>("return select('#', UnitCastingInfo('bogus'))");
        assert_eq!(n.unwrap(), 0);
    }

    #[test]
    fn channel_info_shows_channeling_on_the_players_own_bar() {
        let mut s = vm_with_player(10);
        s.run("CHANNELING = 'Channeling'").unwrap();
        let mut shows = cast("Mind Flay", 103_000);
        shows.channel_shows_name = true;
        s.set_wrath_unit_casts(
            HashMap::new(),
            HashMap::from([
                (10, cast("Drain Life", 103_000)),
                (11, cast("Arcane Missiles", 103_000)),
            ]),
        );
        assert_eq!(
            s.eval::<i64>("return select('#', UnitChannelInfo('player'))")
                .unwrap(),
            8
        );
        assert_eq!(
            s.eval::<String>("return select(3, UnitChannelInfo('PLAYER'))")
                .unwrap(),
            "Channeling"
        );
        assert_eq!(
            s.eval::<String>("return select(3, UnitChannelInfo('target'))")
                .unwrap(),
            "Arcane Missiles"
        );
        s.set_wrath_unit_casts(HashMap::new(), HashMap::from([(10, shows)]));
        assert_eq!(
            s.eval::<String>("return select(3, UnitChannelInfo('player'))")
                .unwrap(),
            "Mind Flay"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', UnitCastingInfo('target'))")
                .unwrap(),
            0
        );
    }
}
