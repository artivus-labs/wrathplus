//! A spell render's fill, and the spell it records for `GameTooltip:GetSpell` (`+0x364`).

use mlua::{Lua, Table};

use super::{
    append_line, drawn_lines, render_spell, spell_view_of, tip_mut, FrameHandle, Model,
    SpellRenderOpts, SpellWait, WHITE,
};

/// Fill a cleared tooltip with the spell's view, or with the fallback name and a wait on the ask.
pub(super) fn fill_spell(
    lua: &Lua,
    this: &Table,
    h: FrameHandle,
    spell_id: u32,
    fallback_name: Option<String>,
    opts: SpellRenderOpts,
    remaining: Option<String>,
) -> mlua::Result<()> {
    // 12340 records a spell Spell.dbc holds (`0x623941`); an aura render records none (`0x62535c`).
    if !opts.aura {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let known = model.spell_catalog.contains_key(&spell_id);
        tip_mut(&mut model, h)?.spell = known.then_some(spell_id);
    }
    match spell_view_of(lua, spell_id, opts.alt_caster) {
        Some(v) => render_spell(lua, this, &v, opts, remaining, None)?,
        None => {
            if let Some(name) = fallback_name.clone() {
                append_line(lua, this, (name, WHITE), None, false)?;
            }
            // The fallback stands until the app answers the ask; id 0 asks nothing.
            if spell_id != 0 {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let drawn = drawn_lines(&mut model, h)?;
                model.spell_tooltip_waits.insert(
                    h,
                    SpellWait {
                        spell_id,
                        fallback_name,
                        opts,
                        remaining,
                        drawn,
                    },
                );
            }
        }
    }
    super::super::tooltip::finish_content(lua, h, "OnTooltipSetSpell");
    Ok(())
}
