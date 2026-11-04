//! `CreateFrame`'s arguments as build 12340 reads them (`0x81bb20`), in its order: the type and
//! parent shapes, the parent object, the templates; the type lookup comes last, in the caller.
//! Every raise is `luaL_error` (`0x84f280`).

use mlua::{Lua, MultiValue, Value};

use super::super::binding_abi::optional_string;
use super::super::Model;
use crate::widget::FrameHandle;

/// `0xa45118`, verbatim.
const USAGE: &str =
    "Usage: CreateFrame(\"frameType\" [, \"name\"] [, parent] [, \"template\"] [, id])";

/// The arguments, read and checked.
pub(in crate::script) struct CreateArgs {
    pub kind: String,
    pub name: Option<String>,
    pub parent: Option<FrameHandle>,
    /// The template list's names, in order.
    pub templates: Vec<String>,
    /// The node's `id` attribute, as text.
    pub id: Option<String>,
}

pub(in crate::script) fn read(lua: &Lua, args: &MultiValue) -> mlua::Result<CreateArgs> {
    let arg = |i: usize| args.get(i).unwrap_or(&Value::Nil);
    // `lua_isstring(1)` (`0x81bb30`), then the parent's type: none, nil or a table (`0x81bb6a`).
    let kind = optional_string(lua, arg(0));
    let parent_ok = matches!(arg(2), Value::Nil | Value::Table(_));
    let (Some(kind), true) = (kind, parent_ok) else {
        return Err(mlua::Error::runtime(USAGE));
    };
    // `lua_tolstring` on 2 and 4 (`0x81bb87`, `0x81bb92`): a number is its text and retags the
    // slot as a string, so it then takes the template loop; anything else is absent.
    let name = optional_string(lua, arg(1));
    let template = optional_string(lua, arg(3));
    let parent = match arg(2) {
        Value::Table(t) => Some(parent_frame(lua, t)?),
        _ => None,
    };
    if template.is_none() && !arg(3).is_nil() {
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .record_warning(format!(
                "CreateFrame: the 4th argument (inherits) must be a template-name string, got {}; \
                 ignored for '{}'",
                arg(3).type_name(),
                name.as_deref().unwrap_or("<unnamed>")
            ));
    }
    // `SStrTokenize` (`0x76f1e0`) with the delimiter set `" ,"` (`0x9eb7ec`): a space splits like
    // a comma and a run of either is one break. Each name must resolve (`0x812ce0`), else the
    // raise names that token (`0x81bd30`); nothing is built yet.
    let templates: Vec<String> = template
        .as_deref()
        .unwrap_or_default()
        .split([' ', ','])
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let known = model.framexml_templates.borrow();
        if let Some(miss) = templates.iter().find(|t| {
            !known.contains_key(t.as_str()) && !known.keys().any(|k| k.eq_ignore_ascii_case(t))
        }) {
            return Err(mlua::Error::runtime(format!(
                "CreateFrame(): Couldn't find inherited node \"{miss}\""
            )));
        }
    }
    // Argument 5 by `lua_isstring` (`0x81bd14`) and `lua_tolstring` (`0x81bd25`): its text is the
    // node's `id` attribute (`0x81bda4`), which the loader reads as it reads `id=` in XML.
    let id = optional_string(lua, arg(4));
    Ok(CreateArgs {
        kind,
        name,
        parent,
        templates,
        id,
    })
}

/// A table parent: its `[0]` must be an object (`0x81bbae`, else `0x81bbcc`), and the object a
/// frame (the type test through `vtbl+0x10`, else `0x81bc0c`): a texture is no parent.
fn parent_frame(lua: &Lua, t: &mlua::Table) -> mlua::Result<FrameHandle> {
    let Ok(Value::LightUserData(p)) = t.raw_get::<Value>(0) else {
        return Err(mlua::Error::runtime(
            "CreateFrame: Couldn't find 'this' in parent object",
        ));
    };
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    model
        .id_to_frame
        .get(&(p.0 as usize as u32))
        .copied()
        .ok_or_else(|| {
            mlua::Error::runtime("CreateFrame: Wrong parent object type, expected frame")
        })
}
