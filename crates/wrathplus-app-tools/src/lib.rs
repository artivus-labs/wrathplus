//! The tools layer of wrathplus-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

pub mod addon_harness;
#[cfg(feature = "dev")]
pub mod capture;
#[cfg(feature = "dev")]
pub mod debug_panel;
pub mod dev;
#[cfg(feature = "dev")]
pub mod hover_log;
pub mod perf;
#[cfg(feature = "dev")]
pub mod preflight;
#[cfg(feature = "dev")]
pub mod probe_shield;
#[cfg(feature = "dev")]
pub mod showcase_capture;

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
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::bindings;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::char_create;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::char_select;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::combat_log;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::console;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::experience_ui;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::glue;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::login;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::npc_text;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::quest_rewards;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::realm_select;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::script_calls;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::shipped_cvars;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::textinput;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::tutorial;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_achievements;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_action;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_arena_team;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_auction;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_aura;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_bank;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_battlefield;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_battlefield_arena_lines;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_battlefield_positions;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_battlefield_score;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_bind_confirm;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_binder;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_calendar;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_cast;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_char;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_chat;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_chat_verbs;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_companions;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_death;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_dialog_verbs;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_dressup;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_duel;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_follow;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_gamma;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_glyphs;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_gm_ticket;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_gossip;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_guild;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_guild_bank;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_honor;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_inspect;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_inspect_wrath;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_instance;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_item_text;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_items;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_layout;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_lfg;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_logout;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_loot;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_loot_roll;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_macro;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_mail;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_merchant;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_mirror;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_models;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_native;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_native_chat;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_net;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_paperdoll;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_party;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pass;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pet;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pet_book;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pet_doll;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pet_stats;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_pet_talent_tree;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_petition;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_quest;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_quest_log;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_quest_share;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_raid_vehicles;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_reputation;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_runes;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_saved;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_script;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_session;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_shapeshift;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_social;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_spellbook;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_stable;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_static_popup;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_summon;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_tabard;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_talent;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_talent_wipe;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_taxi;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_tooltip;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_totems;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_trade;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_tradeskill;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_trainer;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_unit;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_unit_popup;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_vehicle;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::ui_world_map;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_ui::world_state_ui;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::area_poi;
#[cfg(feature = "dev")]
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::asset_churn;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::aura_visual;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::blob_shadow;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::bowstring;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::camera_shake;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::chat_bubble;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::cinematic;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::combat_text;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::creature_anim;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::cursor;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::death;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::doodad_events;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::entities;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::experience;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::experience_ao;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::fishing_line;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::footprints;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::game_tip;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::go_anim;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::guild;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::inventory;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::loading_screen;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::loot;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::minimap;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::motion;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::nameplates;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::opaque2d;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::overlay;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::pipe_warm;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::player;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::poi_marker;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::portrait;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::quest_markers;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::raid_marks;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::ranged_flex;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::screen_fade;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::screenshot;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::shaders;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::smart_rect;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::sound;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::spell;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::swing_refusal;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::target;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::transport;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::ui_text;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::video;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::vplates;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::weapon_trail;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::world_backdrop;
#[allow(unused_imports)]
pub(crate) use wrathplus_app_world::world_state;
