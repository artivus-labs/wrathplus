//! The net layer of wrathplus-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

pub mod area_trigger;
pub mod creature_type;
pub mod go_templates;
pub mod items;
pub mod name_persist;
pub mod names;
pub mod net;
pub mod realmlist;

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
