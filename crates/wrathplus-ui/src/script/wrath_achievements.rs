//! Build-12340 achievement metadata queries (`0x5b18e0`, `0x5b5530`, `0x5b58b0`,
//! `0x5b6110`). Player progress is not synthesized from the static DBC catalogs.
use std::collections::BTreeMap;

/// Static Achievement.dbc fields consumed by this bounded query family.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathAchievement {
    pub id: u32,
    pub name: String,
    pub category_id: u32,
    pub flags: u32,
    pub required_criteria_count: u32,
    pub reference_achievement_id: u32,
    /// -1 any, 0 Horde, 1 Alliance.
    pub required_faction: i32,
    /// The previous step of a series (`GetPreviousAchievement`), 0 for none.
    pub parent_achievement_id: u32,
    pub description: String,
    pub points: u32,
    /// The `SpellIcon` texture path, empty for none.
    pub icon: String,
    pub reward: String,
    /// The `Order` column, the earned list's same-minute tie-break (`[row+0x20]`, `0x5b29fb`).
    pub order: u32,
}

/// An Achievement_Category.dbc row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathAchievementCategory {
    pub id: u32,
    /// -1 for a root.
    pub parent_id: i32,
    pub name: String,
}

/// Actual Achievement_Criteria.dbc rows, not synthetic special-flag criteria.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathAchievementCriterion {
    pub id: u32,
    pub achievement_id: u32,
    pub criteria_type: u32,
    pub asset_id: u32,
    pub required_quantity: u32,
    pub description: String,
    pub flags: u32,
    /// Seconds, 0 for an untimed criterion (`[crit+0x34]`).
    pub time_limit: u32,
}

/// Achievement.dbc's rows in file order, the order 12340 walks its row array (`[0xad3078]`,
/// stride 0x38), with an id index.
#[derive(Clone, Debug, Default)]
pub struct WrathAchievementTable {
    rows: Vec<WrathAchievement>,
    index: BTreeMap<u32, usize>,
}

impl WrathAchievementTable {
    /// Appends `row`, or replaces the row already under `id` in place.
    pub fn insert(&mut self, id: u32, row: WrathAchievement) {
        match self.index.get(&id) {
            Some(&i) => self.rows[i] = row,
            None => {
                self.index.insert(id, self.rows.len());
                self.rows.push(row);
            }
        }
    }
    pub fn get(&self, id: &u32) -> Option<&WrathAchievement> {
        self.index.get(id).map(|&i| &self.rows[i])
    }
    pub fn get_mut(&mut self, id: &u32) -> Option<&mut WrathAchievement> {
        self.index.get(id).map(|&i| &mut self.rows[i])
    }
    pub fn contains_key(&self, id: &u32) -> bool {
        self.index.contains_key(id)
    }
    /// The rows in file order.
    pub fn values(&self) -> impl Iterator<Item = &WrathAchievement> {
        self.rows.iter()
    }
}

/// Metadata installation is explicit: unavailable catalogs are distinct from valid empty
/// progress. Per-owner vectors retain the catalog's raw linked-row order.
#[derive(Clone, Debug, Default)]
pub struct WrathAchievementMetadata {
    pub achievements: WrathAchievementTable,
    pub criteria: BTreeMap<u32, WrathAchievementCriterion>,
    pub linked_criteria: BTreeMap<u32, Vec<u32>>,
    /// In file order, as the category lists walk them.
    pub categories: Vec<WrathAchievementCategory>,
}

impl WrathAchievementMetadata {
    pub(super) fn criteria_owner(&self, achievement: &WrathAchievement) -> Option<u32> {
        let id = if achievement.reference_achievement_id == 0 {
            achievement.id
        } else {
            achievement.reference_achievement_id
        };
        self.achievements.contains_key(&id).then_some(id)
    }

    fn num_criteria(&self, id: u32) -> Option<usize> {
        let achievement = self.achievements.get(&id)?;
        if achievement.flags & 2 != 0 {
            return Some(0);
        }
        let Some(owner) = self.criteria_owner(achievement) else {
            return Some(0);
        };
        Some(
            usize::from(achievement.flags & 0xb0 != 0)
                + self
                    .linked_criteria
                    .get(&owner)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| self.criteria.get(id))
                    .filter(|row| row.flags & 2 == 0)
                    .count(),
        )
    }

    fn indexed_criterion(
        &self,
        id: u32,
        index: i32,
    ) -> Option<(&WrathAchievement, &WrathAchievementCriterion)> {
        let achievement = self.achievements.get(&id)?;
        if achievement.flags & 2 != 0 || index <= 0 {
            return None;
        }
        let owner = self.criteria_owner(achievement)?;
        let criterion = self
            .linked_criteria
            .get(&owner)?
            .iter()
            .filter_map(|id| self.criteria.get(id))
            .filter(|row| row.flags & 2 == 0 || achievement.flags & 0xb0 != 0)
            .nth(index as usize - 1)?;
        Some((achievement, criterion))
    }
}

impl super::UiScript {
    /// Install genuine static metadata before FrameXML runs; this carries no player progress.
    pub fn set_wrath_achievement_metadata(&mut self, metadata: WrathAchievementMetadata) {
        self.model_mut().wrath_achievements = Some(metadata);
    }
}

pub(super) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    use super::binding_abi::number_arg;
    use mlua::{MultiValue, Value};
    lua.globals().set(
        "GetAchievementCategory",
        lua.create_function(|lua, value: Value| {
            let id = number_arg(lua, value, "Usage: GetAchievementCategory(achievementID)")? as u32;
            let model = lua.app_data_ref::<super::Model>().expect("model");
            let row = model
                .wrath_achievements
                .as_ref()
                .and_then(|m| m.achievements.get(&id));
            let category = row
                .filter(|row| row.category_id != 0 && row.flags & 2 == 0)
                .map(|row| row.category_id);
            Ok(MultiValue::from_vec(
                category
                    .into_iter()
                    .map(|id| Value::Integer(i64::from(id)))
                    .collect(),
            ))
        })?,
    )?;
    lua.globals().set(
        "GetAchievementNumCriteria",
        lua.create_function(|lua, value: Value| {
            let id = number_arg(
                lua,
                value,
                "Usage: GetAchievementNumCriteria(achievementID)",
            )? as u32;
            let model = lua.app_data_ref::<super::Model>().expect("model");
            let count = model
                .wrath_achievements
                .as_ref()
                .and_then(|m| m.num_criteria(id));
            Ok(MultiValue::from_vec(
                count
                    .into_iter()
                    .map(|n| Value::Integer(n as i64))
                    .collect(),
            ))
        })?,
    )?;
    lua.globals().set(
        "GetAchievementCriteriaInfo",
        lua.create_function(|lua, (value, index): (Value, Value)| {
            let id = number_arg(
                lua,
                value,
                "Usage: GetAchievementCrieriaInfo(achievementID, criteriaIndex)",
            )? as u32;
            let numeric_index = lua.coerce_number(index)?.map(|n| n as i64 as i32);
            let selected = {
                let model = lua.app_data_ref::<super::Model>().expect("model");
                let Some(metadata) = model.wrath_achievements.as_ref() else {
                    return Ok(MultiValue::new());
                };
                let pair = if let Some(index) = numeric_index {
                    metadata.indexed_criterion(id, index)
                } else {
                    metadata.criteria.get(&id).and_then(|criterion| {
                        metadata
                            .achievements
                            .get(&criterion.achievement_id)
                            .map(|a| (a, criterion))
                    })
                };
                pair.filter(|(a, _)| a.flags & 2 == 0)
                    .map(|(a, c)| (a.clone(), c.clone()))
            };
            let Some((achievement, criterion)) = selected else {
                return Ok(MultiValue::new());
            };
            let flags = criterion.flags | u32::from(achievement.flags & 0x80 != 0);
            let required =
                if achievement.flags & 0xa0 == 0xa0 && achievement.required_criteria_count != 0 {
                    achievement.required_criteria_count as i32
                } else {
                    criterion.required_quantity as i32
                };
            let (quantity, completed) =
                super::wrath_achievement_progress::criterion_progress(lua, criterion.id, required);
            let formatted = formatted_quantity(lua, achievement.flags, flags, quantity, required)?;
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&criterion.description)?),
                Value::Integer(i64::from(criterion.criteria_type)),
                Value::Boolean(completed),
                Value::Integer(quantity.into()),
                Value::Integer(i64::from(required)),
                Value::Nil,
                Value::Integer(i64::from(flags)),
                Value::Integer(i64::from(criterion.asset_id)),
                Value::String(lua.create_string(formatted)?),
                Value::Integer(i64::from(criterion.id)),
            ]))
        })?,
    )?;
    lua.globals().set(
        "GetAchievementLink",
        lua.create_function(|lua, value: Value| {
            let id = number_arg(lua, value, "Usage: GetAchievementLink(achievementID)")? as u32;
            let model = lua.app_data_ref::<super::Model>().expect("model");
            let Some(metadata) = model.wrath_achievements.as_ref() else {
                return Ok(MultiValue::new());
            };
            let Some(row) = metadata.achievements.get(&id) else {
                return Ok(MultiValue::new());
            };
            if model.unit_guids.player == 0 {
                return Err(mlua::Error::RuntimeError(
                    "GetAchievementLink unavailable: current player GUID absent".into(),
                ));
            }
            if metadata.criteria_owner(row).is_none() {
                let reference = row.reference_achievement_id;
                return Err(mlua::Error::RuntimeError(format!(
                    "GetAchievementLink unavailable: referenced achievement metadata {reference} absent"
                )));
            }
            // Native 0x579590 is QuestTextParser, not Lua substitution. Do not silently emit
            // a raw token-bearing title as if it were the native contextual projection.
            if row.name.len() >= 800 {
                return Err(mlua::Error::RuntimeError(
                    "GetAchievementLink unavailable: native title buffer truncation not implemented".into(),
                ));
            }
            if row.name.contains('$') {
                return Err(mlua::Error::RuntimeError(
                    "GetAchievementLink unavailable: QuestTextParser support for token-bearing title".into(),
                ));
            }
            let link = format!(
                "|cffffff00|Hachievement:{}:{:016X}:0:0:0:-1:0:0:0:0|h[{}]|h|r",
                row.id, model.unit_guids.player, row.name
            );
            Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(link)?,
            )]))
        })?,
    )?;
    Ok(())
}

/// The criterion's progress text; `quantity` comes from the published progress
/// (`wrath_achievement_progress`), 0 without any.
pub(super) fn formatted_quantity(
    lua: &mlua::Lua,
    achievement: u32,
    flags: u32,
    quantity: i32,
    required: i32,
) -> mlua::Result<String> {
    let ordinary = if achievement & 1 != 0 && quantity == 0 {
        "--".to_owned()
    } else {
        quantity.to_string()
    };
    let money_base =
        flags & 0x20 != 0 && (achievement & 8 != 0 || (achievement & 0x38 == 0 && flags & 1 != 0));
    let mut result = if money_base {
        money(lua, quantity)?
    } else {
        ordinary
    };
    if flags & 1 != 0 {
        result.push_str(" / ");
        result.push_str(&if flags & 0x20 != 0 {
            money(lua, required)?
        } else {
            required.to_string()
        });
    }
    Ok(result)
}

pub(super) fn money(lua: &mlua::Lua, quantity: i32) -> mlua::Result<String> {
    let symbols = {
        let model = lua.app_data_ref::<super::Model>().expect("model");
        model
            .cvars
            .get("colorblindmode")
            .is_some_and(|row| row.value.parse::<f64>().unwrap_or(0.0) != 0.0)
    };
    let mut remainder = quantity;
    let mut parts = Vec::new();
    for (name, divisor) in [("GOLD", 10000), ("SILVER", 100), ("COPPER", 1)] {
        let count = if divisor == 1 {
            remainder
        } else if remainder >= divisor {
            remainder / divisor
        } else {
            0
        };
        if divisor != 1 && remainder >= divisor {
            remainder %= divisor;
        }
        if count == 0 && !(divisor == 1 && parts.is_empty()) {
            continue;
        }
        let key = format!(
            "{name}_AMOUNT_{}",
            if symbols { "SYMBOL" } else { "TEXTURE" }
        );
        let template = lua
            .globals()
            .get::<Option<String>>(key.as_str())?
            .ok_or_else(|| {
                mlua::Error::RuntimeError(format!(
                    "GetAchievementCriteriaInfo unavailable: missing {key}"
                ))
            })?;
        if symbols {
            parts.push(format!("{count}{template}"));
        } else {
            let string = lua.globals().get::<mlua::Table>("string")?;
            parts.push(
                string
                    .get::<mlua::Function>("format")?
                    .call::<String>((template, count))?,
            );
        }
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    fn fixture() -> UiScript {
        let mut script = UiScript::new().unwrap();
        let mut metadata = WrathAchievementMetadata::default();
        for (id, flags, reference) in [(1, 0, 0), (2, 2, 0), (3, 0xa0, 1), (4, 0, 99), (5, 1, 1)] {
            metadata.achievements.insert(
                id,
                WrathAchievement {
                    id,
                    name: "Plain title".into(),
                    category_id: 42,
                    flags,
                    required_criteria_count: 7,
                    reference_achievement_id: reference,
                    ..Default::default()
                },
            );
        }
        for (id, flags) in [(10, 0), (11, 2), (12, 1)] {
            metadata.criteria.insert(
                id,
                WrathAchievementCriterion {
                    id,
                    achievement_id: 1,
                    criteria_type: 20,
                    asset_id: 123,
                    required_quantity: 100,
                    description: format!("criterion {id}"),
                    flags,
                    ..Default::default()
                },
            );
        }
        metadata.linked_criteria.insert(1, vec![10, 11, 12]);
        script.set_wrath_achievement_metadata(metadata);
        script
    }

    #[test]
    fn metadata_queries_arity_redirect_hidden_and_actual_special_rows() {
        let script = fixture();
        script.run("assert(GetAchievementCategory('1.9')==42); assert(select('#',GetAchievementCategory(2))==0); assert(select('#',GetAchievementCategory(999))==0); assert(GetAchievementNumCriteria(1)==2); assert(GetAchievementNumCriteria(2)==0); assert(GetAchievementNumCriteria(3)==3); assert(GetAchievementNumCriteria(4)==0); assert(select('#',GetAchievementNumCriteria(999))==0); assert(not pcall(GetAchievementCategory,{}))").unwrap();
        script.run("local d,t,c,q,r,n,f,a,s,id=GetAchievementCriteriaInfo(1,2); assert(d=='criterion 12' and t==20 and c==false and q==0 and r==100 and n==nil and f==1 and a==123 and s=='0 / 100' and id==12); assert(select('#',GetAchievementCriteriaInfo(1,2))==10); local d,t,c,q,r,n,f,a,s,id=GetAchievementCriteriaInfo(3,2); assert(id==11 and f==3 and r==7 and s=='0 / 7'); assert(select('#',GetAchievementCriteriaInfo(2,1))==0); assert(select('#',GetAchievementCriteriaInfo(1,0))==0); local d=GetAchievementCriteriaInfo(10,{}); assert(d=='criterion 10')").unwrap();
    }

    #[test]
    fn plain_link_exact_native_bytes_and_missing_dependencies_named() {
        let script = fixture();
        let error = script
            .eval::<String>("return GetAchievementLink(1)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("current player GUID absent"));
        script.model_mut().unit_guids.player = 0xABCD1234;
        assert_eq!(
            script
                .eval::<String>("return GetAchievementLink(1)")
                .unwrap(),
            "|cffffff00|Hachievement:1:00000000ABCD1234:0:0:0:-1:0:0:0:0|h[Plain title]|h|r"
        );
        script
            .run("assert(select('#',GetAchievementLink(999))==0)")
            .unwrap();
        assert!(script
            .eval::<String>("return GetAchievementLink(4)")
            .unwrap_err()
            .to_string()
            .contains("referenced achievement metadata 99 absent"));
        script
            .model_mut()
            .wrath_achievements
            .as_mut()
            .unwrap()
            .achievements
            .get_mut(&1)
            .unwrap()
            .name = "$N wins".into();
        assert!(script
            .eval::<String>("return GetAchievementLink(1)")
            .unwrap_err()
            .to_string()
            .contains("QuestTextParser"));
    }

    #[test]
    fn title_exceeding_native_buffer_is_explicitly_gated() {
        let script = fixture();
        script.model_mut().unit_guids.player = 1;
        script
            .model_mut()
            .wrath_achievements
            .as_mut()
            .unwrap()
            .achievements
            .get_mut(&1)
            .unwrap()
            .name = "x".repeat(800);
        assert!(script
            .eval::<String>("return GetAchievementLink(1)")
            .unwrap_err()
            .to_string()
            .contains("native title buffer truncation not implemented"));
    }

    #[test]
    fn localized_money_signed_denominator_and_ordinary_flag_formats() {
        let script = fixture();
        script.run("COPPER_AMOUNT_TEXTURE='%d copper-icon'; SILVER_AMOUNT_TEXTURE='%d silver-icon'; GOLD_AMOUNT_TEXTURE='%d gold-icon'; COPPER_AMOUNT_SYMBOL='c'; SILVER_AMOUNT_SYMBOL='s'; GOLD_AMOUNT_SYMBOL='g'").unwrap();
        assert_eq!(
            formatted_quantity(&script.lua, 0, 0x21, 0, -5).unwrap(),
            "0 copper-icon / -5 copper-icon"
        );
        assert_eq!(
            formatted_quantity(&script.lua, 1, 1, 0, 100).unwrap(),
            "-- / 100"
        );
        assert_eq!(
            formatted_quantity(&script.lua, 8, 0x20, 0, 100).unwrap(),
            "0 copper-icon"
        );
        assert_eq!(
            money(&script.lua, 10101).unwrap(),
            "1 gold-icon 1 silver-icon 1 copper-icon"
        );
        script.seed_cvars([crate::script::SeededCvar {
            name: "colorblindMode".into(),
            value: "1".into(),
            default: "0".into(),
            latched: false,
        }]);
        assert_eq!(
            formatted_quantity(&script.lua, 0, 0x21, 0, 101).unwrap(),
            "0c / 1s 1c"
        );
        script.run("COPPER_AMOUNT_SYMBOL=nil").unwrap();
        assert!(money(&script.lua, 0)
            .unwrap_err()
            .to_string()
            .contains("COPPER_AMOUNT_SYMBOL"));
    }
}
