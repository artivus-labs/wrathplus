//! `extended_launcher`: the client with a plugin of its own on top, started through
//! [`wrathplus_app::run_with`] as a crate built on WrathPlus starts it. A feature 1.12.1 does not have
//! lives in a crate like this one, never in wrathplus.
//!
//! ```text
//! cargo run -p wrathplus-app --example extended_launcher
//! ```
//!
//! The banner reads `wrathplus build … · extended`, and the plugin logs one line at `Startup`.
//!
//! A crate of its own copies the `wrathplus` launcher (`crates/wrathplus`): a build script that stamps
//! its commit, a `main` like this one that reads the stamp back as `crates/wrathplus/src/main.rs`
//! does, and a manifest that takes WrathPlus as the workspace does:
//!
//! ```toml
//! [features]
//! default = ["dev"]
//! # The instruments; the player build is `--no-default-features`.
//! dev = ["wrathplus-app/dev"]
//!
//! [dependencies]
//! # `default-features = false`, or `--no-default-features` still builds wrathplus-app with `dev`.
//! wrathplus-app = { git = "https://github.com/artivus-labs/wrathplus", default-features = false }
//! # WrathPlus's version, adding no features: bevy's defaults bring its audio stack back.
//! bevy = { version = "0.18.1", default-features = false }
//!
//! [build-dependencies]
//! wrathplus-buildstamp = { git = "https://github.com/artivus-labs/wrathplus" }
//!
//! # Cargo reads `[patch]` and `[profile]` from the root manifest only, so WrathPlus's are repeated:
//! # without the patches the addon Lua and the mixer are not WrathPlus's.
//! [patch.crates-io]
//! lua-src = { git = "https://github.com/artivus-labs/wrathplus" }
//! kira = { git = "https://github.com/artivus-labs/wrathplus" }
//!
//! [profile.dev]
//! opt-level = 1
//!
//! [profile.dev.package."*"]
//! opt-level = 3
//! ```
//!
//! ```text
//! // build.rs
//! fn main() {
//!     wrathplus_buildstamp::emit();
//! }
//! ```
//!
//! Its `Cargo.lock` starts as a copy of WrathPlus's: cargo reads only the root's lockfile too, and
//! without it resolves every crate to its newest compatible version, not the one WrathPlus is built
//! with. A dev build keeps its `WoW` link, `wrathplus-config/` and `.probe-identity` in its own
//! folder, beside that `Cargo.lock`, as WrathPlus's does.

use bevy::prelude::*;
use wrathplus_app::{AppExit, BuildId};

/// The crate's own plugin: one line at `Startup`, to show it runs inside the client.
struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, || info!("a plugin on top of wrathplus is running"));
    }
}

fn main() -> AppExit {
    // A launcher of its own passes the stamp its build script made; an example has no build
    // script, so this names the version and profile alone.
    let build = BuildId {
        version: env!("CARGO_PKG_VERSION"),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        ..Default::default()
    };
    wrathplus_app::run_with(build, |app| {
        app.add_plugins(HelloPlugin);
    })
}
