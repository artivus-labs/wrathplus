//! Build-12340 GameTooltip methods the 1.12 table lacks (one Wow.exe round:
//! `/tmp/azb-ui-tip-native.log`), plus the `IsEquippedItem` global (`0x51c690`).

use mlua::{Lua, MultiValue, ObjectLike, Table, Value};

use super::object::frame_handle_of;
use super::tooltip::{tip_mut, REG_TOOLTIP_METHODS};
use super::Model;

fn tooltip_name(lua: &Lua, this: &Table) -> mlua::Result<String> {
    let name: Option<String> = this.call_method("GetName", ())?;
    let _ = lua;
    Ok(name.unwrap_or_else(|| "<unnamed>".into()))
}

/// The player wears the item: an equipped slot (1..19) holds it. Labelled: native compares the
/// item object (`0x754cd0`/`0x754a20`), this the item id.
fn equipped(m: &Model, item_id: u32) -> bool {
    item_id != 0
        && (1..=19).any(|slot| {
            m.inv_slot("player", slot)
                .is_some_and(|s| s.item_id == item_id)
        })
}

fn flag(on: bool) -> Value {
    super::binding_abi::flag(on)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let m: Table = lua.named_registry_value(REG_TOOLTIP_METHODS)?;

    // GetMinimumWidth() (`0x61d0d0`) → the floor SetMinimumWidth set, and 1 while one is set
    // (labelled: native's flag `+0x3dc` is read as "a floor is set").
    m.set(
        "GetMinimumWidth",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let w = tip_mut(&mut model, h).map_or(0.0, |t| t.min_width);
            Ok((f64::from(w), flag(w > 0.0)))
        })?,
    )?;
    // SetAnchorType(anchorType [, x, y]) (`0x61d3d0`): re-places an owned tooltip under the mode,
    // with the offsets, keeping its content; an unowned one keeps its mode.
    m.set(
        "SetAnchorType",
        lua.create_function(
            |lua, (this, anchor, x, y): (Table, Value, Option<f32>, Option<f32>)| {
                let Value::String(anchor) = anchor else {
                    return Err(mlua::Error::runtime(format!(
                        "Usage: {}:SetAnchorType( anchorType [,Xoffset] [,Yoffset] )",
                        tooltip_name(lua, &this)?
                    )));
                };
                let anchor = anchor.to_string_lossy();
                let h = frame_handle_of(lua, &this)?;
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                if let Some(owner) = tip_mut(&mut model, h).ok().and_then(|t| t.owner) {
                    super::tooltip::place(&mut model, h, owner, Some(&anchor), x, y)?;
                }
                Ok(())
            },
        )?,
    )?;
    // GetItem() (`0x61ef10`) → name, link of the shown item; nothing without one.
    m.set(
        "GetItem",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some((id, name)) = tip_mut(&mut model, h).ok().and_then(|t| t.item.clone()) else {
                return Ok(MultiValue::new());
            };
            let quality = model.item_templates.get(&id).map_or(1, |t| t.quality);
            let link = super::item_stats::item_link(id, &name, quality);
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&name)?),
                Value::String(lua.create_string(&link)?),
            ]))
        })?,
    )?;
    // GetSpell() (`0x61f0f0`) → the shown spell's name, rank ("" for none) and id, from Spell.dbc.
    m.set(
        "GetSpell",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let id = tip_mut(&mut model, h).ok().and_then(|t| t.spell);
            let Some(row) = id.and_then(|id| model.spell_catalog.get(&id)) else {
                return Ok(MultiValue::new());
            };
            let rank = row.rank.as_deref().unwrap_or("");
            lua.pack_multi((row.name.as_str(), rank, row.spell_id))
        })?,
    )?;
    // IsEquippedItem() (`0x61f1f0`) → 1 when the player wears the shown item.
    m.set(
        "IsEquippedItem",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let id = tip_mut(&mut model, h)
                .ok()
                .and_then(|t| t.item.as_ref().map(|i| i.0));
            Ok(flag(id.is_some_and(|id| equipped(&model, id))))
        })?,
    )?;
    // IsUnit("unit") (`0x61da20`) → 1 when the token's unit is the one shown.
    m.set(
        "IsUnit",
        lua.create_function(|lua, (this, token): (Table, Value)| {
            let Value::String(token) = token else {
                return Err(mlua::Error::runtime(format!(
                    "Usage: {}:IsUnit(\"unit\")",
                    tooltip_name(lua, &this)?
                )));
            };
            let token = token.to_string_lossy();
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let guid = model.unit(&token).filter(|u| u.exists).map(|u| u.guid);
            let shown = tip_mut(&mut model, h).ok().and_then(|t| t.unit_guid);
            Ok(flag(guid.is_some_and(|g| g != 0 && Some(g) == shown)))
        })?,
    )?;
    // SetSpellByID(id [, isPet, showSubtext]) (`0x625b90`) → 1 when the spell renders. A negative
    // or missing id raises.
    m.set(
        "SetSpellByID",
        lua.create_function(|lua, (this, id, pet): (Table, Value, Value)| {
            let id = lua
                .coerce_number(id)?
                .map(|n| n as i64)
                .filter(|n| *n >= 0)
                .ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "Invalid spell ID in {}:SetSpellByID",
                        tooltip_name(lua, &this).unwrap_or_default()
                    ))
                })?;
            let alt_caster = super::binding_abi::bool_or_default(Some(&pet), false);
            let known = super::tooltip_spell::spell_view_of(lua, id as u32, alt_caster).is_some();
            super::tooltip_spell::set_spell_by_id(
                lua,
                &this,
                id as u32,
                None,
                super::tooltip_spell::SpellRenderOpts {
                    alt_caster,
                    ..Default::default()
                },
                None,
            )?;
            Ok(flag(known))
        })?,
    )?;
    // SetUnitAura("unit", index | "name" [, "rank"] [, "filter"]) (`0x626240` → `0x625f00`): the
    // aura UnitAura names, HELPFUL by default, in the aura tooltip shape.
    m.set(
        "SetUnitAura",
        lua.create_function(
            |lua, (this, token, which, a, b): (Table, Value, Value, Value, Value)| {
                let (Value::String(token), Value::Integer(_) | Value::Number(_) | Value::String(_)) =
                    (&token, &which)
                else {
                    return Err(mlua::Error::runtime(format!(
                        "Usage: {}:SetUnitAura(\"unit\", [index] or [\"name\", \"rank\"][, \"filter\"])",
                        tooltip_name(lua, &this)?
                    )));
                };
                let filter = if matches!(which, Value::String(_)) { b } else { a };
                let filter = lua.coerce_string(filter)?.map(|s| s.to_string_lossy());
                let hit = {
                    let model = lua.app_data_ref::<Model>().expect("model app_data");
                    super::aura::wrath::find_aura(
                        &model,
                        &token.to_string_lossy(),
                        &which,
                        filter.as_deref(),
                        true,
                    )?
                };
                let (spell_id, name) = hit.map_or((0, None), |a| (a.spell_id, a.name));
                super::tooltip_spell::set_spell_by_id(
                    lua,
                    &this,
                    spell_id,
                    name,
                    super::tooltip_spell::SpellRenderOpts {
                        aura: true,
                        ..Default::default()
                    },
                    None,
                )
            },
        )?,
    )?;
    // SetMerchantCostItem(slot, costIndex) (`0x62ee70`): the vendor row's costIndex-th required
    // item, by id.
    m.set(
        "SetMerchantCostItem",
        lua.create_function(|lua, (this, slot, n): (Table, Value, Value)| {
            let (Some(slot), Some(n)) = (lua.coerce_number(slot)?, lua.coerce_number(n)?) else {
                return Err(mlua::Error::runtime(
                    "Invalid merchant slot in SetMerchantCostItem",
                ));
            };
            let item = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let (slot, n) = (
                    (slot - 1.0).round_ties_even() as i64,
                    (n - 1.0).round_ties_even() as i64,
                );
                usize::try_from(slot)
                    .ok()
                    .zip(usize::try_from(n).ok())
                    .and_then(|(s, n)| {
                        let row = model.merchant.as_ref()?.items.get(s)?;
                        row.extended_cost.as_ref()?.items.get(n).map(|c| c.item_id)
                    })
            };
            match item {
                Some(id) => super::tooltip_item::render_by_id(lua, &this, id, None, None),
                None => Ok(()),
            }
        })?,
    )?;
    // SetHyperlinkCompareItem("hyperlink" [, offset, shiftButton]) (`0x631b60`) → 1 when an
    // equipped item comparable to the link's renders. Labelled: the candidate walk `0x631850` is
    // unread; this takes the offset-th (1-based) worn item in the link item's slots, and the
    // shift and comparison-tooltip arguments do not change it.
    m.set(
        "SetHyperlinkCompareItem",
        lua.create_function(|lua, (this, link, offset): (Table, Value, Value)| {
            let Some(link) = (match &link {
                Value::String(_) | Value::Integer(_) | Value::Number(_) => lua
                    .coerce_string(link.clone())?
                    .map(|s| s.to_string_lossy()),
                _ => None,
            }) else {
                return Err(mlua::Error::runtime(format!(
                    "Usage: {}:SetHyperlinkCompareItem(\"hyperlink\" [, offset, shiftButton])",
                    tooltip_name(lua, &this)?
                )));
            };
            let offset = lua
                .coerce_number(offset)?
                .map_or(0, |n| (n.round_ties_even() as i64 - 1).max(0) as usize);
            let slot = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                if !model.unit("player").is_some_and(|p| p.exists) {
                    return Ok(Value::Nil);
                }
                super::tooltip_item::hyperlink_item_fields(&link)
                    .and_then(|(id, _, _)| model.item_templates.get(&id).map(|t| t.inventory_type))
                    .map(super::tooltip_item::equip_slots_for)
                    .and_then(|slots| {
                        slots
                            .iter()
                            .copied()
                            .filter(|&s| {
                                model
                                    .inv_slot("player", s as usize)
                                    .is_some_and(|i| i.item_id != 0)
                            })
                            .nth(offset)
                    })
            };
            match slot {
                Some(slot) => {
                    // A compare binding renders with the gray `CURRENTLY_EQUIPPED` head (p5 of the
                    // builder, `tooltip_item::render::BuilderFlags`).
                    let h = super::object::frame_handle_of(lua, &this)?;
                    if let Ok(t) = super::tooltip::tip_mut(
                        &mut lua.app_data_mut::<Model>().expect("model app_data"),
                        h,
                    ) {
                        t.equipped_header_armed = true;
                    }
                    this.call_method::<()>("SetInventoryItem", ("player", slot))?;
                    Ok(Value::Integer(1))
                }
                None => Ok(Value::Nil),
            }
        })?,
    )?;

    // SetFrameStack(showHidden) (`0x626560`): the debug frame-stack listing. Labelled: native's
    // builder `0x6230d0` is unread; this lists the mouse focus and its parents, topmost first,
    // the hidden ones only with showHidden.
    let frame_stack: mlua::Function = lua
        .load(
            r#"return function(self, showHidden)
                self:ClearLines()
                local frame = GetMouseFocus and GetMouseFocus()
                while frame do
                    if showHidden or frame:IsVisible() then
                        self:AddLine(frame:GetName() or "<Anon>")
                    end
                    frame = frame:GetParent()
                end
                self:Show()
            end"#,
        )
        .set_name("=[AzerothBench SetFrameStack]")
        .call(())?;
    m.set("SetFrameStack", frame_stack)?;
    // SetTracking() (`0x625940`, Minimap.xml:497): the selected type's name, gold (`0xad2d2c`);
    // with none, the tracking spell's tooltip (`0x625350`, our SetTrackingSpell); with neither,
    // MINIMAP_TRACKING_TOOLTIP_NONE. Labelled: no OnTooltipSet* handler is evident in it.
    let spell: mlua::Function = m.get("SetTrackingSpell")?;
    m.set(
        "SetTracking",
        lua.create_function(move |lua, this: Table| {
            let key = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                match model.tracking_row_key() {
                    Some(k) => Some(k),
                    None if model.tracking.is_some() => None,
                    None => Some("MINIMAP_TRACKING_TOOLTIP_NONE"),
                }
            };
            let Some(key) = key else {
                return spell.call::<()>(this);
            };
            let text = lua.globals().get::<Option<String>>(key)?;
            let h = frame_handle_of(lua, &this)?;
            super::tooltip::clear_content(&mut lua.app_data_mut::<Model>().expect("model"), h);
            super::tooltip::fire_cleared(lua, h);
            let gold = [1.0, 210.0 / 255.0, 0.0, 1.0];
            let line = (text.unwrap_or_else(|| key.to_string()), gold);
            super::tooltip::append_line(lua, &this, line, None, false)?;
            super::tooltip::show_or_hide_empty(lua, h);
            Ok(())
        })?,
    )?;
    // FontString:GetFieldSize() (`0x48bc70`): the text buffer's fixed size, 8191 (`0x9ead70`).
    let fontstrings: Table = lua.named_registry_value(super::REG_FONTSTRING_METHODS)?;
    fontstrings.set(
        "GetFieldSize",
        lua.create_function(|_, _this: Table| Ok(8191.0))?,
    )?;

    // IsEquippedItem(itemID | "name" | "link") (`0x51c690`) → 1 when the player wears it.
    lua.globals().set(
        "IsEquippedItem",
        lua.create_function(|lua, item: Value| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            if !model.unit("player").is_some_and(|p| p.exists) {
                return Ok(Value::Nil);
            }
            let id = match &item {
                Value::Integer(i) => u32::try_from(*i).ok(),
                Value::Number(n) => Some(*n as u32),
                Value::String(s) => {
                    let s = s.to_string_lossy();
                    super::tooltip_item::hyperlink_item_fields(&s)
                        .map(|(id, _, _)| id)
                        .or_else(|| {
                            model
                                .item_templates
                                .iter()
                                .find(|(_, t)| t.name.eq_ignore_ascii_case(&s))
                                .map(|(id, _)| *id)
                        })
                }
                _ => None,
            };
            Ok(flag(id.is_some_and(|id| equipped(&model, id))))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use crate::script::{InvSlotView, ItemTemplateView, UiScript, UnitState};

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                ..Default::default()
            }),
        );
        s.model_mut().unit_guids.player = 1;
        s.set_item_template(
            19019,
            ItemTemplateView {
                name: "Thunderfury".into(),
                quality: 5,
                inventory_type: 13,
                ..Default::default()
            },
        );
        s.set_item_template(
            2589,
            ItemTemplateView {
                name: "Linen Cloth".into(),
                quality: 1,
                ..Default::default()
            },
        );
        let mut slots: crate::script::InventorySlots = Default::default();
        slots[16] = Some(InvSlotView {
            item_id: 19019,
            ..Default::default()
        });
        s.set_inventory_slots(slots);
        s.run("TIP_OWNER = CreateFrame('Frame') TIP = CreateFrame('GameTooltip', 'Tip', TIP_OWNER) TIP:SetOwner(TIP_OWNER, 'ANCHOR_NONE')").unwrap();
        s
    }

    #[test]
    fn shown_item_unit_and_equipped_checks() {
        let s = vm();
        for check in [
            "select('#', TIP:GetItem()) == 0 and TIP:IsEquippedItem() == nil",
            "IsEquippedItem(19019) == 1 and IsEquippedItem('Thunderfury') == 1 and IsEquippedItem(2589) == nil",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
        s.run("TIP:SetHyperlink('item:19019:0:0:0')").unwrap();
        assert!(s.eval::<bool>("local n, l = TIP:GetItem() return n == 'Thunderfury' and l:find('item:19019') ~= nil and TIP:IsEquippedItem() == 1").unwrap());
        s.run("TIP:SetHyperlink('item:2589:0:0:0')").unwrap();
        assert!(s
            .eval::<bool>("return TIP:GetItem() == 'Linen Cloth' and TIP:IsEquippedItem() == nil")
            .unwrap());
        s.run("TIP:ClearLines()").unwrap();
        assert!(s
            .eval::<bool>("return TIP:IsUnit('player') == nil")
            .unwrap());
        assert!(s.run("TIP:IsUnit()").is_err());
    }

    #[test]
    fn spell_aura_merchant_and_compare_shapes() {
        let s = vm();
        assert!(s
            .eval::<bool>("return TIP:SetSpellByID(133) == nil")
            .unwrap());
        assert!(s.run("TIP:SetSpellByID(-1)").is_err() && s.run("TIP:SetSpellByID()").is_err());
        s.run("TIP:SetUnitAura('player', 1) TIP:SetUnitAura('player', 'Arcane Intellect', 'Rank 1', 'HARMFUL')").unwrap();
        assert!(s.run("TIP:SetUnitAura()").is_err());
        s.run("TIP:SetMerchantCostItem(1, 1)").unwrap();
        assert!(s.run("TIP:SetMerchantCostItem(1)").is_err());
        assert!(s.eval::<bool>("return TIP:SetHyperlinkCompareItem('item:19019:0:0:0') == 1 and TIP:GetItem() == 'Thunderfury'").unwrap());
        assert!(s.eval::<bool>("return TIP:SetHyperlinkCompareItem('item:19019:0:0:0', 2) == nil and TIP:SetHyperlinkCompareItem('item:2589:0:0:0') == nil").unwrap());
        assert!(s.run("TIP:SetHyperlinkCompareItem()").is_err());
    }
}

#[cfg(test)]
mod widget_tests {
    use crate::script::UiScript;

    #[test]
    fn minimum_width_anchor_type_focus_field_size_and_region_parent() {
        let s = UiScript::new().unwrap();
        s.run("P = CreateFrame('Frame') T = CreateFrame('GameTooltip', 'WTip', P) T:SetOwner(P, 'ANCHOR_RIGHT') T:SetMinimumWidth(120) T:SetAnchorType('ANCHOR_LEFT', 5, 0) E = CreateFrame('EditBox', 'WBox', P) F = P:CreateFontString() G = P:CreateAnimationGroup() A = G:CreateAnimation('Alpha')").unwrap();
        for check in [
            "select(1, T:GetMinimumWidth()) == 120 and select(2, T:GetMinimumWidth()) == 1",
            "T:GetAnchorType() == 'ANCHOR_LEFT'",
            "E:HasFocus() == false",
            "F:GetFieldSize() == 8191",
            "A:GetRegionParent() == P",
            "type(T.SetTracking) == 'function' and T.SetTracking ~= T.SetTrackingSpell",
        ] {
            assert!(
                s.eval::<bool>(&format!("return {check}")).unwrap(),
                "{check}"
            );
        }
        assert!(s.run("T:SetAnchorType()").is_err());
        s.run("E:SetFocus()").unwrap();
        assert!(s.eval::<bool>("return E:HasFocus() == true").unwrap());
    }
}

#[cfg(test)]
mod debug_tests {
    use crate::script::UiScript;

    #[test]
    fn frame_stack_lists_and_battle_net_stays_inert() {
        let s = UiScript::new().unwrap();
        s.run("P = CreateFrame('Frame', 'StackOwner') T = CreateFrame('GameTooltip', 'StackTip', P) T:SetOwner(P, 'ANCHOR_NONE') T:SetFrameStack(true)").unwrap();
        for check in [
            "select('#', BNIsSelf(1)) == 0 and select('#', BNGetMatureLanguageFilter()) == 0",
            "select('#', BNCreateConversation()) == 0 and select('#', BNGetConversationMemberInfo()) == 0",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
    }
}
