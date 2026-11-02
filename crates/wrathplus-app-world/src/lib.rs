//! The world layer of wrathplus-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

pub mod app_icon;
pub mod area_poi;
#[cfg(feature = "dev")]
pub mod asset_churn;
pub mod aura_visual;
pub mod blob_shadow;
pub mod bowstring;
pub mod camera_shake;
pub mod chat_bubble;
pub mod cinematic;
pub mod combat_text;
pub mod creature_anim;
pub mod cursor;
pub mod death;
pub mod doodad_events;
pub mod entities;
pub mod experience;
pub mod experience_ao;
pub mod fishing_line;
pub mod footprints;
pub mod game_tip;
pub mod go_anim;
pub mod guild;
pub mod inventory;
pub mod loading_screen;
pub mod loot;
pub mod minimap;
pub mod motion;
pub mod nameplates;
pub mod opaque2d;
pub mod overlay;
pub mod pipe_warm;
pub mod player;
pub mod poi_marker;
pub mod portrait;
pub mod quest_markers;
pub mod raid_marks;
pub mod ranged_flex;
pub mod screen_fade;
pub mod screenshot;
pub mod shaders;
pub mod smart_rect;
pub mod sound;
pub mod spell;
pub mod swing_refusal;
pub mod target;
pub mod transport;
pub mod ui_text;
pub mod video;
pub mod vplates;
pub mod weapon_trail;
pub mod world_backdrop;
pub mod world_state;

// Every lower layer's modules at this crate's root, so `crate::<module>::…` resolves.
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::app_core;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::area;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::chr_classes;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::crash;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::cvars;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::glue_strings;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::launch_plan;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::local_state;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::pending_item_ops;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::query_cache;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::quest_reputation;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::run_mode;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::shutdown;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::text_filter;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::text_reshape;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_core::wrath_vehicle_seats;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::area_trigger;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::creature_type;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::go_templates;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::items;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::name_persist;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::names;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::net;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_net::realmlist;
