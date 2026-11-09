//! The control port an external agent drives one client through (issue #59): local HTTP on
//! 127.0.0.1 with a bearer token, opened only when the launch asks for it (`--agent-port <port>`
//! or `WOW_AGENT_PORT`, with the token in `WOW_AGENT_TOKEN`; never on the command line).
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /status` | client state (login, character select, in world) and, in world, the player |
//! | `POST /lua[?budget=N]` | run the body in the UI's Lua; values, `print` output or the error |
//! | `GET /screenshot[?scale=0.5][&format=jpeg[&quality=85]]` | the next rendered frame, PNG or JPEG |
//! | `GET /events?since=N[&wait_ms=M][&limit=K]` | every UI event since cursor `N`, long-polling |
//! | `GET /view?token=…` | a read-only page for people: the live frame, the character, events and the agent's Lua |
//! | `GET /files`, `GET|PUT|DELETE /files/<module>` | the agent's own Lua modules (see `env`) |
//!
//! In Lua, besides the stock API: `AgentNearby([yards])`, `AgentFindNpc(service [, yards])`,
//! `AgentPosition()`, `AgentTarget(guid)`, `AgentInteract(guid)` (`world`), `AgentQuestPOI(quest)`
//! (`quest_poi`), `AgentZones()` (`zones`), `AgentPath`, `AgentMoveTo`, `AgentStop`, `AgentWalk` (`nav`,
//! over the realm's mmaps from `WOW_NAV_MMAPS`), and the module functions (`env`).
//!
//! With the port open, the UI's movement verbs run from any Lua (stock gates them on a key
//! binding), so an agent's scripts can walk. Everything else an agent does goes through the same
//! Lua API the stock interface uses.
//!
//! Each Lua call is also logged on the event stream as `AGENT_LUA` (code, ok, error), for watchers,
//! and counts as input for the idle handler, so a driven client is not seated, AFK or logged out.
//!
//! A headless client (`WOW_HEADLESS=1`, `wrathplus_world::headless`) answers screenshots from its
//! offscreen frame.

mod env;
mod events;
mod lua;
mod nav;
mod quest_poi;
mod server;
mod world;
mod zones;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use crossbeam_channel::Receiver;
use events::EventLog;
use serde_json::{json, Value};
use server::{Format, Job, Port};
use std::sync::Arc;
use std::time::Instant;
use wrathplus_app_core::app_core::state::ClientState;
use wrathplus_ui::script::{ScriptValue, UiScript};

/// Instruction bound for one agent chunk unless the request names one: the same bound an addon
/// load gets, about 50x the heaviest addon, so only a runaway loop meets it.
const DEFAULT_BUDGET: u64 = 200_000_000;

/// The player summary `/status` reports in world, read through the stock API.
const STATUS_LUA: &str = r#"local x, y = GetPlayerMapPosition("player")
return {
  name = UnitName("player"), level = UnitLevel("player"),
  class = select(2, UnitClass("player")), class_name = UnitClass("player"),
  race = select(2, UnitRace("player")),
  health = UnitHealth("player"), health_max = UnitHealthMax("player"),
  power = UnitPower("player"), power_max = UnitPowerMax("player"),
  dead = UnitIsDeadOrGhost("player") and true or false,
  combat = UnitAffectingCombat("player") and true or false,
  zone = GetRealZoneText(), subzone = GetSubZoneText(), map_x = x, map_y = y,
  target = UnitName("target"),
}"#;

/// Opens the control port when the launch asks for one; does nothing otherwise.
pub struct AgentPlugin;

impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        let Some(port) = wrathplus_app_core::launch_plan::plan().agent_port else {
            return;
        };
        let token = match std::env::var("WOW_AGENT_TOKEN") {
            Ok(token) if token.len() >= 16 => token,
            _ => {
                error!("agent port: WOW_AGENT_TOKEN (16+ characters) is required; port not opened");
                return;
            }
        };
        let (jobs, queue) = crossbeam_channel::unbounded();
        let events = Arc::new(EventLog::new());
        match server::start(
            port,
            Port {
                token,
                jobs,
                events: events.clone(),
                scripts: env::scripts_dir(),
            },
        ) {
            Ok(address) => info!("agent port: listening on http://{address}"),
            Err(e) => {
                error!("agent port: {e}");
                return;
            }
        }
        let (world, world_link) = world::link();
        let started = Instant::now();
        let (nav, nav_link) = nav::link(events.clone(), started, world.orders.clone());
        let (pois, poi_link) = quest_poi::link();
        let (zones, zone_link) = zones::link();
        app.insert_resource(Link {
            world,
            nav,
            pois,
            zones,
            queue,
            events,
            started,
            scripts: env::scripts_dir(),
            env_session: 0,
            autoexec_due: false,
        })
        .insert_resource(world_link)
        .insert_resource(nav_link)
        .insert_resource(poi_link)
        .insert_resource(zone_link)
        .add_systems(Update, (pump_events, serve_jobs).chain())
        .add_systems(
            Update,
            (
                world::snapshot,
                world::apply_orders,
                nav::walk,
                quest_poi::mirror,
                zones::load,
            ),
        );
    }
}

#[derive(Resource)]
struct Link {
    queue: Receiver<Job>,
    events: Arc<EventLog>,
    started: Instant,
    /// The agent's modules (`env`).
    scripts: std::path::PathBuf,
    /// The VM session the environment was last installed into.
    env_session: u64,
    /// An `autoexec` module waits for this VM to reach the world.
    autoexec_due: bool,
    /// The Lua side of the world snapshot and orders (`world`).
    world: world::Handles,
    /// Routes and the path walker (`nav`).
    nav: nav::Handles,
    /// The quests' map areas (`quest_poi`).
    pois: quest_poi::Handles,
    /// The zones and their levels (`zones`).
    zones: zones::Handles,
}

/// Keep the VM's agent hooks on (a relog or `ReloadUI()` builds a fresh VM) and move this
/// frame's events into the log.
fn pump_events(
    mut link: ResMut<Link>,
    script: Option<NonSend<UiScript>>,
    state: Option<Res<State<ClientState>>>,
) {
    let Some(script) = script else {
        return;
    };
    script.set_event_tap(true);
    script.set_agent_input(true);
    if link.env_session != script.session() {
        link.env_session = script.session();
        match env::install(&script, link.scripts.clone()) {
            Ok(autoexec) => link.autoexec_due = autoexec,
            Err(e) => error!("agent port: Lua environment: {e}"),
        }
        if let Err(e) = world::install(script.lua(), &link.world) {
            error!("agent port: world functions: {e}");
        }
        if let Err(e) = quest_poi::install(script.lua(), &link.pois) {
            error!("agent port: quest POIs: {e}");
        }
        if let Err(e) = zones::install(script.lua(), &link.zones) {
            error!("agent port: zones: {e}");
        }
        if let Err(e) = nav::install(script.lua(), &link.nav, &link.world) {
            error!("agent port: navigation functions: {e}");
        }
        if let Err(e) = install_signal(script.lua(), link.events.clone(), link.started) {
            error!("agent port: AgentSignal: {e}");
        }
    }
    if link.autoexec_due && state.is_some_and(|s| *s.get() == ClientState::InWorld) {
        link.autoexec_due = false;
        let reply = lua::run(&script, "return require(\"autoexec\")", DEFAULT_BUDGET);
        link.events.push([json!({
            "time": link.started.elapsed().as_secs_f64(),
            "event": "AGENT_LUA",
            "args": ["require(\"autoexec\")", reply["ok"], reply.get("error")],
        })]);
    }
    let taken = script.take_tapped_events();
    let time = link.started.elapsed().as_secs_f64();
    let dropped = (taken.dropped > 0)
        .then(|| json!({"time": time, "event": "AGENT_EVENTS_DROPPED", "args": [taken.dropped]}));
    link.events.push(
        taken
            .events
            .into_iter()
            .map(|(event, args)| {
                json!({
                    "time": time,
                    "event": event,
                    "args": args.iter().map(script_value).collect::<Vec<_>>(),
                })
            })
            .chain(dropped),
    );
}

/// The askers of the screenshot in flight: its scale, encoding and reply each.
type Shots = std::sync::Arc<
    std::sync::Mutex<
        Vec<(
            f32,
            Format,
            crossbeam_channel::Sender<Result<Vec<u8>, String>>,
        )>,
    >,
>;

fn serve_jobs(
    mut commands: Commands,
    link: Res<Link>,
    script: Option<NonSend<UiScript>>,
    state: Option<Res<State<ClientState>>>,
    headless: Option<Res<wrathplus_world::headless::HeadlessFrame>>,
    time: Res<Time<bevy::time::Real>>,
    mut idle: Option<ResMut<wrathplus_app_ui::ui_chat::idle::LastInput>>,
    shots: Local<Shots>,
    mut shot_at: Local<Option<f64>>,
) {
    for job in link.queue.try_iter() {
        match job {
            Job::Lua {
                code,
                budget,
                reply,
            } => {
                // An agent's Lua is its input: without this the stock idle handler sits the
                // player and marks them AFK at 5 minutes, and logs out at 30 (#82).
                if let Some(idle) = idle.as_mut() {
                    idle.stamp_present(time.elapsed());
                }
                let answer = match &script {
                    Some(script) => lua::run(script, &code, budget.unwrap_or(DEFAULT_BUDGET)),
                    None => {
                        json!({"ok": false, "error": "the UI is loading; try again", "output": []})
                    }
                };
                // Into the event log too, so a watcher (the web view) sees what the agent ran.
                let shown: String = code.chars().take(2_000).collect();
                link.events.push([json!({
                    "time": link.started.elapsed().as_secs_f64(),
                    "event": "AGENT_LUA",
                    "args": [shown, answer["ok"], answer.get("error")],
                })]);
                let _ = reply.send(answer);
            }
            Job::Status { reply } => {
                let state = state.as_ref().map(|s| *s.get());
                let in_world = state == Some(ClientState::InWorld);
                let player = match (&script, in_world) {
                    (Some(script), true) => {
                        let reply = lua::run(script, STATUS_LUA, DEFAULT_BUDGET);
                        reply["values"].get(0).cloned().unwrap_or(reply)
                    }
                    _ => Value::Null,
                };
                // Out of the world the glue API (GetNumCharacters, CreateCharacter...) installs a
                // few seconds after the VM: ready only once it is there, so an agent never calls
                // a character-select function that is still nil.
                let ui_ready = match &script {
                    Some(script) if !in_world => script
                        .eval::<bool>("return type(GetNumCharacters) == 'function'")
                        .unwrap_or(false),
                    Some(_) => true,
                    None => false,
                };
                let _ = reply.send(json!({
                    "state": state.map(|s| format!("{s:?}")),
                    "in_world": in_world,
                    "ui_ready": ui_ready,
                    "time": link.started.elapsed().as_secs_f64(),
                    "player": player,
                }));
            }
            Job::Screenshot {
                scale,
                format,
                reply,
            } => {
                // One capture at a time serves everyone who asks while it is in flight: a second
                // capture of the same target is dropped by the renderer ("Duplicate render target
                // for screenshot"), and its asker would wait out the port's timeout.
                // A capture that never lands (2 s is many frames) must not hold the rest: ask again.
                let now = link.started.elapsed().as_secs_f64();
                let mut waiting = shots.lock().unwrap_or_else(|p| p.into_inner());
                let in_flight = !waiting.is_empty() && shot_at.is_some_and(|at| now - at < 2.0);
                waiting.push((scale, format, reply));
                if in_flight {
                    continue;
                }
                drop(waiting);
                *shot_at = Some(now);
                // A headless client draws into an image, not a window.
                let shot = match &headless {
                    Some(frame) => Screenshot::image(frame.0.clone()),
                    None => Screenshot::primary_window(),
                };
                let shots = shots.clone();
                commands
                    .spawn(shot)
                    .observe(move |captured: On<ScreenshotCaptured>| {
                        let image = captured.image.clone();
                        let waiting =
                            std::mem::take(&mut *shots.lock().unwrap_or_else(|p| p.into_inner()));
                        std::thread::spawn(move || {
                            for (scale, format, reply) in waiting {
                                let _ = reply.send(encode(image.clone(), scale, format));
                            }
                        });
                    });
            }
        }
    }
}

/// A captured frame as PNG or JPEG bytes, downscaled by `scale` (0 < scale <= 1).
fn encode(image: Image, scale: f32, format: Format) -> Result<Vec<u8>, String> {
    let mut frame = image
        .try_into_dynamic()
        .map_err(|e| format!("frame conversion: {e}"))?;
    if scale < 1.0 {
        let width = ((frame.width() as f32) * scale).round().max(1.0) as u32;
        let height = ((frame.height() as f32) * scale).round().max(1.0) as u32;
        frame = frame.resize_exact(width, height, image::imageops::FilterType::Triangle);
    }
    let rgb = frame.to_rgb8();
    let mut bytes = Vec::new();
    match format {
        Format::Png => rgb
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("png: {e}"))?,
        Format::Jpeg(quality) => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&rgb)
                .map_err(|e| format!("jpeg: {e}"))?
        }
    }
    Ok(bytes)
}

fn script_value(value: &ScriptValue) -> Value {
    match value {
        ScriptValue::Nil => Value::Null,
        ScriptValue::Bool(b) => json!(b),
        ScriptValue::Int(i) => json!(i),
        ScriptValue::Number(n) if n.is_finite() => json!(n),
        ScriptValue::Number(n) => json!(n.to_string()),
        ScriptValue::Str(s) => json!(s),
        ScriptValue::Bytes(b) => json!(String::from_utf8_lossy(b)),
    }
}

/// `AgentSignal(name, ...)`: put `AGENT_<NAME>` with the arguments on the event stream, so the
/// agent's own Lua can wake the tiers above (an activity ended, a decision is open, anything else).
fn install_signal(lua: &mlua::Lua, events: Arc<EventLog>, started: Instant) -> mlua::Result<()> {
    lua.globals().set(
        "AgentSignal",
        lua.create_function(
            move |_, (name, args): (String, mlua::Variadic<mlua::Value>)| {
                let name = name.to_ascii_uppercase();
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(mlua::Error::runtime(
                        "AgentSignal: names are words and underscores",
                    ));
                }
                events.push([json!({
                    "time": started.elapsed().as_secs_f64(),
                    "event": format!("AGENT_{name}"),
                    "args": args.iter().map(|v| lua::to_json(v, 0)).collect::<Vec<_>>(),
                })]);
                Ok(())
            },
        )?,
    )
}
