//! What every layer of the app reads: the schedule sets systems order themselves on, the shared
//! game state and UI error types, the movement model the wire and the world share, and the death
//! state the wire fills. The first piece of the crate split (`https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN`,
//! step 0); the owning modules re-export from their old paths.

pub mod anim;
pub mod arena;
pub mod catalogs;
pub mod chat;
pub mod console;
pub mod cooldowns;
pub mod death;
pub mod errors;
pub mod gather;
pub mod group;
pub mod interaction;
pub mod loot;
pub mod movement;
pub mod player;
pub mod raid_rows;
pub mod schedule;
pub mod session;
#[cfg(any(test, feature = "test-support"))]
pub mod source_tree;
pub mod state;
pub mod tutorial;
pub mod ui_flags;
pub mod ui_state;
pub mod vm_memo;
