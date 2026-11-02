//! The control port an external agent drives one client through (issue #59): local HTTP on
//! 127.0.0.1 with a bearer token, opened only when the launch asks for it (`--agent-port <port>`
//! or `WOW_AGENT_PORT`, with the token in `WOW_AGENT_TOKEN`; never on the command line).
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /status` | client state (login, character select, in world) and, in world, the player |
//! | `POST /lua[?budget=N]` | run the body in the UI's Lua; values, `print` output or the error |
//! | `GET /screenshot[?scale=0.5]` | the next rendered frame as PNG |
//! | `GET /events?since=N[&wait_ms=M][&limit=K]` | every UI event since cursor `N`, long-polling |
//! | `GET /view?token=…` | a read-only page for people: the live frame, the character, events and the agent's Lua |
//! | `GET /files`, `GET|PUT|DELETE /files/<module>` | the agent's own Lua modules (see `env`) |
//!
//! In Lua, besides the stock API: `AgentNearby([yards])`, `AgentPosition()`, `AgentTarget(guid)`,
//! `AgentInteract(guid)` (`world`), and the module functions (`env`).
//!
//! With the port open, the UI's movement verbs run from any Lua (stock gates them on a key
//! binding), so an agent's scripts can walk. Everything else an agent does goes through the same
//! Lua API the stock interface uses.
//!
//! Each Lua call is also logged on the event stream as `AGENT_LUA` (code, ok, error), for watchers.
//!
//! A headless client (`WOW_HEADLESS=1`, `wrathplus_world::headless`) answers screenshots from its
//! offscreen frame.

mod env;
mod events;
mod lua;
mod server;
mod world;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use crossbeam_channel::Receiver;
use events::EventLog;
use serde_json::{json, Value};
use server::{Job, Port};
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
        app.insert_resource(Link {
            world,
            queue,
            events,
            started: Instant::now(),
            scripts: env::scripts_dir(),
            env_session: 0,
            autoexec_due: false,
        })
        .insert_resource(world_link)
        .add_systems(Update, (pump_events, serve_jobs).chain())
        .add_systems(Update, (world::snapshot, world::apply_orders));
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

fn serve_jobs(
    mut commands: Commands,
    link: Res<Link>,
    script: Option<NonSend<UiScript>>,
    state: Option<Res<State<ClientState>>>,
    headless: Option<Res<wrathplus_world::headless::HeadlessFrame>>,
) {
    for job in link.queue.try_iter() {
        match job {
            Job::Lua {
                code,
                budget,
                reply,
            } => {
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
                let _ = reply.send(json!({
                    "state": state.map(|s| format!("{s:?}")),
                    "in_world": in_world,
                    "ui_ready": script.is_some(),
                    "time": link.started.elapsed().as_secs_f64(),
                    "player": player,
                }));
            }
            Job::Screenshot { scale, reply } => {
                // A headless client draws into an image, not a window.
                let shot = match &headless {
                    Some(frame) => Screenshot::image(frame.0.clone()),
                    None => Screenshot::primary_window(),
                };
                commands
                    .spawn(shot)
                    .observe(move |captured: On<ScreenshotCaptured>| {
                        let image = captured.image.clone();
                        let reply = reply.clone();
                        std::thread::spawn(move || {
                            let _ = reply.send(encode(image, scale));
                        });
                    });
            }
        }
    }
}

/// A captured frame as PNG bytes, downscaled by `scale` (0 < scale <= 1).
fn encode(image: Image, scale: f32) -> Result<Vec<u8>, String> {
    let mut frame = image
        .try_into_dynamic()
        .map_err(|e| format!("frame conversion: {e}"))?;
    if scale < 1.0 {
        let width = ((frame.width() as f32) * scale).round().max(1.0) as u32;
        let height = ((frame.height() as f32) * scale).round().max(1.0) as u32;
        frame = frame.resize_exact(width, height, image::imageops::FilterType::Triangle);
    }
    let mut png = Vec::new();
    frame
        .to_rgb8()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("png: {e}"))?;
    Ok(png)
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
