//! The startup cover: the black loading cover, up before the login screen draws, so the pipeline
//! warm pass ([`crate::pipe_warm`]) compiles every reachable variant before the first visible frame
//! rather than on the glue screens, where a compile is a visible stall. Shaders up front, even if
//! loading is longer (Shannon, 2026-10-27). The glue's entry waits for it
//! ([`LoadingScreen::startup_covering`]), and world entry then finds the process warmed.
//!
//! It lifts once a warm pass has run to its end, drained or released by its own nets. It never
//! waits on residency: nothing streams behind the glue.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::LoadingScreen;

/// How long the cover waits for a warm pass to start before it lifts anyway. The pass needs the
/// world camera and the shared light; a client without them must still reach its login.
const START_GRACE_SECS: f32 = 15.0;

pub(super) fn build(app: &mut App) {
    app.add_systems(
        Startup,
        raise_startup_cover.after(super::setup_loading_screen),
    );
}

/// Raise it wherever a warm pass runs: not in a capture or, by default, a headless client
/// ([`crate::pipe_warm::runs_a_menagerie`]), which have nothing to wait for.
fn raise_startup_cover(
    mut screen: ResMut<LoadingScreen>,
    window: Query<(), With<PrimaryWindow>>,
    time: Res<Time<Real>>,
) {
    if window.is_empty() || !crate::pipe_warm::runs_a_menagerie() {
        return;
    }
    screen.raise("startup, pipeline warm", false, None, time.elapsed_secs());
    screen.startup = true;
}

/// What the startup cover does this frame.
#[derive(Debug, PartialEq)]
enum Release {
    /// A warm pass has run to its end.
    Warmed,
    /// No pass started within [`START_GRACE_SECS`].
    NoPass,
    Hold,
}

fn release_rule(finished: bool, started: bool, up_for: f32) -> Release {
    if finished {
        Release::Warmed
    } else if !started && up_for >= START_GRACE_SECS {
        Release::NoPass
    } else {
        Release::Hold
    }
}

/// Lift the startup cover once its warm pass is over.
pub(super) fn drive(screen: &mut LoadingScreen, warm: &crate::pipe_warm::WarmPass, now: f32) {
    if !screen.startup {
        return;
    }
    let up_for = now - screen.active_since;
    match release_rule(warm.finished(), warm.started(), up_for) {
        Release::Hold => {}
        Release::Warmed => {
            screen.dismiss();
            info!("loading screen: cleared (startup, {up_for:.1}s)");
        }
        Release::NoPass => {
            screen.dismiss();
            error!(
                "loading screen: startup cover lifted after {up_for:.1}s with no warm pass \
                 started (no world camera or shared light); the glue screens compile live"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_cover_holds_for_the_pass_and_never_strands_the_login() {
        // A pass under way holds it however long it runs; the pass has its own nets.
        assert_eq!(release_rule(false, true, 0.5), Release::Hold);
        assert_eq!(release_rule(false, true, 120.0), Release::Hold);
        assert_eq!(release_rule(true, true, 9.0), Release::Warmed);
        // No pass yet: a slow first frame holds, a client that can't warm lifts at the grace.
        assert_eq!(release_rule(false, false, 3.0), Release::Hold);
        assert_eq!(
            release_rule(false, false, START_GRACE_SECS),
            Release::NoPass
        );
    }

    #[test]
    fn a_startup_raise_is_black_counts_as_a_cover_and_only_the_pass_lifts_it() {
        let mut screen = LoadingScreen::default();
        screen.raise("startup, pipeline warm", false, None, 0.0);
        screen.startup = true;
        assert!(screen.covering() && screen.startup_covering());
        assert_eq!(screen.map, None, "no map: no art, a black cover");

        let mut warm = crate::pipe_warm::WarmPass::default();
        drive(&mut screen, &warm, 5.0);
        assert!(screen.covering(), "a pass not yet run holds it");
        warm.mark_finished_for_test();
        drive(&mut screen, &warm, 9.0);
        assert!(!screen.covering() && !screen.startup_covering());
    }

    #[derive(Resource, Default)]
    struct Ran(u32);

    fn count(mut ran: ResMut<Ran>) {
        ran.0 += 1;
    }

    /// The glue theme's gate: held under the startup cover, free once it lifts or with no screen.
    #[test]
    fn the_glue_theme_waits_out_the_startup_cover() {
        let mut app = App::new();
        app.init_resource::<Ran>()
            .add_systems(Update, count.run_if(super::super::no_startup_cover));
        app.update();
        assert_eq!(
            app.world().resource::<Ran>().0,
            1,
            "no loading screen at all"
        );

        let mut screen = LoadingScreen::default();
        screen.raise("startup, pipeline warm", false, None, 0.0);
        screen.startup = true;
        app.insert_resource(screen);
        app.update();
        assert_eq!(app.world().resource::<Ran>().0, 1, "held under the cover");

        app.world_mut().resource_mut::<LoadingScreen>().dismiss();
        app.update();
        assert_eq!(app.world().resource::<Ran>().0, 2, "free once it lifts");
    }
}
