//! Frame attributes: one store per frame object (`+0x1cc`), its names matched case-insensitively.

use mlua::{Lua, Table, Value};

use super::super::object::decode_id;
use super::super::Model;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let methods: Table = lua.named_registry_value(super::super::REG_FRAME_METHODS)?;
    // `OnAttributeChanged` (`0x48ec10`) runs on the frame through the handler ABI and `lua_pcall`.
    let changed = lua.create_function(|lua, (this, name, value): (Table, Value, Value)| {
        let Ok(id) = decode_id(&this) else {
            return Ok(());
        };
        let fired = super::super::event::fire_widget_handler(
            lua,
            id,
            "OnAttributeChanged",
            vec![name, value],
        );
        if let Err(e) = fired {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .record_script_error(e.to_string());
        }
        Ok(())
    })?;
    lua.load(
        r##"
        local methods, changed = ...
        local lower, format = string.lower, string.format
        -- One store per frame, by its `[0]`, so a stand-in shares it (`RestrictedFrames.lua:96`).
        local stores = {}
        local function text(v) local kind = type(v) return kind == "string" or kind == "number" end
        -- A string or number names an attribute, lowered; else the usage error (`0x9ece00`).
        local function named(self, name, usage)
            if not text(name) then error(format(usage, self:GetName() or "<unnamed>"), 0) end
            return lower(name)
        end
        function methods.SetAttribute(self, name, value)
            name = named(self, name, 'Usage: %s:SetAttribute("name", value)')
            local key = rawget(self, 0) or self
            local own = stores[key]
            if not own then own = {} stores[key] = own end
            own[name] = value
            changed(self, name, value)
        end
        -- Three arguments, a name second: p..n..s, *..n..s, p..n..*, *..n..*, then n (`0x49f30f`).
        function methods.GetAttribute(self, ...)
            local prefix, name, suffix = ...
            if select("#", ...) ~= 3 or not text(name) then
                prefix = named(self, prefix, 'Usage: %s:GetAttribute("name")')
                local own = stores[rawget(self, 0) or self]
                return own and own[prefix]
            end
            local own = stores[rawget(self, 0) or self]
            if not own then return nil end
            -- An affix that is no string reads empty: `lua_tolstring`'s zero length (`0x84e10f`).
            prefix = text(prefix) and lower(prefix) or ""
            suffix = text(suffix) and lower(suffix) or ""
            name = lower(name)
            local value = own[prefix .. name .. suffix]
            if value == nil then value = own["*" .. name .. suffix] end
            if value == nil then value = own[prefix .. name .. "*"] end
            if value == nil then value = own["*" .. name .. "*"] end
            if value == nil then value = own[name] end
            return value
        end
        "##,
    )
    .set_name("=[AzerothBench Wrath frame attributes]")
    .call::<()>((methods, changed))
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    /// Modified lookup is ordered by specificity; false is a value, and nil removes a row.
    #[test]
    fn modified_attributes_preserve_false_and_fall_back_after_removal() {
        let script = UiScript::new().unwrap();
        script
            .run(
                r#"
                local f, other = CreateFrame('Frame'), CreateFrame('Frame')
                f:SetAttribute('type', 'plain')
                f:SetAttribute('*type*', 'both')
                f:SetAttribute('shift-type*', 'prefix')
                f:SetAttribute('*type1', 'suffix')
                f:SetAttribute('shift-type1', false)
                local function get() return f:GetAttribute('SHIFT-', 'TYPE', '1') end
                assert(get() == false, 'false must not fall through')
                f:SetAttribute('SHIFT-TYPE1', nil)
                assert(get() == 'suffix', 'wildcard prefix precedes wildcard suffix')
                f:SetAttribute('*TYPE1', nil)
                assert(get() == 'prefix')
                f:SetAttribute('SHIFT-TYPE*', nil)
                assert(get() == 'both')
                f:SetAttribute('*TYPE*', nil)
                assert(get() == 'plain')
                f:SetAttribute('TYPE', nil)
                assert(get() == nil)
                assert(other:GetAttribute('type') == nil, 'no cross-frame fallback')
                "#,
            )
            .unwrap();
    }

    #[test]
    fn attributes_store_per_frame_and_fire_on_attribute_changed() {
        let script = UiScript::new().unwrap();
        let doc = crate::framexml::parse(
            r#"<Ui><Button name="Unitish"><Scripts>
                <OnAttributeChanged>SEEN = name .. "=" .. tostring(value)</OnAttributeChanged>
            </Scripts></Button></Ui>"#,
        )
        .unwrap();
        let report = crate::loader::load(&script, &doc, &|_| None);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(script
            .eval::<bool>(
                "Unitish:SetAttribute('unit', 'target') \
                 local other = CreateFrame('Frame') \
                 return Unitish:GetAttribute('unit') == 'target' and SEEN == 'unit=target' \
                    and other:GetAttribute('unit') == nil and Unitish:IsProtected() == nil"
            )
            .unwrap());
    }

    /// Names fold, a stand-in sharing `[0]` reaches the store, and the handler runs on the frame.
    #[test]
    fn attributes_fold_case_and_belong_to_the_frame_not_its_table() {
        let mut script = UiScript::new().unwrap();
        assert!(script
            .eval::<bool>(
                "local f, seen, seen_self, seen_this = CreateFrame('Frame') \
                 f:SetScript('OnAttributeChanged', function(s, name) \
                     seen, seen_self, seen_this = name, s, this end) \
                 f:SetAttribute('FooBar', 1) \
                 local stand_in = setmetatable({ [0] = f[0], [1] = f }, getmetatable(f)) \
                 stand_in:SetAttribute('Shift-Type1', 'spell') \
                 local folded = seen == 'shift-type1' and seen_self == f and seen_this == f \
                    and f:GetAttribute('foobar') == 1 and stand_in:GetAttribute('FOOBAR') == 1 \
                    and f:GetAttribute('SHIFT-', 'type', '1') == 'spell' \
                 f:SetScript('OnAttributeChanged', function() error('boom') end) \
                 return folded and pcall(f.SetAttribute, f, 'after', 2) \
                    and f:GetAttribute('after') == 2"
            )
            .unwrap());
        assert!(script.take_errors().iter().any(|e| e.contains("boom")));
    }

    /// A name that is no string or number raises the usage error, with or without attributes.
    #[test]
    fn a_name_that_is_no_string_raises_the_usage_error() {
        let script = UiScript::new().unwrap();
        assert_eq!(
            script
                .eval::<String>(
                    "local held, bare = CreateFrame('Frame', 'Holder'), CreateFrame('Frame') \
                     held:SetAttribute(5, 'five') \
                     local function raised(f, ...) \
                         local ok, e = pcall(f, ...) return ok and '<none>' or e end \
                     return raised(bare.GetAttribute, bare, nil) .. '|' \
                         .. raised(held.GetAttribute, held, {}) .. '|' \
                         .. raised(held.SetAttribute, held, true, 1) .. '|' \
                         .. held:GetAttribute('5')"
                )
                .unwrap(),
            "Usage: <unnamed>:GetAttribute(\"name\")|Usage: Holder:GetAttribute(\"name\")|\
             Usage: Holder:SetAttribute(\"name\", value)|five"
        );
    }

    /// The modified lookup takes exactly three arguments with a name; else the first is the name.
    #[test]
    fn the_modified_lookup_takes_three_arguments_and_reads_other_affixes_as_empty() {
        let script = UiScript::new().unwrap();
        assert_eq!(
            script
                .eval::<String>(
                    "local f = CreateFrame('Frame') \
                     f:SetAttribute('type', 'plain') f:SetAttribute('shift-', 'affix') \
                     return tostring(f:GetAttribute({}, 'type', true)) .. '|' \
                         .. tostring(f:GetAttribute('shift-', 'type')) .. '|' \
                         .. tostring(f:GetAttribute('shift-', {}, '1')) .. '|' \
                         .. tostring(f:GetAttribute('nope-', 'type', nil))"
                )
                .unwrap(),
            "plain|affix|affix|plain"
        );
    }
}
