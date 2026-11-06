//! The stable-master bindings: the app pushes a snapshot of the hunter's stable
//! ([`UiScript::set_stable`], every wire row resolved to strings) and the Lua's clicks queue verbs
//! the app drains.
//!
//! Wrath shows slots 0..=4: 0 is the current pet and 1..=4 are purchasable stable slots
//! (AzerothCore `PetDefines.h`: `MAX_PET_STABLES = 4`). `NPCHandler.cpp::SendStablePet` marks
//! current and inactive rows with flags 1 and 2; the app resolves them into these slots. Slot 0
//! can hold a row with no pet out: a dismissed or distant pet still comes from the server's
//! character-pet cache, and the stock window falls back to `GetStablePetInfo(0)` when
//! `UnitExists("pet")` is false (`PetStable.lua:138`).
//!
//! `PickupStablePet` puts the pet on the global cursor as payload mode 10
//! ([`super::cursor::CursorPayload::StablePet`]): `[0xb4d900] = 10` is written at one site,
//! `0x4950ae`, in the stabled-pet grab `0x495010`.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// The current pet plus four purchasable slots (Wrath `PetDefines.h`: `MAX_PET_STABLES = 4`).
pub const NUM_STABLE_SLOTS: usize = 5;

/// One stable window row, every wire field resolved to what the Lua renders. An empty slot is
/// `None` in [`StableState::slots`]; an unbought one lies past [`StableState::num_stable_slots`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StablePetSlot {
    /// The pet's id, which the unstable and swap verbs name; never its slot.
    pub pet_number: u32,
    /// The family icon path, `None` while the creature query is in flight; the stock window passes
    /// it to `SetItemButtonTexture`, which draws the empty-slot art for nil.
    pub icon: Option<String>,
    /// The name the hunter gave the pet, not the creature template's.
    pub name: String,
    pub level: u32,
    /// The localized `CreatureFamily.dbc` word ("Wolf"), `None` until the creature query lands.
    pub family: Option<String>,
    /// The localized `PetLoyalty.dbc` name for the wire's loyalty level.
    pub loyalty: Option<String>,
    /// The localized food names the pet's family eats: `GetStablePetFoodTypes`'s returns.
    pub diet: Vec<String>,
}

/// The open stable's snapshot, pushed whole; `None` when no stable is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StableState {
    /// Stable slots bought, 0..=4 (the wire's `numStableSlots`), not occupied: it enables buttons
    /// `1..=n` and prices the next slot.
    pub num_stable_slots: u32,
    /// The next slot's price in copper (`StableSlotPrices.dbc` row `num_stable_slots + 1`), 0 past
    /// the table, where the stock window has hidden the purchase row.
    pub next_slot_cost: u32,
    /// The five window slots; index 0 is the current pet.
    pub slots: [Option<StablePetSlot>; NUM_STABLE_SLOTS],
    /// A live pet is out: the client's pet-guid test (`[0xb714a0]|[0xb714a4]`), which forks a
    /// stabled pet's drop between swap and unstable. Not `slots[0].is_some()`: a dismissed or
    /// distant pet keeps its slot-0 row with no live guid.
    pub has_live_pet: bool,
}

impl StableState {
    /// `GetNumStablePets()`: how many of the five slots hold a pet.
    fn num_pets(&self) -> u32 {
        self.slots.iter().filter(|s| s.is_some()).count() as u32
    }
}

/// An outbound stable verb, drained by the app, which addresses it to the open stable master.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StableIntent {
    /// `CMSG_STABLE_PET`: stable the current pet; the server picks the first free bought slot.
    Stable,
    /// `CMSG_UNSTABLE_PET`: summon this pet number, valid only with no current pet.
    Unstable(u32),
    /// `CMSG_STABLE_SWAP_PET`: trade the current pet for this pet number in one step.
    Swap(u32),
    /// `CMSG_BUY_STABLE_SLOT`: buy the next slot.
    BuySlot,
}

/// The window's own state beside the snapshot: the selection, the queued verbs and the close flag.
#[derive(Debug)]
pub(crate) struct StableModel {
    pub(crate) state: Option<StableState>,
    /// The selection as the client holds it (`[0xb72250]`): -1 the summoned pet, 0 nothing, else
    /// a pet number, so a pet no longer listed reads back as nothing (`0x4cb84c`).
    pub(crate) selected: i32,
    pub(crate) intents: Vec<StableIntent>,
    pub(crate) close: bool,
}

/// Nothing selected, the client's reset (`0x4caad3`); Lua reads it as -1 through
/// [`super::UiScript::stable_selection`] (`PetStable.lua:50`).
impl Default for StableModel {
    fn default() -> Self {
        Self {
            state: None,
            selected: 0,
            intents: Vec::new(),
            close: false,
        }
    }
}

impl super::UiScript {
    /// Push the open stable's snapshot, or clear it with `None`. Every push clears the selection,
    /// as each list does in the client (`0x4cadf8`); the stock window then re-picks the current pet
    /// or the first occupied slot (`PetStable.lua:49-62`).
    pub fn set_stable(&mut self, state: Option<StableState>) {
        let mut model = self.model_mut();
        model.stable.selected = 0;
        // A held pet stays on the cursor: the reference's close clears only the stable-master
        // guid (`0x4cae10`).
        model.stable.state = state;
    }

    /// Drain the queued stable verbs.
    pub fn take_stable_intents(&mut self) -> Vec<StableIntent> {
        std::mem::take(&mut self.model_mut().stable.intents)
    }

    /// Whether `ClosePetStables()` was called since the last drain; no packet exists for it.
    pub fn take_stable_close(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().stable.close)
    }

    /// `GetSelectedStablePet()`'s answer, translated from the held pet number: 0 the summoned
    /// pet, 1..=4 a stable slot, -1 nothing.
    pub fn stable_selection(&mut self) -> i32 {
        let model = self.model_mut();
        selected_slot(&model.stable)
    }
}

/// The pet number to slot translation behind `GetSelectedStablePet` (`0x4cb810`).
fn selected_slot(m: &StableModel) -> i32 {
    match m.selected {
        -1 => 0,
        0 => -1,
        pet_number => m
            .state
            .as_ref()
            .and_then(|s| {
                s.slots
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find(|(_, slot)| {
                        slot.as_ref()
                            .is_some_and(|p| i64::from(p.pet_number) == i64::from(pet_number))
                    })
                    .map(|(i, _)| i as i32)
            })
            // A pet no longer listed reads as nothing, the loop's fall-through at `0x4cb84c`.
            .unwrap_or(-1),
    }
}

/// A slot argument as an index into [`StableState::slots`], `None` out of range.
fn slot_index(i: i64) -> Option<usize> {
    usize::try_from(i).ok().filter(|&i| i < NUM_STABLE_SLOTS)
}

/// The verb a drop from slot `from` onto slot `to` sends (`ClickStablePet 0x4cb420`): the summoned
/// pet onto an occupied slot swaps, onto an empty bought slot stables, onto an unbought slot sends
/// nothing; a stabled pet onto slot 0 swaps when a live pet is out, else unstables. Stable to
/// stable has no opcode.
fn drag_verb(from: u8, to: u8, state: &StableState) -> Option<StableIntent> {
    if from == to {
        return None;
    }
    let occupied = |i: u8| state.slots.get(usize::from(i)).and_then(|s| s.as_ref());
    match (from, to) {
        (0, _) => match occupied(to) {
            Some(target) => Some(StableIntent::Swap(target.pet_number)),
            None if u32::from(to) <= state.num_stable_slots => Some(StableIntent::Stable),
            None => None,
        },
        (_, 0) => occupied(from).map(|held| {
            if state.has_live_pet {
                StableIntent::Swap(held.pet_number)
            } else {
                StableIntent::Unstable(held.pet_number)
            }
        }),
        _ => None,
    }
}

/// Register the stable globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetStablePetInfo(i) → icon, name, level, family, loyalty: five values on every exit
    // (`0x4cb280`), a miss answering nil, nil, 0, nil, nil.
    g.set(
        "GetStablePetInfo",
        lua.create_function(|lua, i: i64| {
            let pet = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                slot_index(i).and_then(|i| model.stable.state.as_ref()?.slots[i].clone())
            };
            let Some(pet) = pet else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Number(0.0),
                    Value::Nil,
                    Value::Nil,
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                match &pet.icon {
                    Some(t) => Value::String(lua.create_string(t)?),
                    None => Value::Nil,
                },
                Value::String(lua.create_string(&pet.name)?),
                Value::Number(f64::from(pet.level)),
                match &pet.family {
                    Some(f) => Value::String(lua.create_string(f)?),
                    None => Value::Nil,
                },
                match &pet.loyalty {
                    Some(l) => Value::String(lua.create_string(l)?),
                    None => Value::Nil,
                },
            ]))
        })?,
    )?;

    // GetStablePetFoodTypes(i) → one localized diet name per value; none for an empty slot or a
    // family with no diet.
    g.set(
        "GetStablePetFoodTypes",
        lua.create_function(|lua, i: i64| {
            let diet = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                slot_index(i)
                    .and_then(|i| {
                        Some(model.stable.state.as_ref()?.slots[i].as_ref()?.diet.clone())
                    })
                    .unwrap_or_default()
            };
            let mut out = Vec::with_capacity(diet.len());
            for d in &diet {
                out.push(Value::String(lua.create_string(d)?));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // GetNumStableSlots() → slots bought, 0..=4.
    g.set(
        "GetNumStableSlots",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(
                model
                    .stable
                    .state
                    .as_ref()
                    .map_or(0, |s| s.num_stable_slots),
            ))
        })?,
    )?;

    // GetNumStablePets() → how many of the five slots hold a pet.
    g.set(
        "GetNumStablePets",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(
                model.stable.state.as_ref().map_or(0, StableState::num_pets),
            ))
        })?,
    )?;

    // GetNextStableSlotCost() → the next slot's price in copper, 0 with no stable open or past the
    // table.
    g.set(
        "GetNextStableSlotCost",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(
                model.stable.state.as_ref().map_or(0, |s| s.next_slot_cost),
            ))
        })?,
    )?;

    // GetSelectedStablePet() → the selected slot, or -1, on which the stock window picks one itself
    // (`PetStable.lua:50`).
    g.set(
        "GetSelectedStablePet",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(selected_slot(&model.stable)))
        })?,
    )?;

    // ClickStablePet(i) → one value (`0x4cb420`), keyed only on whether the cursor held a pet: a
    // plain click selects and answers 1, so the stock window repaints; a drop answers nil, packet
    // or not, and the repaint comes with the server's next list.
    g.set(
        "ClickStablePet",
        lua.create_function(|lua, i: i64| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(to) = slot_index(i) else {
                // Out of range: a click selects nothing, a drop clears the cursor.
                if !matches!(
                    model.cursor,
                    Some(super::cursor::CursorPayload::StablePet(_))
                ) {
                    model.stable.selected = 0;
                    return Ok(Value::Number(1.0));
                }
                super::cursor::clear_cursor(&mut model);
                return Ok(Value::Nil);
            };
            let to = to as u8;

            // A drop.
            if let Some(super::cursor::CursorPayload::StablePet(held)) = model.cursor.clone() {
                let from = held.slot;
                let intent = model
                    .stable
                    .state
                    .as_ref()
                    .and_then(|state| drag_verb(from, to, state));
                super::cursor::clear_cursor(&mut model);
                if let Some(intent) = intent {
                    model.stable.intents.push(intent);
                }
                return Ok(Value::Nil);
            }

            // A plain click: select only.
            let pet_number = model
                .stable
                .state
                .as_ref()
                .and_then(|s| s.slots[usize::from(to)].as_ref())
                .map(|p| p.pet_number);
            model.stable.selected = match (to, pet_number) {
                // Slot 0 selects the summoned pet by its sentinel, not a pet number.
                (0, _) => -1,
                (_, Some(n)) => n as i32,
                // An empty slot selects nothing.
                (_, None) => 0,
            };
            Ok(Value::Number(1.0))
        })?,
    )?;

    // PickupStablePet(i): the mode-10 cursor grab, returning nothing. The gate is the family icon,
    // not occupancy (`0x495010`): no icon, no grab.
    g.set(
        "PickupStablePet",
        lua.create_function(|lua, i: i64| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let texture = slot_index(i)
                .and_then(|i| model.stable.state.as_ref()?.slots[i].as_ref())
                .and_then(|p| p.icon.clone());
            if let Some(texture) = texture {
                model.cursor = Some(super::cursor::CursorPayload::StablePet(
                    super::cursor::CursorStablePet {
                        slot: i as u8,
                        texture,
                    },
                ));
                super::cursor::queue_cursor_update(&mut model);
            }
            Ok(())
        })?,
    )?;

    // StablePet(): no arguments, gated only on an open stable; stock FrameXML stables by drag.
    g.set(
        "StablePet",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.stable.state.is_some() {
                model.stable.intents.push(StableIntent::Stable);
            }
            Ok(())
        })?,
    )?;

    // UnstablePet(i): silently refused with a pet out (1.12 `0x468550`), a gate the drag path
    // lacks; its unsigned bound rejects slot 0. The reference's other gate, no charmed unit, is not
    // built.
    g.set(
        "UnstablePet",
        lua.create_function(|lua, i: i64| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let pet_number = model
                .stable
                .state
                .as_ref()
                .filter(|s| !s.has_live_pet)
                .and_then(|s| {
                    slot_index(i)
                        .filter(|&i| i != 0)
                        .and_then(|i| s.slots[i].as_ref())
                })
                .map(|p| p.pet_number);
            if let Some(pet_number) = pet_number {
                model
                    .stable
                    .intents
                    .push(StableIntent::Unstable(pet_number));
            }
            Ok(())
        })?,
    )?;

    // Deviation: SetPetStablePaperdoll(model) is inert, because the model pane is an app-side
    // booth that follows the selection every frame, with no VM-side unit to point.
    g.set(
        "SetPetStablePaperdoll",
        lua.create_function(|_, _model: Value| Ok(()))?,
    )?;

    // BuyStableSlot(): an open stable, fewer than four purchased slots (Wrath
    // `MAX_PET_STABLES`), a price row and affordability.
    g.set(
        "BuyStableSlot",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // Affordability reads `model.money`, the purse `GetMoney()` answers.
            let money = model.money;
            let allowed = model.stable.state.as_ref().is_some_and(|s| {
                (s.num_stable_slots as usize) < NUM_STABLE_SLOTS - 1
                    && s.next_slot_cost != 0
                    && u64::from(s.next_slot_cost) <= money
            });
            if allowed {
                model.stable.intents.push(StableIntent::BuySlot);
            }
            Ok(())
        })?,
    )?;

    // ClosePetStables(): no packet exists; the flag tells the app to clear its session.
    g.set(
        "ClosePetStables",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.stable.close = true;
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[path = "stable_tests.rs"]
mod tests;
