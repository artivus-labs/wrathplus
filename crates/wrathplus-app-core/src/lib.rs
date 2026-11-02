//! The core layer of wrathplus-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

pub mod app_core;
pub mod area;
pub mod chr_classes;
pub mod crash;
pub mod cvars;
pub mod data_config;
pub mod glue_strings;
pub mod launch_plan;
pub mod local_state;
pub mod pending_item_ops;
pub mod query_cache;
pub mod quest_reputation;
pub mod run_mode;
pub mod shutdown;
pub mod text_filter;
pub mod text_reshape;
pub mod wrath_vehicle_seats;
