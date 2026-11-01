//! The shipped CVar rows that leave build 12340's registered defaults (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-CVARS`).

use crate::cvars::Reference;

#[test]
fn the_options_that_leave_the_reference_are_this_list_and_no_other() {
    let mut names: Vec<&str> = crate::shipped_cvars::rows()
        .filter(|r| matches!(r.reference, Reference::Deviates { .. }))
        .map(|r| r.name)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "ffxDeath",
            "ffxGlow",
            "frillDensity",
            "gxApi",
            "gxColorBits",
            "gxDepthBits",
            "gxResolution",
            "nearclip",
            "realmList",
            "spellEffectLevel",
            "weatherDensity",
        ],
    );
}
