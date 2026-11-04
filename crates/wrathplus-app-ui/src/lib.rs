//! The ui layer of wrathplus-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

pub mod bindings;
pub mod char_create;
pub mod char_select;
pub mod combat_log;
pub mod console;
pub mod experience_ui;
pub mod glue;
pub mod login;
pub mod npc_text;
pub mod quest_rewards;
pub mod realm_select;
pub mod script_calls;
pub mod shipped_cvars;
pub mod textinput;
pub mod tutorial;
pub mod ui_achievements;
pub mod ui_action;
pub mod ui_arena_team;
pub mod ui_auction;
pub mod ui_aura;
pub mod ui_bank;
pub mod ui_battlefield;
pub mod ui_battlefield_arena_lines;
pub mod ui_battlefield_positions;
pub mod ui_battlefield_score;
pub mod ui_bind_confirm;
pub mod ui_binder;
pub mod ui_calendar;
pub mod ui_cast;
pub mod ui_char;
pub mod ui_char_state;
pub mod ui_chat;
pub mod ui_chat_verbs;
pub mod ui_companions;
pub mod ui_death;
pub mod ui_dialog_verbs;
pub mod ui_dressup;
pub mod ui_duel;
pub mod ui_follow;
pub mod ui_gamma;
pub mod ui_glyphs;
pub mod ui_gm_response;
pub mod ui_gm_ticket;
pub mod ui_gossip;
pub mod ui_guild;
pub mod ui_guild_bank;
pub mod ui_honor;
pub mod ui_inspect;
pub mod ui_inspect_wrath;
pub mod ui_instance;
pub mod ui_item_text;
pub mod ui_items;
pub mod ui_layout;
pub mod ui_lfg;
pub mod ui_logout;
pub mod ui_loot;
pub mod ui_loot_roll;
pub mod ui_macro;
pub mod ui_mail;
pub mod ui_merchant;
pub mod ui_mirror;
pub mod ui_models;
pub mod ui_native;
pub mod ui_native_chat;
pub mod ui_net;
pub mod ui_paperdoll;
pub mod ui_party;
pub mod ui_pass;
pub mod ui_pet;
pub mod ui_pet_book;
pub mod ui_pet_doll;
pub mod ui_pet_stats;
pub mod ui_pet_talent_tree;
pub mod ui_petition;
pub mod ui_quest;
pub mod ui_quest_log;
pub mod ui_quest_share;
pub mod ui_raid_vehicles;
pub mod ui_reputation;
pub mod ui_runes;
pub mod ui_saved;
pub mod ui_script;
pub mod ui_session;
pub mod ui_shapeshift;
pub mod ui_social;
pub mod ui_spellbook;
pub mod ui_stable;
pub mod ui_static_popup;
pub mod ui_summon;
pub mod ui_tabard;
pub mod ui_talent;
pub mod ui_talent_wipe;
pub mod ui_taxi;
pub mod ui_tooltip;
pub mod ui_totems;
pub mod ui_trade;
pub mod ui_tradeskill;
pub mod ui_trainer;
pub mod ui_unit;
pub mod ui_unit_popup;
pub mod ui_unit_quest_tooltip;
pub mod ui_vehicle;
pub mod ui_world_map;
pub mod world_state_ui;
pub mod wrathplus_pack;

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
