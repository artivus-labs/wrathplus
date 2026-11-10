//! Whether this process runs the warm pass at all.
//!
//! Windowed play warms everything up front: shaders at the outset, zero live compiles in play. A
//! headless agent client (`WOW_HEADLESS=1`) skips the pass and compiles each pipeline as it is
//! first drawn (Shannon, 2026-11-10): on a CPU-only host the pass's 2075 variants cost about
//! 10 GiB resident and 40–75 s of start-up, and nobody watches a headless frame stall.
//!
//! | Env | Meaning |
//! |---|---|
//! | `WOW_PIPE_WARM=1` | run the warm pass even headless |
//! | `WOW_PIPE_WARM=0` | skip it even windowed (pipelines compile live; for testing) |
//!
//! A capture (`WOW_CAPTURE`) never warms, whatever the override: it must stay deterministic.

use std::sync::OnceLock;

/// What this process does about the warm pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmPlan {
    /// The full up-front pass; a pipeline compiled uncovered afterwards is a coverage hole.
    Run,
    /// A capture: no menagerie, so the run is deterministic.
    Capture,
    /// Skipped on purpose (headless, or `WOW_PIPE_WARM=0`): live compiles are expected.
    Skipped,
}

impl WarmPlan {
    /// Whether the menagerie spawns under a loading cover.
    pub(crate) fn runs(self) -> bool {
        self == WarmPlan::Run
    }

    /// Whether an uncovered compile is the plan rather than a bug: the tripwire then logs at
    /// debug instead of warning.
    pub(crate) fn expects_live_compiles(self) -> bool {
        self == WarmPlan::Skipped
    }
}

/// `WOW_PIPE_WARM`'s value: on, off, or unset/unrecognised (no override).
pub(crate) fn parse_override(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The rule: never in a capture; else the override; else every client but a headless one.
pub(crate) fn decide(capture: bool, headless: bool, forced: Option<bool>) -> WarmPlan {
    if capture {
        WarmPlan::Capture
    } else if forced.unwrap_or(!headless) {
        WarmPlan::Run
    } else {
        WarmPlan::Skipped
    }
}

/// This process's plan, read from the environment once.
pub(crate) fn plan() -> WarmPlan {
    static PLAN: OnceLock<WarmPlan> = OnceLock::new();
    *PLAN.get_or_init(|| {
        let raw = std::env::var("WOW_PIPE_WARM").ok();
        let forced = parse_override(raw.as_deref());
        if let (Some(raw), None) = (&raw, forced) {
            bevy::log::warn!("WOW_PIPE_WARM={raw:?} is not 1 or 0; ignored");
        }
        decide(
            wrathplus_world::dev_state::deterministic_run(),
            wrathplus_world::headless::active(),
            forced,
        )
    })
}

/// Say once, at start-up, when the pass is skipped on purpose or forced on headless.
pub(super) fn announce() {
    let headless = wrathplus_world::headless::active();
    match plan() {
        WarmPlan::Skipped if headless => bevy::log::info!(
            "pipeline warm: skipped (headless); pipelines compile as first drawn. \
             WOW_PIPE_WARM=1 runs the pass"
        ),
        WarmPlan::Skipped => bevy::log::warn!(
            "pipeline warm: skipped (WOW_PIPE_WARM=0); pipelines compile live, with visible stalls"
        ),
        WarmPlan::Run if headless => {
            bevy::log::info!("pipeline warm: on in a headless client (WOW_PIPE_WARM=1)");
        }
        WarmPlan::Run | WarmPlan::Capture => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windowed_client_warms_as_before() {
        assert_eq!(decide(false, false, None), WarmPlan::Run);
        assert!(decide(false, false, None).runs());
        assert!(!decide(false, false, None).expects_live_compiles());
    }

    #[test]
    fn a_headless_client_skips_the_pass_and_expects_live_compiles() {
        let plan = decide(false, true, None);
        assert_eq!(plan, WarmPlan::Skipped);
        assert!(!plan.runs());
        assert!(plan.expects_live_compiles());
    }

    #[test]
    fn the_override_forces_either_way() {
        assert_eq!(decide(false, true, Some(true)), WarmPlan::Run);
        assert_eq!(decide(false, false, Some(false)), WarmPlan::Skipped);
        // Agreeing with the default changes nothing.
        assert_eq!(decide(false, false, Some(true)), WarmPlan::Run);
        assert_eq!(decide(false, true, Some(false)), WarmPlan::Skipped);
    }

    #[test]
    fn a_capture_never_warms_and_keeps_its_live_compile_warning() {
        for headless in [false, true] {
            for forced in [None, Some(true), Some(false)] {
                let plan = decide(true, headless, forced);
                assert_eq!(plan, WarmPlan::Capture);
                assert!(!plan.runs() && !plan.expects_live_compiles());
            }
        }
    }

    #[test]
    fn the_override_reads_the_same_words_as_wow_headless() {
        for on in ["1", "true", "yes", "on", " ON "] {
            assert_eq!(parse_override(Some(on)), Some(true), "{on:?}");
        }
        for off in ["0", "false", "no", "off"] {
            assert_eq!(parse_override(Some(off)), Some(false), "{off:?}");
        }
        for neither in ["", "2", "maybe"] {
            assert_eq!(parse_override(Some(neither)), None, "{neither:?}");
        }
        assert_eq!(parse_override(None), None);
    }
}
