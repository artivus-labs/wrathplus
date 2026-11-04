//! `dump_widget_methods`: print WrathPlus's widget method surface, asked of a real VM.
//!
//! ```text
//! cargo run -q -p wrathplus-ui --example dump_widget_methods    # class<TAB>method, one per line
//! ```
//!
//! The census is `widget_method_census`, the same function the widget-surface gate reads.
use wrathplus_ui::script::{widget_method_census, UiScript};

fn main() -> mlua::Result<()> {
    for (class, method) in widget_method_census(&UiScript::new()?)? {
        println!("{class}\t{method}");
    }
    Ok(())
}
