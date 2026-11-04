//! The `wrathplus` launcher: a shim that carries the build stamp, so a new commit recompiles these
//! few lines and relinks instead of rebuilding `wrathplus-app`.

use wrathplus_app::BuildId;

fn main() -> wrathplus_app::AppExit {
    wrathplus_app::run(BuildId {
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
