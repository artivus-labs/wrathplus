//! The `wrathplus-worldview` launcher: boots the `wrathplus-world` engine with no game attached, so an
//! engine system that needs a gameplay resource fails here, a coupling no compiler sees. Like
//! `wrathplus`, a shim that carries the build stamp, so a new commit recompiles only these lines.

use wrathplus_world::build_id::BuildId;

fn main() -> wrathplus_world::AppExit {
    wrathplus_world::worldview::run(BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("WRATHPLUS_GIT_DESCRIBE"),
        sha: env!("WRATHPLUS_GIT_SHA"),
        short: env!("WRATHPLUS_GIT_SHORT"),
        date: env!("WRATHPLUS_GIT_DATE"),
        profile: env!("WRATHPLUS_PROFILE"),
        project_dir: env!("WRATHPLUS_PROJECT_DIR"),
        ..Default::default()
    })
}
