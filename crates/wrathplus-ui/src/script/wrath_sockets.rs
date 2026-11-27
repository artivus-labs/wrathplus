//! Build-12340 socketing window (Blizzard_ItemSocketingUI.lua/.xml) over a session the host
//! publishes ([`UiScript::set_socket_session`]). `SocketContainerItem`/`SocketInventoryItem` ask the
//! host to open one (`wrath_item_services`); the window's clicks, accept and close are queued for
//! it ([`UiScript::take_socket_requests`]). With no session every query keeps the closed shapes:
//! no item, no sockets, nil gems, and nothing queued.

use super::{binding_abi, ScriptValue, UiScript};
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};

/// A gem in a socket, existing or newly placed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocketGemView {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub link: Option<String>,
    /// The gem's colour fits the socket (`gemMatchesSocket`).
    pub matches: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocketSlotView {
    /// `GetSocketTypes`: `"Red"`, `"Yellow"`, `"Blue"`, `"Meta"`, or `"Socket"` for a prismatic one.
    pub color: String,
    pub existing: Option<SocketGemView>,
    pub new: Option<SocketGemView>,
}

/// The item being socketed and its sockets, in socket order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocketSessionView {
    pub name: String,
    pub icon: Option<String>,
    pub quality: u32,
    pub link: Option<String>,
    pub refundable: bool,
    pub bound_tradeable: bool,
    pub sockets: Vec<SocketSlotView>,
}

/// A socketing gesture for the host.
#[derive(Clone, Debug, PartialEq)]
pub enum SocketRequest {
    /// `ClickSocketButton(i)` (0-based here): with an item on the cursor, place it; the cursor
    /// gives it up. Without one, take back the socket's new gem.
    Click {
        index: usize,
        held: Option<(i64, u32)>,
    },
    AcceptSockets,
    /// `CloseSocketInfo`: the window closed itself; the session ends with no event.
    Close,
}

#[derive(Default)]
struct State {
    session: Option<SocketSessionView>,
    requests: Vec<SocketRequest>,
}

impl UiScript {
    /// Publish the session; a new or changed one queues `SOCKET_INFO_UPDATE`, which opens the
    /// window, and the end of one `SOCKET_INFO_CLOSE`.
    pub fn set_socket_session(&mut self, session: Option<SocketSessionView>) {
        let event = match self.lua.app_data_mut::<State>() {
            Some(mut state) if state.session != session => {
                let closing = session.is_none();
                state.session = session;
                Some(if closing {
                    "SOCKET_INFO_CLOSE"
                } else {
                    "SOCKET_INFO_UPDATE"
                })
            }
            _ => None,
        };
        if let Some(event) = event {
            self.queue_event(event, Vec::<ScriptValue>::new());
        }
    }

    pub fn take_socket_requests(&mut self) -> Vec<SocketRequest> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }
}

fn session(lua: &Lua) -> Option<SocketSessionView> {
    lua.app_data_ref::<State>()?.session.clone()
}

/// The 1-based socket argument of an open session, 0-based; raises on a non-number.
fn socket(lua: &Lua, index: Value, usage: &str) -> mlua::Result<Option<(usize, SocketSlotView)>> {
    let index = lua
        .coerce_number(index)?
        .ok_or_else(|| mlua::Error::RuntimeError(usage.into()))? as i64;
    Ok(session(lua).and_then(|s| {
        let i = usize::try_from(index - 1).ok()?;
        s.sockets.get(i).cloned().map(|slot| (i, slot))
    }))
}

fn text(lua: &Lua, v: Option<&str>) -> mlua::Result<Value> {
    v.map(|v| lua.create_string(v).map(Value::String))
        .unwrap_or(Ok(Value::Nil))
}

fn gem_info(lua: &Lua, gem: Option<&SocketGemView>) -> mlua::Result<MultiValue> {
    Ok(match gem {
        Some(g) => MultiValue::from_vec(vec![
            text(lua, g.name.as_deref())?,
            text(lua, g.icon.as_deref())?,
            // The number 1 (`fld1`, `0x5c4cb2`, `0x5c52ea`), not a boolean.
            binding_abi::flag(g.matches),
        ]),
        None => MultiValue::from_vec(vec![Value::Nil, Value::Nil, Value::Nil]),
    })
}

fn queue(lua: &Lua, request: SocketRequest) {
    if let Some(mut state) = lua.app_data_mut::<State>() {
        state.requests.push(request);
    }
}

/// A tooltip shows the link's item, as `GameTooltip:SetHyperlink` does.
fn show_link(tooltip: &Table, link: Option<String>) -> mlua::Result<()> {
    match link {
        Some(link) => tooltip.call_method::<()>("SetHyperlink", link),
        None => Ok(()),
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();
    g.set(
        "GetSocketItemInfo",
        lua.create_function(|lua, ()| {
            Ok(match session(lua) {
                Some(s) => (
                    Value::String(lua.create_string(&s.name)?),
                    text(lua, s.icon.as_deref())?,
                    s.quality,
                ),
                None => (Value::Nil, Value::Nil, 0),
            })
        })?,
    )?;
    g.set(
        "GetNumSockets",
        lua.create_function(|lua, ()| Ok(session(lua).map_or(0, |s| s.sockets.len())))?,
    )?;
    for (name, pick) in [
        ("GetSocketItemRefundable", true),
        ("GetSocketItemBoundTradeable", false),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                let set = session(lua).is_some_and(|s| {
                    if pick {
                        s.refundable
                    } else {
                        s.bound_tradeable
                    }
                });
                // 1 or nil (`fld1` at `0x5c547c` and `0x5c4eac`).
                Ok(binding_abi::flag(set))
            })?,
        )?;
    }
    for (name, new) in [("GetExistingSocketInfo", false), ("GetNewSocketInfo", true)] {
        let usage = format!("Usage: {name}(index)");
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let slot = socket(lua, index, &usage)?.map(|(_, s)| s);
                let gem = slot.as_ref().and_then(|s| {
                    if new {
                        s.new.as_ref()
                    } else {
                        s.existing.as_ref()
                    }
                });
                gem_info(lua, gem)
            })?,
        )?;
    }
    for (name, new) in [("GetExistingSocketLink", false), ("GetNewSocketLink", true)] {
        let usage = format!("Usage: {name}(index)");
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let slot = socket(lua, index, &usage)?.map(|(_, s)| s);
                let link = slot
                    .and_then(|s| if new { s.new } else { s.existing })
                    .and_then(|g| g.link);
                Ok(match link {
                    Some(link) => {
                        MultiValue::from_vec(vec![Value::String(lua.create_string(&link)?)])
                    }
                    None => MultiValue::new(),
                })
            })?,
        )?;
    }
    g.set(
        "GetSocketTypes",
        lua.create_function(|lua, index: Value| {
            Ok(match socket(lua, index, "Usage: GetSocketTypes(index)")? {
                Some((_, slot)) => {
                    MultiValue::from_vec(vec![Value::String(lua.create_string(&slot.color)?)])
                }
                None => MultiValue::new(),
            })
        })?,
    )?;
    g.set(
        "ClickSocketButton",
        lua.create_function(|lua, index: Value| {
            let Some((index, _)) = socket(lua, index, "Usage: ClickSocketButton(index)")? else {
                return Ok(());
            };
            let held = {
                let mut model = lua.app_data_mut::<super::Model>().expect("model");
                let held = match &model.cursor {
                    Some(super::cursor::CursorPayload::Item(item)) => Some((item.bag, item.slot)),
                    _ => None,
                };
                if held.is_some() {
                    super::cursor::clear_cursor(&mut model);
                }
                held
            };
            queue(lua, SocketRequest::Click { index, held });
            Ok(())
        })?,
    )?;
    g.set(
        "AcceptSockets",
        lua.create_function(|lua, ()| {
            if session(lua).is_some() {
                queue(lua, SocketRequest::AcceptSockets);
            }
            Ok(())
        })?,
    )?;
    g.set(
        "CloseSocketInfo",
        lua.create_function(|lua, ()| {
            let open = lua
                .app_data_mut::<State>()
                .is_some_and(|mut s| s.session.take().is_some());
            if open {
                queue(lua, SocketRequest::Close);
            }
            Ok(())
        })?,
    )?;
    // The window's tooltips: the socketed item, a new gem and an existing one, by their links.
    let tooltip: Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetSocketedItem",
        lua.create_function(|lua, this: Table| {
            show_link(&this, session(lua).and_then(|s| s.link))
        })?,
    )?;
    for (name, new) in [("SetSocketGem", true), ("SetExistingSocketGem", false)] {
        let usage = format!("Usage: GameTooltip:{name}(index)");
        tooltip.set(
            name,
            lua.create_function(move |lua, (this, index): (Table, Value)| {
                let link = socket(lua, index, &usage)?
                    .and_then(|(_, s)| if new { s.new } else { s.existing })
                    .and_then(|g| g.link);
                show_link(&this, link)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "wrath_sockets_tests.rs"]
mod tests;
