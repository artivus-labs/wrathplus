//! Build 12340's aura readers: `UnitAura(unit, index | name [, rank] [, filter])` (`0x614d40`),
//! and `UnitBuff`/`UnitDebuff` (`0x614ca0`/`0x614cf0`) as its `HELPFUL`/`HARMFUL` forms, returning
//! `name, rank, icon, count, debuffType, duration, expirationTime, unitCaster, isStealable,
//! shouldConsolidate, spellId` (`0x6147c0`).
//!
//! One argument parser (`0x6110a0` + `0x608390`) and one search (`0x72c9b0`) serve the three, and
//! `CancelUnitBuff` and `GameTooltip:SetUnitAura` with them. Labelled: the list is in the app's
//! slot order, not 12340's caster-ranked insertion order (`0x727c00`), and isStealable is nil.

use mlua::{Lua, MultiValue, Value};

use super::{auras_of, AuraState, Model};

/// The parsed filter word set (`0x608419`): `HELPFUL` (1) and `HARMFUL` (2) override each other,
/// `PLAYER` 4, `RAID` 8, `CANCELABLE` 0x10, `NOT_CANCELABLE` 0x20 (independent bits, `0x6084fc`,
/// `0x608519`), each matched whole and case-insensitively (`0x76e780`) between `" |"` separators
/// (`0xa1f7fc`).
#[derive(Clone, Copy)]
pub(crate) struct Filter {
    helpful: bool,
    player: bool,
    raid: bool,
    cancelable: bool,
    not_cancelable: bool,
}

impl Filter {
    fn parse(filter: Option<&str>) -> Filter {
        let mut f = Filter {
            helpful: true,
            player: false,
            raid: false,
            cancelable: false,
            not_cancelable: false,
        };
        for word in filter.unwrap_or("").split([' ', '|']) {
            match word.to_ascii_uppercase().as_str() {
                "HELPFUL" => f.helpful = true,
                "HARMFUL" => f.helpful = false,
                "PLAYER" => f.player = true,
                "RAID" => f.raid = true,
                "CANCELABLE" => f.cancelable = true,
                "NOT_CANCELABLE" => f.not_cancelable = true,
                _ => {}
            }
        }
        f
    }
}

/// What the arguments name (`0x608390`): a 1-based index, or a name with an optional rank.
pub(crate) enum Which {
    Index(i64),
    Name(String, Option<String>),
}

/// The parsed arguments: the token, what they name and the filter.
pub(crate) struct Query {
    pub(crate) token: String,
    which: Which,
    pub(crate) filter: Filter,
}

impl Query {
    /// The buff/debuff siblings force the sign after parsing, preserving every other filter bit.
    pub(crate) fn force_helpful(&mut self, helpful: bool) {
        self.filter.helpful = helpful;
    }
}

/// `0x6110a0`: `isstring(1)` and (`isnumber(2)` or `isstring(2)`), else `usage` raises. A number,
/// numeric strings included (`lua_isnumber`), is an index (`lua_tointeger`) with the filter
/// third; otherwise argument 2 is the name, 3 the rank (`lua_tolstring`, absent unless a string
/// or number) and 4 the filter. A filter that is no string is no filter (`0x60841d`).
pub(crate) fn parse(lua: &Lua, args: &MultiValue, usage: &'static str) -> mlua::Result<Query> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
    let text = |v: Value| crate::script::binding_abi::optional_string(lua, &v);
    let raise = || mlua::Error::runtime(usage);
    let token = text(arg(0)).ok_or_else(raise)?;
    let (which, filter) = match lua.coerce_number(arg(1))? {
        // `lua_tointeger` (`0x84e070`) is a bare `fistp`: round to nearest, ties to even.
        Some(n) => (Which::Index(n.round_ties_even() as i64), arg(2)),
        None => {
            let name = text(arg(1)).ok_or_else(raise)?;
            (Which::Name(name, text(arg(2))), arg(3))
        }
    };
    Ok(Query {
        token,
        which,
        filter: Filter::parse(text(filter).as_deref()),
    })
}

/// `SStrCmpI` (`0x76ea40`), the name and rank compare.
fn same_text(a: &str, b: &str) -> bool {
    crate::script::unit::token::sstr_eq_i(a.as_bytes(), b.as_bytes())
}

/// `0x53bd70`: with a player and a caster, the caster is the player, or a held caster whose
/// `CHARM`-else-`SUMMON` or `CHARMEDBY`-else-`CREATEDBY` (`0x4f5f20`) is the player: its pet,
/// totems, guardians and summons, or the unit charming it.
fn cast_by_player(model: &Model, a: &AuraState) -> bool {
    let me = model.unit_guids.player;
    me != 0 && a.caster != 0 && (a.caster == me || a.caster_links.contains(&me))
}

/// The aura `query` names on its unit (`0x72c9b0`): each entry must match the sign (`!(flags &
/// 0x80)`) and the caster when `PLAYER` is set, and a cancelable aura fails `NOT_CANCELABLE`, any
/// other `CANCELABLE` (`0x72cad0`), so both reject all; an index counts the entries
/// that pass, a name compares `SStrCmpI` on the name and, if one is given, the rank.
pub(crate) fn find_aura(model: &Model, query: &Query) -> mlua::Result<Option<AuraState>> {
    let f = query.filter;
    let Some(list) = auras_of(model, &query.token)? else {
        return Ok(None);
    };
    let mut passing = list.iter().filter(|a| {
        a.helpful == f.helpful
            && (!f.raid || a.raid)
            && (!f.player || cast_by_player(model, a))
            && !(if a.cancelable {
                f.not_cancelable
            } else {
                f.cancelable
            })
    });
    Ok(match &query.which {
        Which::Index(i) if *i >= 1 => passing.nth((*i - 1) as usize),
        Which::Index(_) => None,
        Which::Name(name, rank) => passing.find(|a| {
            a.name.as_deref().is_some_and(|n| same_text(name, n))
                && rank
                    .as_deref()
                    .is_none_or(|r| same_text(r, a.rank.as_deref().unwrap_or("")))
        }),
    }
    .cloned())
}

/// The token `0x60b420` gives an aura's caster: the first unit slot holding `caster` in the order
/// player, vehicle, pet, partyN/partypetN, raidN/raidpetN, arenaN/arenapetN, target, focus, npc,
/// mouseover; failing that, for a held caster, the first slot holding its owner or its pet
/// (`0x60b6a6`), so a totem's aura names its shaman. Nil for nobody.
fn caster_token(model: &Model, a: &AuraState) -> Option<String> {
    let g = &model.unit_guids;
    let slot = |hit: &dyn Fn(u64) -> bool| -> Option<String> {
        for (token, guid) in [
            ("player", g.player),
            ("vehicle", model.vehicle_guid()),
            ("pet", g.pet),
        ] {
            if hit(guid) {
                return Some(token.into());
            }
        }
        let numbered = [
            ("party", "partypet", &g.party[..], &g.party_pets[..]),
            ("raid", "raidpet", &g.raid[..], &g.raid_pets[..]),
            (
                "arena",
                "arenapet",
                &g.arena[..g.arena_count()],
                &g.arena_pets[..],
            ),
        ];
        for (unit, pet, units, pets) in numbered {
            for (i, &guid) in units.iter().enumerate() {
                if hit(guid) {
                    return Some(format!("{unit}{}", i + 1));
                }
                if hit(pets.get(i).copied().unwrap_or(0)) {
                    return Some(format!("{pet}{}", i + 1));
                }
            }
        }
        [
            ("target", g.target),
            ("focus", g.focus),
            ("npc", g.npc),
            ("mouseover", g.mouseover),
        ]
        .into_iter()
        .find(|&(_, guid)| hit(guid))
        .map(|(token, _)| token.into())
    };
    if a.caster == 0 {
        return None;
    }
    slot(&|guid| guid == a.caster).or_else(|| {
        let [owner, pet] = a.caster_links;
        slot(&|guid| guid != 0 && (guid == owner || guid == pet))
    })
}

fn returns(lua: &Lua, model: &Model, a: &AuraState) -> mlua::Result<MultiValue> {
    let text = |s: Option<&str>| -> mlua::Result<Value> {
        Ok(match s {
            Some(s) => Value::String(lua.create_string(s)?),
            None => Value::Nil,
        })
    };
    let one = |b: bool| if b { Value::Integer(1) } else { Value::Nil };
    Ok(MultiValue::from_vec(vec![
        text(Some(a.name.as_deref().unwrap_or("")))?,
        // `+0x224`, the rank text ("Rank 2", or empty).
        text(Some(a.rank.as_deref().unwrap_or("")))?,
        text(a.icon.as_deref())?,
        Value::Integer(i64::from(a.count)),
        text(a.debuff_type.as_deref())?,
        Value::Number(a.duration),
        Value::Number(a.expiration_time),
        text(caster_token(model, a).as_deref())?,
        // isStealable (`0x53d680`) needs the player's steal mask; labelled nil.
        Value::Nil,
        // `0x6149c1`: `AttributesEx7 & 0x10000000`, the number 1 or nil.
        one(a.consolidate),
        Value::Integer(i64::from(a.spell_id)),
    ]))
}

/// The three readers' body: parse, force the sign (`0x614cd2`, `0x614d22`), search, answer.
fn read(
    lua: &Lua,
    args: MultiValue,
    usage: &'static str,
    sign: Option<bool>,
) -> mlua::Result<MultiValue> {
    let mut query = parse(lua, &args, usage)?;
    if let Some(helpful) = sign {
        query.filter.helpful = helpful;
    }
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    match find_aura(&model, &query)? {
        Some(a) => returns(lua, &model, &a),
        None => Ok(MultiValue::new()),
    }
}

/// CancelUnitBuff(unit, index | name [, rank] [, filter]) (12340 `0x804220`): the same parser and
/// search, HELPFUL by default; on a token naming the player, the aura found is cancelled when its
/// flag `0x10` or its spell's `AttributesEx & 4` (channeled) is set, with no sign test (`0x8042ec`,
/// `0x8042f0`). Labelled: the pet and vehicle branch is not carried.
fn install_cancel(lua: &Lua) -> mlua::Result<()> {
    const USAGE: &str = r#"Usage: CancelUnitBuff("unit", [index] or ["name", "rank"][, "filter"])"#;
    lua.globals().set(
        "CancelUnitBuff",
        lua.create_function(|lua, args: MultiValue| {
            let query = parse(lua, &args, USAGE)?;
            let spell = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                // `0x804281`: the resolved guid must be the active player's (`0x4d3790`).
                if model.unit_guid(&query.token) != Some(model.local_guid()) {
                    return Ok(());
                }
                find_aura(&model, &query)?
                    .filter(|a| a.cancelable || a.channeled)
                    .map(|a| a.spell_id)
            };
            if let Some(spell) = spell {
                lua.app_data_mut::<Model>()
                    .expect("model app_data")
                    .cancel_aura_requests
                    .push(spell);
            }
            Ok(())
        })?,
    )
}

/// Replace the 1.12 readers with build 12340's.
pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    install_cancel(lua)?;
    let g = lua.globals();
    for (name, usage, sign) in [
        (
            "UnitAura",
            r#"Usage: UnitAura("unit", [index] or ["name", "rank"][, "filter"])"#,
            None,
        ),
        (
            "UnitBuff",
            r#"Usage: UnitBuff("unit", [index] or ["name", "rank"][, "filter"])"#,
            Some(true),
        ),
        (
            "UnitDebuff",
            r#"Usage: UnitDebuff("unit", [index] or ["name", "rank"][, "filter"])"#,
            Some(false),
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| read(lua, args, usage, sign))?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{AuraState, UiScript, UnitGuids};

    const ME: u64 = 0x10;
    const PET: u64 = 0x11;
    const PARTY1: u64 = 0x21;
    const TOTEM: u64 = 0x31;
    const MY_TOTEM: u64 = 0x32;
    const TARGET: u64 = 0xF130_0000_0000_0001;

    fn aura(spell_id: u32, name: &str, rank: &str, helpful: bool, caster: u64) -> AuraState {
        AuraState {
            spell_id,
            name: Some(name.into()),
            rank: Some(rank.into()),
            helpful,
            cancelable: helpful,
            caster,
            ..Default::default()
        }
    }

    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_unit_guids(&UnitGuids {
            player: ME,
            pet: PET,
            party: [PARTY1, 0, 0, 0],
            target: TARGET,
            ..Default::default()
        });
        let mut totem = aura(8076, "Strength of Earth", "Rank 1", true, TOTEM);
        totem.caster_links = [PARTY1, 0];
        let mut shout = aura(6673, "Battle Shout", "Rank 1", true, PET);
        shout.caster_links = [ME, 0];
        let mut spring = aura(5677, "Mana Spring", "Rank 1", true, MY_TOTEM);
        spring.caster_links = [ME, 0];
        let mut fort = aura(1243, "Power Word: Fortitude", "Rank 1", true, PARTY1);
        (fort.consolidate, fort.raid) = (true, true);
        s.set_unit_auras(
            TARGET,
            Some(vec![
                aura(1126, "Mark of the Wild", "Rank 2", true, 0x99),
                fort,
                shout,
                totem,
                spring,
                aura(589, "Shadow Word: Pain", "Rank 1", false, ME),
            ]),
        );
        s
    }

    /// `0x6110a0`'s gate and argument forms, `0x72c9b0`'s case-insensitive name and rank, and the
    /// filter words (`0x608419`).
    #[test]
    fn the_aura_readers_parse_search_and_filter_as_12340() {
        let mut s = script();
        for call in [
            "UnitBuff()",
            "UnitBuff('target')",
            "UnitBuff('target', nil)",
            "UnitBuff({}, 1)",
            "UnitAura('target', true)",
        ] {
            let err = s.eval::<()>(call).unwrap_err().to_string();
            assert!(err.contains("Usage: Unit"), "{call}: {err}");
        }
        let name = |call: &str| {
            s.eval::<Option<String>>(&format!("return (select(1, {call}))"))
                .unwrap()
        };
        for (call, want) in [
            ("UnitBuff('target', '2')", Some("Power Word: Fortitude")),
            ("UnitBuff('target', 2.6)", Some("Battle Shout")),
            ("UnitBuff('target', 2.5)", Some("Power Word: Fortitude")),
            (
                "UnitBuff('target', 'mark of the WILD')",
                Some("Mark of the Wild"),
            ),
            (
                "UnitBuff('target', 'Mark of the Wild', 'rank 2')",
                Some("Mark of the Wild"),
            ),
            ("UnitBuff('target', 'Mark of the Wild', 'Rank 1')", None),
            (
                "UnitAura('target', 1, 'HARMFUL')",
                Some("Shadow Word: Pain"),
            ),
            ("UnitBuff('target', 1, 'HARMFUL')", Some("Mark of the Wild")),
            ("UnitDebuff('target', 1, true)", Some("Shadow Word: Pain")),
            (
                "UnitAura('target', 1, 'helpful player')",
                Some("Battle Shout"),
            ),
            // Our totem's buff: its `CREATEDBY` is the player (`0x4f5f20`).
            (
                "UnitAura('target', 2, 'HELPFUL|PLAYER')",
                Some("Mana Spring"),
            ),
            ("UnitAura('target', 3, 'HELPFUL|PLAYER')", None),
            (
                "UnitBuff('target', 1, 'RAID')",
                Some("Power Word: Fortitude"),
            ),
            ("UnitBuff('target', 2, 'RAID')", None),
            (
                "UnitDebuff('target', 1, 'PLAYER')",
                Some("Shadow Word: Pain"),
            ),
            (
                "UnitBuff('target', 1, 'CANCELABLE')",
                Some("Mark of the Wild"),
            ),
            (
                "UnitDebuff('target', 1, 'NOT_CANCELABLE')",
                Some("Shadow Word: Pain"),
            ),
            // Both bits set reject every aura (`0x72cad0`).
            ("UnitBuff('target', 1, 'NOT_CANCELABLE|CANCELABLE')", None),
            ("UnitDebuff('target', 1, 'CANCELABLE NOT_CANCELABLE')", None),
        ] {
            assert_eq!(name(call).as_deref(), want, "{call}");
        }
        // Rank, unitCaster (`0x60b420`: exact slot, else the caster's owner's), consolidate.
        let row = |i: i64| {
            s.eval::<String>(&format!(
                "local _, r, _, _, _, _, _, c, st, co = UnitBuff('target', {i}) \
                 return strjoin(',', r, tostring(c), tostring(st), tostring(co))"
            ))
            .unwrap()
        };
        assert_eq!(row(1), "Rank 2,nil,nil,nil");
        assert_eq!(row(2), "Rank 1,party1,nil,1");
        assert_eq!(row(3), "Rank 1,pet,nil,nil");
        assert_eq!(row(4), "Rank 1,party1,nil,nil");
        assert_eq!(row(5), "Rank 1,player,nil,nil");

        // `CancelUnitBuff` cancels on any token whose guid is the active player's (`0x804281`),
        // the player's own name included.
        let me = crate::script::UnitState {
            exists: true,
            has_object: true,
            guid: ME,
            name: Some("Aldren".into()),
            ..Default::default()
        };
        s.set_unit("player", Some(me));
        // A cancelable harmful aura cancels too: the gate has no sign test (`0x8042ec`).
        let mut harmful = aura(589, "Shadow Word: Pain", "Rank 1", false, 0x99);
        harmful.cancelable = true;
        let mark = aura(1126, "Mark of the Wild", "Rank 2", true, 0x99);
        s.set_player_auras(vec![mark, harmful]);
        s.run("CancelUnitBuff('target', 1) CancelUnitBuff('aldren', 1)")
            .unwrap();
        s.run("CancelUnitBuff('player', 1, 'HARMFUL')").unwrap();
        assert_eq!(s.take_cancel_aura_requests(), vec![1126, 589]);
    }
}
