//! Build-12340 equipment sets: the queries PaperDollFrame.lua:1118,2218,2227,2524 and
//! EquipmentManager.lua:297 read, over the sets a host publishes ([`UiScript::set_equipment_sets`]),
//! and the gear manager's mutations (PaperDollFrame.lua:2544, StaticPopup.lua:12,25,
//! EquipmentManager.lua:302), queued for the host ([`UiScript::take_equipment_set_requests`]).
//! Missing sets return no values, not fabricated records, tables or mutation success.

use super::{binding_abi, Model, ScriptValue, UiScript};
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};

/// Equipment slots a set covers, `INVSLOT_HEAD` (1) to `INVSLOT_TABARD` (19).
pub const EQUIPMENT_SET_SLOTS: usize = 19;
/// `EQUIPMENT_SET_IGNORED_SLOT` (Constants.lua:235): the slot is left alone.
pub const EQUIPMENT_SET_IGNORED_SLOT: u32 = 1;
const ICON_PATH: &str = "Interface\\Icons\\";

/// One saved set as the gear manager reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EquipmentSetView {
    pub name: String,
    /// The icon's file name under `Interface\Icons`, as the server stores it.
    pub icon: String,
    /// Per slot: 0 empty, [`EQUIPMENT_SET_IGNORED_SLOT`], else the item id (0 while unknown).
    pub items: [u32; EQUIPMENT_SET_SLOTS],
    /// `EquipmentSetContainsLockedItems`.
    pub locked: bool,
}

/// A gear-manager gesture for the host; none of them succeeds until the server says so.
#[derive(Clone, Debug, PartialEq)]
pub enum EquipmentSetRequest {
    /// `SaveEquipmentSet(name, iconIndex)`: the worn items, minus the slots ignored for save.
    Save {
        name: String,
        icon: String,
        ignored: [bool; EQUIPMENT_SET_SLOTS],
    },
    Delete {
        name: String,
    },
    Use {
        name: String,
    },
}

#[derive(Default)]
struct State {
    sets: Vec<EquipmentSetView>,
    ignored: [bool; EQUIPMENT_SET_SLOTS],
    requests: Vec<EquipmentSetRequest>,
}

impl UiScript {
    /// Full replacement, in `GetEquipmentSetInfo`'s order; a change queues
    /// `EQUIPMENT_SETS_CHANGED`.
    pub fn set_equipment_sets(&mut self, sets: Vec<EquipmentSetView>) {
        let changed = match self.lua.app_data_mut::<State>() {
            Some(mut state) if state.sets != sets => {
                state.sets = sets;
                true
            }
            _ => false,
        };
        if changed {
            self.queue_event("EQUIPMENT_SETS_CHANGED", Vec::new());
        }
    }

    pub fn take_equipment_set_requests(&mut self) -> Vec<EquipmentSetRequest> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }

    /// `EQUIPMENT_SWAP_FINISHED(completed, setName)`, for a set use's result.
    pub fn equipment_swap_finished(&mut self, completed: bool, name: Option<String>) {
        self.queue_event(
            "EQUIPMENT_SWAP_FINISHED",
            vec![
                ScriptValue::Bool(completed),
                name.map_or(ScriptValue::Nil, ScriptValue::Str),
            ],
        );
    }
}

fn texture(icon: &str) -> String {
    format!("{ICON_PATH}{icon}")
}

fn find(lua: &Lua, name: &str) -> Option<(usize, EquipmentSetView)> {
    let state = lua.app_data_ref::<State>()?;
    let i = state.sets.iter().position(|s| s.name == name)?;
    Some((i, state.sets[i].clone()))
}

/// A 1-based equipment slot argument; none outside 1..19.
fn slot_arg(lua: &Lua, slot: Value) -> Option<usize> {
    let slot = binding_abi::coerced_number(lua, Some(slot)) as i64;
    usize::try_from(slot - 1)
        .ok()
        .filter(|&s| s < EQUIPMENT_SET_SLOTS)
}

/// `SaveEquipmentSet`'s icon index (PaperDollFrame.lua:2491): `-slot` is the worn item's icon,
/// a positive index a macro icon. Kept by file name, as the server stores it.
fn icon_name(lua: &Lua, index: i64) -> mlua::Result<String> {
    let g = lua.globals();
    let path: Option<String> = if index < 0 {
        g.get::<mlua::Function>("GetInventoryItemTexture")?
            .call(("player", -index))?
    } else {
        g.get::<mlua::Function>("GetMacroIconInfo")?.call(index)?
    };
    let path = path.unwrap_or_else(|| texture("INV_Misc_QuestionMark"));
    Ok(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string())
}

fn queue(lua: &Lua, request: EquipmentSetRequest) {
    if let Some(mut state) = lua.app_data_mut::<State>() {
        state.requests.push(request);
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let globals = lua.globals();
    globals.set(
        "CanUseEquipmentSets",
        lua.create_function(|lua, ()| {
            // 0x5AD880 pushes one NUMBER from equipmentManager's integer value.
            // Native also gates on the equipment-use busy latch C1E956 (set by
            // 0x5ADDD8, cleared by result handler 0x5AF71C). This read-only
            // empty-set implementation never starts an equipment-set operation.
            let model = lua.app_data_ref::<Model>().expect("model");
            let enabled = model
                .cvars
                .get("equipmentmanager")
                .is_some_and(|slot| super::cvars::sstr_to_int(&slot.value) != 0);
            Ok(i32::from(enabled))
        })?,
    )?;
    globals.set(
        "GetNumEquipmentSets",
        lua.create_function(|lua, ()| Ok(lua.app_data_ref::<State>().map_or(0, |s| s.sets.len())))?,
    )?;
    // PickupEquipmentSet(setIndex) (`0x5af2c0`): the index-th set (`0x5ae580`, list order) onto
    // the cursor, mode 0xd (`0x520dc0`); nothing for an index naming none. Its grab sound
    // (`INTERFACESOUND_CURSORGRABOBJECT`) is the app's cursor-sound feed's.
    globals.set(
        "PickupEquipmentSet",
        lua.create_function(|lua, index: Value| {
            let index = lua
                .coerce_number(index)?
                .ok_or_else(|| mlua::Error::runtime("Usage: PickupEquipmentSet(setIndex)"))?
                .round_ties_even() as i64;
            let set = usize::try_from(index - 1).ok().and_then(|i| {
                lua.app_data_ref::<State>()
                    .and_then(|s| s.sets.get(i).cloned())
            });
            if let Some(set) = set {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                super::cursor::clear_cursor(&mut model);
                model.cursor = Some(super::CursorPayload::EquipmentSet(
                    super::CursorEquipmentSet {
                        texture: (!set.icon.is_empty())
                            .then(|| format!("Interface\\Icons\\{}", set.icon)),
                        name: set.name,
                    },
                ));
                super::cursor::queue_cursor_update(&mut model);
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "GetEquipmentSetInfo",
        lua.create_function(|lua, index: Value| {
            // 0x5AF120 uses an unguarded tonumber, truncates, then checks 1..10.
            let index = binding_abi::coerced_number(lua, Some(index)) as i64 as i32;
            if !(1..=10).contains(&index) {
                return Err(mlua::Error::runtime("Usage: GetEquipmentSetInfo(setIndex)"));
            }
            let set = lua
                .app_data_ref::<State>()
                .and_then(|s| s.sets.get(index as usize - 1).cloned());
            // name, texture and the 0-based index (`lessIndex`).
            match set {
                Some(set) => Ok(MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&set.name)?),
                    Value::String(lua.create_string(texture(&set.icon))?),
                    Value::Integer(i64::from(index - 1)),
                ])),
                None => Ok(MultiValue::new()),
            }
        })?,
    )?;
    globals.set(
        "GetEquipmentSetInfoByName",
        lua.create_function(|lua, name: Value| {
            let usage = "Usage: GetEquipmentSetInfoByName(\"setName\")";
            let name = binding_abi::string_arg(lua, name, usage)?;
            match find(lua, &name) {
                Some((i, set)) => Ok(MultiValue::from_vec(vec![
                    Value::String(lua.create_string(texture(&set.icon))?),
                    Value::Integer(i as i64),
                ])),
                None => Ok(MultiValue::new()),
            }
        })?,
    )?;
    globals.set(
        "GetEquipmentSetItemIDs",
        lua.create_function(|lua, (name, out): (Value, Option<Table>)| {
            let usage = "Usage: GetEquipmentSetItemIDs(\"setName\" [, returnTable])";
            let name = binding_abi::string_arg(lua, name, usage)?;
            // A missing set must not touch a caller's table (0x5AEF90).
            let Some((_, set)) = find(lua, &name) else {
                return Ok(MultiValue::new());
            };
            let out = match out {
                Some(t) => t,
                None => lua.create_table()?,
            };
            for (slot, item) in set.items.iter().enumerate() {
                out.raw_set(slot + 1, *item)?;
            }
            Ok(MultiValue::from_vec(vec![Value::Table(out)]))
        })?,
    )?;
    globals.set(
        "EquipmentSetContainsLockedItems",
        lua.create_function(|lua, name: Value| {
            let usage = "Usage: EquipmentSetContainsLockedItems(\"setName\")";
            let name = binding_abi::string_arg(lua, name, usage)?;
            Ok(match find(lua, &name) {
                Some((_, set)) => MultiValue::from_vec(vec![Value::Boolean(set.locked)]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    globals.set(
        "SaveEquipmentSet",
        lua.create_function(|lua, (name, icon): (Value, Value)| {
            let usage = "Usage: SaveEquipmentSet(\"setName\", iconIndex)";
            let name = binding_abi::string_arg(lua, name, usage)?;
            let icon = icon_name(lua, binding_abi::coerced_number(lua, Some(icon)) as i64)?;
            let ignored = lua
                .app_data_ref::<State>()
                .map(|s| s.ignored)
                .unwrap_or_default();
            queue(
                lua,
                EquipmentSetRequest::Save {
                    name,
                    icon,
                    ignored,
                },
            );
            Ok(())
        })?,
    )?;
    const DELETE: &str = r#"Usage: DeleteEquipmentSet("setName")"#;
    for (global, usage) in [
        ("DeleteEquipmentSet", DELETE),
        ("UseEquipmentSet", r#"Usage: UseEquipmentSet("setName")"#),
    ] {
        globals.set(
            global,
            lua.create_function(move |lua, name: Value| {
                let name = binding_abi::string_arg(lua, name, usage)?;
                if find(lua, &name).is_some() {
                    let request = match global {
                        "UseEquipmentSet" => EquipmentSetRequest::Use { name },
                        _ => EquipmentSetRequest::Delete { name },
                    };
                    queue(lua, request);
                }
                Ok(())
            })?,
        )?;
    }
    // Action-bar equipment-set buttons are not modelled: picking a set up holds nothing.
    globals.set(
        "PickupEquipmentSetByName",
        lua.create_function(|lua, name: Value| {
            binding_abi::string_arg(lua, name, "Usage: PickupEquipmentSetByName(\"setName\")")?;
            Ok(())
        })?,
    )?;
    for (global, ignore) in [
        ("EquipmentManagerIgnoreSlotForSave", true),
        ("EquipmentManagerUnignoreSlotForSave", false),
    ] {
        globals.set(
            global,
            lua.create_function(move |lua, slot: Value| {
                let slot = slot_arg(lua, slot);
                if let (Some(slot), Some(mut state)) = (slot, lua.app_data_mut::<State>()) {
                    state.ignored[slot] = ignore;
                }
                Ok(())
            })?,
        )?;
    }
    // This and `EquipmentSetContainsLockedItems` push Lua booleans; whether build 12340 pushes
    // 1/nil instead is untraced.
    globals.set(
        "EquipmentManagerIsSlotIgnoredForSave",
        lua.create_function(|lua, slot: Value| {
            let slot = slot_arg(lua, slot);
            Ok(slot
                .is_some_and(|slot| lua.app_data_ref::<State>().is_some_and(|s| s.ignored[slot])))
        })?,
    )?;
    globals.set(
        "EquipmentManagerClearIgnoredSlotsForSave",
        lua.create_function(|lua, ()| {
            if let Some(mut state) = lua.app_data_mut::<State>() {
                state.ignored = [false; EQUIPMENT_SET_SLOTS];
            }
            Ok(())
        })?,
    )?;
    // GameTooltip:SetEquipmentSet(name) (PaperDollFrame.lua:2524): the set's name, then the
    // tooltip's `OnTooltipSetEquipmentSet`. The equipped/bags/missing count lines are not drawn.
    // The handler runs after the content, as the 12340 cores do (`0x622dd0` runs `+0x514`).
    let tooltip: Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetEquipmentSet",
        lua.create_function(|lua, (this, name): (Table, Value)| {
            let usage = "Usage: GameTooltip:SetEquipmentSet(\"setName\")";
            let name = binding_abi::string_arg(lua, name, usage)?;
            if let Some((_, set)) = find(lua, &name) {
                this.call_method::<()>("SetText", set.name)?;
                let id = super::object::decode_id(&this)?;
                super::event::fire_widget_handler(lua, id, "OnTooltipSetEquipmentSet", vec![])?;
                this.call_method::<()>("Show", ())?;
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    fn script() -> UiScript {
        UiScript::new().unwrap()
    }

    #[test]
    fn missing_sets_have_native_zero_result_arity() {
        script()
            .run(
                r#"
            assert(GetNumEquipmentSets()==0)
            assert(select('#',GetNumEquipmentSets())==1)
            for i=1,10 do assert(select('#',GetEquipmentSetInfo(i))==0) end
            for _,name in ipairs({'', 'missing', 123}) do
                assert(select('#',GetEquipmentSetInfoByName(name))==0)
                assert(select('#',GetEquipmentSetItemIDs(name))==0)
                assert(select('#',EquipmentSetContainsLockedItems(name))==0)
            end
            local work={17, keep='untouched'}
            assert(select('#',GetEquipmentSetItemIDs('missing',work))==0)
            assert(work[1]==17 and work.keep=='untouched')
            assert(select('#',GetEquipmentSetInfo('1.9'))==0)
        "#,
            )
            .unwrap();
    }

    /// `SetEquipmentSet` names the set and runs the tooltip's `OnTooltipSetEquipmentSet`.
    #[test]
    fn set_equipment_set_fires_its_tooltip_script() {
        let mut ui = script();
        ui.set_equipment_sets(vec![view("Tank", "INV_Shield_06")]);
        ui.run(
            r#"
            local owner = CreateFrame('Frame')
            local tip = CreateFrame('GameTooltip', 'EquipSetTip')
            tip:SetOwner(owner, 'ANCHOR_NONE')
            tip:SetScript('OnTooltipSetEquipmentSet', function(self) SEEN = self:GetName() end)
            tip:SetEquipmentSet('Tank')
            "#,
        )
        .unwrap();
        assert_eq!(ui.eval::<String>("return SEEN").unwrap(), "EquipSetTip");
    }

    fn view(name: &str, icon: &str) -> super::EquipmentSetView {
        let mut items = [0; super::EQUIPMENT_SET_SLOTS];
        items[0] = 40_000;
        items[4] = super::EQUIPMENT_SET_IGNORED_SLOT;
        super::EquipmentSetView {
            name: name.into(),
            icon: icon.into(),
            items,
            locked: false,
        }
    }

    #[test]
    fn published_sets_answer_the_gear_manager_and_changes_fire_once() {
        let mut ui = script();
        ui.run(
            r#"
            seen = {}
            local f = CreateFrame('Frame')
            f:RegisterEvent('EQUIPMENT_SETS_CHANGED')
            f:RegisterEvent('EQUIPMENT_SWAP_FINISHED')
            f:SetScript('OnEvent', function(_, e, a, b) seen[#seen+1] = {e, a, b} end)
        "#,
        )
        .unwrap();
        let sets = vec![
            view("Tank", "INV_Shield_06"),
            view("Heal", "Spell_Holy_Heal"),
        ];
        ui.set_equipment_sets(sets.clone());
        ui.set_equipment_sets(sets);
        ui.equipment_swap_finished(true, Some("Heal".into()));
        ui.tick(0.0);
        ui.run(
            r#"
            assert(#seen == 2, #seen)
            assert(seen[1][1] == 'EQUIPMENT_SETS_CHANGED')
            assert(seen[2][1] == 'EQUIPMENT_SWAP_FINISHED' and seen[2][2] == true and seen[2][3] == 'Heal')
            assert(GetNumEquipmentSets() == 2)
            local name, tex, less = GetEquipmentSetInfo(2)
            assert(name == 'Heal' and tex == 'Interface\\Icons\\Spell_Holy_Heal' and less == 1)
            local tex, less = GetEquipmentSetInfoByName('Tank')
            assert(tex == 'Interface\\Icons\\INV_Shield_06' and less == 0)
            local ids = GetEquipmentSetItemIDs('Tank')
            assert(#ids == 19 and ids[1] == 40000 and ids[5] == 1 and ids[2] == 0)
            local mine = {}
            assert(GetEquipmentSetItemIDs('Tank', mine) == mine and mine[1] == 40000)
            assert(EquipmentSetContainsLockedItems('Tank') == false)
            assert(select('#', GetEquipmentSetInfo(3)) == 0)
        "#,
        )
        .unwrap();
    }

    #[test]
    fn gestures_queue_host_requests_and_only_for_known_sets() {
        use super::EquipmentSetRequest;
        let mut ui = script();
        ui.set_equipment_sets(vec![view("Tank", "INV_Shield_06")]);
        ui.run(
            r#"
            EquipmentManagerIgnoreSlotForSave(5)
            EquipmentManagerIgnoreSlotForSave(19)
            EquipmentManagerIgnoreSlotForSave(20)
            EquipmentManagerUnignoreSlotForSave(19)
            assert(EquipmentManagerIsSlotIgnoredForSave(5) and not EquipmentManagerIsSlotIgnoredForSave(19))
            SaveEquipmentSet('PvP', 1)
            EquipmentManagerClearIgnoredSlotsForSave()
            assert(not EquipmentManagerIsSlotIgnoredForSave(5))
            UseEquipmentSet('Tank')
            UseEquipmentSet('missing')
            DeleteEquipmentSet('Tank')
            DeleteEquipmentSet('missing')
            PickupEquipmentSetByName('Tank')
            assert(not pcall(SaveEquipmentSet))
            assert(not pcall(UseEquipmentSet))
        "#,
        )
        .unwrap();
        let requests = ui.take_equipment_set_requests();
        let mut ignored = [false; super::EQUIPMENT_SET_SLOTS];
        ignored[4] = true;
        let EquipmentSetRequest::Save {
            name,
            icon,
            ignored: got,
        } = &requests[0]
        else {
            panic!("{requests:?}");
        };
        assert_eq!((name.as_str(), *got), ("PvP", ignored));
        assert!(
            !icon.contains('\\'),
            "an icon file name, not a path: {icon}"
        );
        assert_eq!(
            requests[1..],
            [
                EquipmentSetRequest::Use {
                    name: "Tank".into()
                },
                EquipmentSetRequest::Delete {
                    name: "Tank".into()
                },
            ]
        );
        assert!(ui.take_equipment_set_requests().is_empty());
    }

    #[test]
    fn pickup_equipment_set_holds_the_set_and_reports_its_name() {
        let mut ui = script();
        ui.set_equipment_sets(vec![super::EquipmentSetView {
            name: "Tank".into(),
            icon: "INV_Shield_06".into(),
            ..Default::default()
        }]);
        ui.run("PickupEquipmentSet(2)").unwrap();
        assert!(ui
            .eval::<bool>("return select('#', GetCursorInfo()) == 0")
            .unwrap());
        ui.run("PickupEquipmentSet(1)").unwrap();
        assert!(ui
            .eval::<bool>("local k, n = GetCursorInfo() return k == 'equipmentset' and n == 'Tank'")
            .unwrap());
        ui.run("PlaceAction(3)").unwrap();
        assert!(
            ui.take_action_sets().is_empty(),
            "labelled: a set stays held"
        );
        assert!(ui.run("PickupEquipmentSet()").is_err());
    }

    #[test]
    fn availability_is_numeric_cvar_not_set_count() {
        script()
            .run(
                r#"
            assert(CanUseEquipmentSets()==0 and select('#',CanUseEquipmentSets())==1)
            assert(type(CanUseEquipmentSets())=='number')
            SetCVar('equipmentManager','1')
            assert(CanUseEquipmentSets()==1 and GetNumEquipmentSets()==0)
            SetCVar('equipmentManager','-1')
            assert(CanUseEquipmentSets()==1)
            SetCVar('equipmentManager','0')
            assert(CanUseEquipmentSets()==0)
        "#,
            )
            .unwrap();
    }

    #[test]
    fn invalid_arguments_raise_and_do_not_create_sets() {
        script().run(r#"
            for _,arg in ipairs({0,-1,11,false,{}}) do
                assert(not pcall(GetEquipmentSetInfo,arg))
            end
            assert(not pcall(GetEquipmentSetInfo))
            for _,query in ipairs({GetEquipmentSetInfoByName,GetEquipmentSetItemIDs,EquipmentSetContainsLockedItems}) do
                assert(not pcall(query))
                assert(not pcall(query,false))
                assert(not pcall(query,{}))
            end
            assert(GetNumEquipmentSets()==0)
        "#).unwrap();
    }
}
