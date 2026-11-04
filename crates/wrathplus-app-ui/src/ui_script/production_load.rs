//! The production in-game load over a hermetic AddOns root, for the tests that load third-party
//! addons alongside the stock interface.

use wrathplus_ui::script::UiScript;

use crate::local_state::test_env::EnvGuard;

/// The production in-game load ([`super::manifest::load_session_ui`] with addons) over a hermetic
/// AddOns root that `addons` fills; `before` runs ahead of the load. The caller holds
/// [`crate::local_state::test_env::ENV_LOCK`]. Returns the VM and the load's failures.
pub(super) fn production_load_with(
    tag: &str,
    before: &str,
    addons: impl FnOnce(&std::path::Path),
) -> (UiScript, Vec<String>) {
    let tmp = std::env::temp_dir().join(format!("wrathplus-load-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let home = tmp.join("wrathplus-config");
    let root = home.join("AddOns");
    std::fs::create_dir_all(&root).unwrap();
    addons(&root);
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::set("WRATHPLUS_HOME", home.to_str().unwrap());

    let mut s = UiScript::new().unwrap();
    // A reply that hid nothing, so `GetNumAddOns` counts the registry.
    s.note_addon_info_reply(&[]);
    s.run(before).unwrap();
    let failures = super::manifest::load_session_ui(&mut s, true);
    let _ = std::fs::remove_dir_all(&tmp);
    (s, failures)
}
