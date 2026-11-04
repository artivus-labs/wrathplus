//! The CVar registry, host side: the reference's engine-side table (`ConsoleVar.cpp`, 0xc4-byte
//! records of name, value, default, latch slot and the change callback handed to `CVar::Register`,
//! `0x63db90`). [`Cvars`] holds every registered row's live value, survives a VM replacement and is
//! what `config.toml` is composed from; the script VM keeps a mirror for Lua's synchronous
//! `GetCVar`/`SetCVar` ([`wrathplus_ui::script::UiScript::seed_cvars`]) whose writes queue back here.
//!
//! - [`REGISTERED`] holds only vars something reads, a host knob or a Lua consumer. A row's default
//!   is the reference's, and [`table::Registered::reference`] says where it stands against it.
//! - The change callback is a Bevy observer on [`CvarChanged`], beside the knob it writes; the
//!   registry applies nothing itself.
//! - A latched row's write is staged in [`Row::pending`] until [`Cvars::commit_latched`], the
//!   reference's `0x639ec0` inside `RestartGx`. A stage never committed is dropped at exit, as the
//!   reference's `SaveConfig` writes only the applied value (`rec+0x20`).
//! - Boot folds `wrathplus-config/config.toml` in and fires the observers before anything after
//!   [`CvarLoad`]; each frame drains the VM's writes in and pushes the registry's out; a dirty
//!   registry saves after one quiet second or at exit, writing only values off their default and
//!   keeping unknown keys verbatim.
//!
//! Env overrides (`WOW_UI_SCALE`, `WOW_FARCLIP`, …) win for the session and never touch the file
//! ([`Cvars::own_for_session`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use bevy::prelude::*;

use crate::app_core::vm_memo::VmMemo;
use wrathplus_ui::script::{SeededCvar, UiScript};

mod renamed;
mod table;
#[cfg(any(test, feature = "test-support"))]
pub use table::Reference;
pub use table::{deviates, ours, same, Registered, REGISTERED};

/// `config.toml`: a `[cvars]` table of `Name = "value"` strings, sorted so every save is stable.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct LocalConfig {
    #[serde(default)]
    pub cvars: BTreeMap<String, String>,
    // Preserve non-CVar tables (including experience) verbatim through CVar saves.
    #[serde(flatten)]
    other: BTreeMap<String, toml::Value>,
}

// ─── The registry ────────────────────────────────────────────────────────────────────────────

/// An accepted move of a CVar's applied value: the reference's change callback, as a Bevy event.
/// Not fired for a no-op write, a staged latched value, or a session override.
#[derive(Event, Clone, Debug, PartialEq, Eq)]
pub struct CvarChanged {
    /// The registered spelling (`MasterVolume`).
    pub name: String,
    pub old: String,
    pub new: String,
}

impl CvarChanged {
    /// Case-insensitive, like every lookup the client makes.
    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// The lowercased name, as observers' `match` arms spell it.
    pub fn key(&self) -> String {
        self.name.to_ascii_lowercase()
    }

    /// The new value as a number; `0` on a string row, whose observer reads [`Self::new`]. The
    /// registry refuses an unparseable write to a numeric row ([`Cvars::set`]).
    pub fn num(&self) -> f32 {
        self.new.trim().parse().unwrap_or(0.0)
    }

    /// The new value as the client's flag: int-parse, then `!= 0`.
    pub fn flag(&self) -> bool {
        self.num() != 0.0
    }
}

/// One row of the live registry: the parts of the reference's `CVar` record WrathPlus keeps.
#[derive(Clone, Debug)]
pub struct Row {
    /// The registered spelling.
    pub name: String,
    pub default: String,
    /// The applied value: what `GetCVar` answers and what the file is composed from.
    pub value: String,
    /// A latched row's staged value (`rec+0x38`), applied by [`Cvars::commit_latched`].
    pub pending: Option<String>,
    /// The reference's flag bit1.
    pub latched: bool,
    /// Declared by an addon's `RegisterCVar`: persisted like any row and re-seeded into every later
    /// VM, so the addon's re-declaration is a no-op.
    pub addon: bool,
}

impl Row {
    /// A row whose default parses as a number refuses a write that does not ([`Cvars::set`]).
    fn numeric(&self) -> bool {
        self.default.trim().parse::<f32>().is_ok()
    }
}

/// What a write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOutcome {
    /// No such row (warned).
    Unknown,
    /// A numeric row and a value that does not parse: refused, the applied value stands (warned).
    Refused,
    /// Already the applied value (or already the staged one).
    Unchanged,
    /// A latched row: staged, not applied; nothing fires until the commit.
    Staged,
    /// Applied: a [`CvarChanged`] is queued for the next flush and the config is dirty.
    Changed,
}

/// The engine-side CVar table.
#[derive(Resource)]
pub struct Cvars {
    rows: Vec<Row>,
    /// Lowercased name → row.
    index: HashMap<String, usize>,
    /// The file's `[cvars]` entries in their own spelling, the merge base of every save.
    file: BTreeMap<String, String>,
    /// Lowercased names the session owns rather than the player: the env levers, and `gxApi`, the
    /// render backend. Never saved; the file's entry is left as found.
    session_owned: HashSet<String>,
    /// Accepted moves not yet triggered, flushed by [`sync_cvars`], the boot load, the session-edge
    /// fold, or a caller of [`Cvars::take_events`].
    events: Vec<CvarChanged>,
    /// Host-side writes the VM's mirror has not seen; cleared by a seed.
    outbox: Vec<(String, String)>,
    /// A change since the last save; `last_change` drives the one-quiet-second debounce.
    pub dirty: bool,
    pub last_change: Option<Instant>,
}

impl Default for Cvars {
    /// The core table alone; a feature's rows join through [`RegisterCvars`].
    fn default() -> Self {
        let mut cvars = Self {
            rows: Vec::with_capacity(REGISTERED.len()),
            index: HashMap::with_capacity(REGISTERED.len()),
            file: BTreeMap::new(),
            session_owned: HashSet::new(),
            events: Vec::new(),
            outbox: Vec::new(),
            dirty: false,
            last_change: None,
        };
        cvars.add_rows(REGISTERED);
        cvars
    }
}

/// Rows registered before [`CvarPlugin`] built the registry, appended when it does.
#[derive(Resource, Default)]
struct PendingCvarRows(Vec<&'static [Registered]>);

/// A feature registers the CVar rows it owns in its own plugin, beside the observer on
/// [`CvarChanged`] that applies them: the registry names no consumer. Rows join in registration
/// order after the core table; only the latched rows' relative order is observable
/// ([`Cvars::commit_latched`] fires in row order), and each latched family registers whole.
pub trait RegisterCvars {
    fn register_cvars(&mut self, rows: &'static [Registered]) -> &mut Self;
    /// A [`SessionLever`] over this feature's knob, registered beside its rows.
    fn register_session_lever(&mut self, lever: SessionLever) -> &mut Self;
}

impl RegisterCvars for App {
    fn register_cvars(&mut self, rows: &'static [Registered]) -> &mut Self {
        let world = self.world_mut();
        match world.get_resource_mut::<Cvars>() {
            Some(mut cvars) => cvars.add_rows(rows),
            None => world.get_resource_or_init::<PendingCvarRows>().0.push(rows),
        }
        self
    }

    fn register_session_lever(&mut self, lever: SessionLever) -> &mut Self {
        self.world_mut()
            .get_resource_or_init::<SessionLevers>()
            .0
            .push(lever);
        self
    }
}

/// A lever's rows and their values, `None` where its knob is absent.
pub type LeverRows = Vec<(&'static str, Option<String>)>;

/// A `$WOW_*` lever a feature reads into its own knob, which then owns rows for the session: when
/// [`SessionLever::taken`] holds, each of [`SessionLever::read`]'s rows answers the env value and
/// never reaches the file ([`Cvars::own_for_session`]). A lever whose resource is absent still
/// marks its rows, so the file cannot apply over the env.
#[derive(Clone, Copy)]
pub struct SessionLever {
    /// What the lever is, for the log and the pin: the env var, or the condition.
    pub name: &'static str,
    /// Whether this session took the lever.
    pub taken: fn() -> bool,
    /// The rows it owns, and their values off the knob it seeded.
    pub read: fn(&World) -> LeverRows,
}

/// The levers features registered, in registration order; [`session_values`] reads the
/// registry's own ([`CORE_LEVERS`]) first.
#[derive(Resource, Default)]
pub struct SessionLevers(pub Vec<SessionLever>);

impl Cvars {
    /// Wrath build 12340: registration `0x51e04a -> 0x767fc0`, native flags `0x110`
    /// (not latched), default pointer `0x9e14ff` is the empty string. Source: installed
    /// client disassembly, `/tmp/tutorial-history-final-edges.log` and
    /// `/tmp/tutorial-history-default-event.log` (SHA-256 c6f92098…bc289c).
    /// This internal, non-addon row uses the existing file persistence path unchanged.
    pub fn register_wrath_tutorial_history(&mut self) {
        if self.slot("flaggedTutorials").is_none() {
            self.insert_row(Row {
                name: "flaggedTutorials".into(),
                default: String::new(),
                value: String::new(),
                pending: None,
                latched: false,
                addon: false,
            });
        }
    }

    /// Append `rows`, each at its default.
    pub fn add_rows(&mut self, rows: &[Registered]) {
        self.rows.reserve(rows.len());
        for r in rows {
            self.insert_row(Row {
                name: r.name.to_string(),
                default: r.default.to_string(),
                value: r.default.to_string(),
                pending: None,
                latched: r.latched,
                addon: false,
            });
        }
    }

    fn insert_row(&mut self, row: Row) {
        let key = row.name.to_ascii_lowercase();
        debug_assert!(
            !self.index.contains_key(&key),
            "{}: registered twice",
            row.name
        );
        self.index.insert(key, self.rows.len());
        self.rows.push(row);
    }

    fn slot(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_ascii_lowercase()).copied()
    }

    /// The row, matched case-insensitively.
    pub fn row(&self, name: &str) -> Option<&Row> {
        self.slot(name).map(|i| &self.rows[i])
    }

    /// Every row, in registration order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.row(name).map(|r| r.value.as_str())
    }

    pub fn num(&self, name: &str) -> Option<f32> {
        self.get(name).and_then(|v| v.trim().parse().ok())
    }

    /// The applied value as the client's flag (int-parse, `!= 0`).
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.num(name).map(|v| v != 0.0)
    }

    pub fn default_of(&self, name: &str) -> Option<&str> {
        self.row(name).map(|r| r.default.as_str())
    }

    pub fn is_session_owned(&self, name: &str) -> bool {
        self.session_owned.contains(&name.to_ascii_lowercase())
    }

    /// The persisted `checkAddonVersion` the addon load walk gates on; registered on.
    pub fn addon_version_check(&self) -> bool {
        self.flag("checkAddonVersion").unwrap_or(true)
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.last_change = Some(Instant::now());
    }

    /// A write from either side of the VM boundary. A `from_vm` write is not echoed back, but a
    /// refusal is, because the mirror already stored the refused value.
    fn write(&mut self, name: &str, value: &str, from_vm: bool) -> SetOutcome {
        let Some(i) = self.slot(name) else {
            warn!("cvar {name}: not registered — write ignored");
            return SetOutcome::Unknown;
        };
        let row = &mut self.rows[i];
        if row.numeric() && value.trim().parse::<f32>().is_err() {
            warn!(
                "cvar {}: unparseable value '{value}' refused (still {:?})",
                row.name, row.value
            );
            if from_vm {
                self.outbox.push((row.name.clone(), row.value.clone()));
            }
            return SetOutcome::Refused;
        }
        if row.latched {
            // The reference's `Set 0x63df50` on flag bit1 stores `latchedValue` and skips
            // `InternalSet`: no dirty, no callback. The stage lives here, not in the mirror, which
            // only ever learns applied values.
            let staged = (value != row.value).then(|| value.to_string());
            if row.pending == staged {
                return SetOutcome::Unchanged;
            }
            row.pending = staged;
            return if row.pending.is_some() {
                SetOutcome::Staged
            } else {
                SetOutcome::Unchanged // the stage cleared: the boundary has nothing to do
            };
        }
        if row.value == value {
            return SetOutcome::Unchanged;
        }
        let old = std::mem::replace(&mut row.value, value.to_string());
        let name = row.name.clone();
        self.events.push(CvarChanged {
            name: name.clone(),
            old,
            new: value.to_string(),
        });
        if !from_vm {
            self.outbox.push((name, value.to_string()));
        }
        self.touch();
        SetOutcome::Changed
    }

    /// A host-side write (minimap zoom, camera views, the remembered character, the tip cursor):
    /// mirrored into the VM, persisted and observed.
    pub fn set(&mut self, name: &str, value: &str) -> SetOutcome {
        self.write(name, value, false)
    }

    /// A write the VM's mirror already made, drained from its change queue.
    pub fn set_from_vm(&mut self, name: &str, value: &str) -> SetOutcome {
        self.write(name, value, true)
    }

    /// Follow a value the engine already applied, for a second spelling of one knob
    /// (`WorldDetail`/`frillDensity`): the row moves, persists and reaches the mirror, but no
    /// observer fires, since a full write would queue an event that lands a flush late and wins
    /// stale. Returns whether the row moved.
    pub fn mirror(&mut self, name: &str, value: &str) -> bool {
        let Some(i) = self.slot(name) else {
            warn!("cvar {name}: not registered — mirror ignored");
            return false;
        };
        let row = &mut self.rows[i];
        if row.value == value {
            return false;
        }
        row.value = value.to_string();
        row.pending = None;
        self.outbox.push((row.name.clone(), value.to_string()));
        self.touch();
        true
    }

    /// The latch boundary `RestartGx` crosses: the reference's `0x639ec0..0x639f5a`, which calls
    /// `CVar::Commit` (`0x63e060`) on the fourteen gx records `[0xc4ea90]` … `[0xc4eab4]`. Each
    /// staged `gx*` value is applied, fires, persists and reaches the mirror; returns how many
    /// moved.
    ///
    /// Only `gx*` rows: `SoundBufferSize`'s register site (`0x4571ca`) discards its record, so
    /// nothing commits it and its stage is lost at exit, as in the reference. The reference runs a
    /// gx callback at `SetCVar` time (`0x63df50`); ours fire here, before the device rebuild reads
    /// them.
    pub fn commit_latched(&mut self) -> usize {
        let mut moved = 0;
        for row in &mut self.rows {
            if !row.name.starts_with("gx") {
                continue;
            }
            let Some(staged) = row.pending.take() else {
                continue;
            };
            if staged == row.value {
                continue;
            }
            let old = std::mem::replace(&mut row.value, staged.clone());
            self.events.push(CvarChanged {
                name: row.name.clone(),
                old,
                new: staged.clone(),
            });
            self.outbox.push((row.name.clone(), staged));
            moved += 1;
        }
        if moved > 0 {
            self.touch();
        }
        moved
    }

    /// The session owns this row: it is never saved and the file's entry is left alone. `value` is
    /// what it answers this session; `None` marks it without moving it. No observer fires: the knob
    /// already read the env.
    pub fn own_for_session(&mut self, name: &str, value: Option<&str>) {
        let key = name.to_ascii_lowercase();
        self.session_owned.insert(key.clone());
        let Some(value) = value else {
            return;
        };
        let Some(&i) = self.index.get(&key) else {
            warn!("cvar {name}: not registered — session value ignored");
            return;
        };
        let row = &mut self.rows[i];
        if row.value != value {
            row.value = value.to_string();
            row.pending = None;
            self.outbox.push((row.name.clone(), value.to_string()));
        }
    }

    /// An addon's `RegisterCVar`: a row of its own at the file's value for the name, else at the
    /// declared default. A name already registered is a no-op.
    pub fn learn_addon_row(&mut self, name: &str, default: &str) {
        if self.slot(name).is_some() {
            return;
        }
        let saved = self
            .file
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone());
        self.insert_row(Row {
            name: name.to_string(),
            default: default.to_string(),
            value: saved.unwrap_or_else(|| default.to_string()),
            pending: None,
            latched: false,
            addon: true,
        });
    }

    /// Fold the file in: a known, player-owned key becomes its row's applied value and fires (the
    /// reference's `Register` on a record `Config.wtf` created calls the callback); an unknown key
    /// is kept for the save and warned; a session-owned one is skipped. Nothing dirties.
    pub fn load_file(&mut self, file: BTreeMap<String, String>) {
        for (name, value) in &file {
            let key = name.to_ascii_lowercase();
            let Some(&i) = self.index.get(&key) else {
                warn!("config: unknown cvar '{name}' — preserved, not applied");
                continue;
            };
            if self.session_owned.contains(&key) {
                info!("config: {name} is owned by this session, not the file (file value kept)");
                continue;
            }
            let row = &mut self.rows[i];
            if row.numeric() && value.trim().parse::<f32>().is_err() {
                warn!("config: {name}: unparseable value '{value}' ignored");
                continue;
            }
            if row.value == *value {
                continue;
            }
            let old = std::mem::replace(&mut row.value, value.clone());
            self.events.push(CvarChanged {
                name: row.name.clone(),
                old,
                new: value.clone(),
            });
        }
        self.file = file;
    }

    /// The file's entries no row claims: a newer build's keys, or an addon's not yet registered.
    /// Handed to the VM as its saved base, so an addon's `RegisterCVar` starts at the saved value.
    pub fn orphans(&self) -> Vec<(String, String)> {
        self.file
            .iter()
            .filter(|(k, _)| !self.index.contains_key(&k.to_ascii_lowercase()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// The whole table as a VM's mirror is seeded from it.
    pub fn vm_seed(&self) -> Vec<SeededCvar> {
        self.rows
            .iter()
            .map(|r| SeededCvar {
                name: r.name.clone(),
                value: r.value.clone(),
                default: r.default.clone(),
                latched: r.latched,
            })
            .collect()
    }

    /// Read before taking, so a quiet frame never deref-muts the registry.
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// The accepted moves since the last flush, for the caller to trigger.
    pub fn take_events(&mut self) -> Vec<CvarChanged> {
        std::mem::take(&mut self.events)
    }

    fn has_outbox(&self) -> bool {
        !self.outbox.is_empty()
    }

    pub fn take_outbox(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.outbox)
    }

    /// The file to save: the previous file as the merge base, each row off its default at its
    /// applied value (a staged one is not the player's yet), each at its default removed;
    /// session-owned and unknown keys untouched.
    pub fn compose(&self) -> BTreeMap<String, String> {
        let mut out = self.file.clone();
        for row in &self.rows {
            let key = row.name.to_ascii_lowercase();
            if self.session_owned.contains(&key) {
                continue;
            }
            // Match any existing entry case-insensitively so a hand-edited spelling doesn't fork.
            let existing = out
                .keys()
                .find(|k| k.eq_ignore_ascii_case(&row.name))
                .cloned();
            if row.value == row.default {
                if let Some(k) = existing {
                    out.remove(&k);
                }
            } else {
                out.insert(
                    existing.unwrap_or_else(|| row.name.clone()),
                    row.value.clone(),
                );
            }
        }
        out
    }

    /// A registry already holding one stored value, as if `config.toml` said so.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_value(name: &str, value: &str) -> Self {
        let mut cvars = Self::default();
        cvars.load_file(BTreeMap::from([(name.to_string(), value.to_string())]));
        cvars.events.clear();
        cvars
    }

    /// The core table and `rows`, as the features owning them would register them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rows(rows: &[&'static [Registered]]) -> Self {
        let mut cvars = Self::default();
        for rows in rows {
            cvars.add_rows(rows);
        }
        cvars
    }

    /// [`Self::with_rows`] already holding one stored value.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rows_and_value(rows: &[&'static [Registered]], name: &str, value: &str) -> Self {
        let mut cvars = Self::with_rows(rows);
        cvars.load_file(BTreeMap::from([(name.to_string(), value.to_string())]));
        cvars.events.clear();
        cvars
    }
}

/// How long a dirty config waits before saving: long enough to coalesce a slider drag.
pub const SAVE_QUIET: std::time::Duration = std::time::Duration::from_secs(1);

/// The startup fold of `config.toml` into the registry and the knobs ([`load_config`]). A set
/// because the world camera reads `gxMultisample` once at spawn, so `setup_player` must order after
/// it; without the constraint that order is the executor's choice.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CvarLoad;

pub struct CvarPlugin;

impl Plugin for CvarPlugin {
    fn build(&self, app: &mut App) {
        let mut cvars = Cvars::default();
        if let Some(pending) = app.world_mut().remove_resource::<PendingCvarRows>() {
            for rows in pending.0 {
                cvars.add_rows(rows);
            }
        }
        app.insert_resource(cvars)
            .add_systems(
                Startup,
                (load_config, publish_filter_policy)
                    .chain()
                    .in_set(CvarLoad),
            )
            // After the tick, so a `SetCVar` from this frame reaches the registry and its observers
            // before the frame's drains; `video::drain_restart_gx` orders after this so the commit
            // finds the stage.
            .add_systems(Update, sync_cvars.after(crate::app_core::schedule::UiInput))
            // In `Last`, after every `Update` writer of the registry, so the save reads the frame's
            // settled state; before the exit flush, which finds it clean on a quiet exit frame.
            .add_systems(
                Last,
                save_config_when_quiet.before(crate::shutdown::OnAppExit),
            );
        // The exit flush runs on the exit edge: the close button's `AppExit` is written in
        // `PostUpdate`, so an `Update` save never sees it.
        crate::shutdown::on_app_exit(app, save_config_on_exit.into_configs());
    }
}

/// What the environment took for this session and its value, read off the knobs the env levers
/// seeded (`RenderScale::default()` reads `$WOW_RENDER_SCALE`, …): the registry's own levers, over
/// the crates below it, then every feature's in registration order. Each row is owned on its own,
/// so the order only fixes the log's.
fn session_values(world: &World) -> Vec<(&'static str, Option<String>)> {
    let registered = world
        .get_resource::<SessionLevers>()
        .map(|l| l.0.as_slice())
        .unwrap_or_default();
    CORE_LEVERS
        .iter()
        .chain(registered)
        .filter(|lever| (lever.taken)())
        .flat_map(|lever| {
            let rows = (lever.read)(world);
            debug!(
                "cvars: session lever {} owns {:?}",
                lever.name,
                rows.iter().map(|(n, _)| *n).collect::<Vec<_>>()
            );
            rows
        })
        .collect()
}

/// Whether `$var` is set at all, which is how most levers are taken.
pub fn env_set(var: &str) -> bool {
    std::env::var_os(var).is_some()
}

/// A boolean knob's CVar spelling.
pub fn lever_flag(b: bool) -> String {
    if b { "1" } else { "0" }.to_string()
}

/// The levers over `wrathplus-world` and `wrathplus-assets` knobs, which no feature plugin owns.
pub const CORE_LEVERS: &[SessionLever] = &[
    SessionLever {
        name: "WOW_FARCLIP",
        taken: || env_set("WOW_FARCLIP"),
        read: |world| {
            let v = world.get_resource::<wrathplus_world::view::ViewDistance>();
            vec![("farclip", v.map(|s| s.farclip.to_string()))]
        },
    },
    // The clutter lever takes both spellings of its knob; an off-grid multiplier seeds off-grid.
    SessionLever {
        name: "WOW_CLUTTER_DENSITY",
        taken: || env_set("WOW_CLUTTER_DENSITY"),
        read: |world| {
            let v = world.get_resource::<wrathplus_world::clutter::ClutterConfig>();
            vec![
                ("WorldDetail", v.map(|c| (c.density - 1.0).to_string())),
                ("frillDensity", v.map(|c| c.frill_density().to_string())),
            ]
        },
    },
    SessionLever {
        name: "WOW_TRILINEAR",
        taken: || env_set("WOW_TRILINEAR"),
        read: |world| {
            let tex = world.get_resource::<wrathplus_assets::TexFilterSetting>();
            vec![("trilinear", tex.map(|t| lever_flag(t.trilinear)))]
        },
    },
    SessionLever {
        name: "WOW_ANISO",
        taken: || env_set("WOW_ANISO"),
        read: |world| {
            let tex = world.get_resource::<wrathplus_assets::TexFilterSetting>();
            vec![("anisotropic", tex.map(|t| t.aniso.to_string()))]
        },
    },
    // An instrument run must not pin 4× multisampling into the file.
    SessionLever {
        name: "WOW_MSAA",
        taken: || env_set("WOW_MSAA"),
        read: |world| {
            let v = world.get_resource::<wrathplus_world::view::MsaaSetting>();
            vec![("gxMultisample", v.map(|m| m.samples.to_string()))]
        },
    },
];

/// Startup: mark what the session owns, fold `config.toml` in (no file means all defaults) and fire
/// the observers now, so everything after [`CvarLoad`] finds its knob written. The VM does not
/// exist yet; [`sync_cvars`] seeds its mirror.
fn load_config(world: &mut World) {
    let session = session_values(world);
    let stored = stored_config();
    let wrath = world
        .get_resource::<wrathplus_assets::ClientDataProfile>()
        .is_some_and(|p| matches!(p.0, wrathplus_formats::DataProfile::Wrath { .. }));
    let events = {
        let mut cvars = world.resource_mut::<Cvars>();
        if wrath {
            cvars.register_wrath_tutorial_history();
        }
        for (name, value) in session {
            cvars.own_for_session(name, value.as_deref());
        }
        // `gxApi` is the render backend, a fact about the machine that `sync_cvars` pushes live, so
        // it is never persisted.
        cvars.own_for_session("gxApi", None);
        match stored {
            StoredConfig::Absent => {} // no file, hermetic capture, or no install
            StoredConfig::Bad(msg) => {
                // A malformed file is kept: nothing loads, and nothing saves over it until a
                // change.
                warn!("{msg}");
            }
            StoredConfig::Table(table) => cvars.load_file(table),
        }
        cvars.take_events()
    };
    for event in events {
        world.trigger(event);
    }
}

/// What the one read of `config.toml` found.
enum StoredConfig {
    /// No file, no install, or a hermetic capture: every value is its registered default.
    Absent,
    Table(BTreeMap<String, String>),
    /// Unreadable or malformed; the message is carried because this read runs before `LogPlugin`
    /// exists.
    Bad(String),
}

/// Read `config.toml`, for [`load_config`] and for the primary window in [`crate::run`], which
/// needs `gxWindow`/`gxResolution` before it exists. Not cached: a process-wide cache would answer
/// every test from the first one's file.
fn stored_config() -> StoredConfig {
    let Some(path) = crate::local_state::config_path() else {
        return StoredConfig::Absent; // hermetic capture or no install: session-only state
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StoredConfig::Absent,
        Err(e) => return StoredConfig::Bad(format!("config: cannot read {}: {e}", path.display())),
    };
    match toml::from_str::<LocalConfig>(&text) {
        Ok(cfg) => StoredConfig::Table(renamed::migrate(cfg.cvars)),
        Err(e) => StoredConfig::Bad(format!(
            "config: {} is malformed ({e}) — running on defaults",
            path.display()
        )),
    }
}

/// One CVar as `config.toml` holds it, before the `App` exists: for the primary window, which is
/// built with its display mode resolved. Everything else reads [`Cvars::get`].
pub fn boot_cvar(name: &str) -> Option<String> {
    match stored_config() {
        StoredConfig::Table(t) => t
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v),
        StoredConfig::Absent | StoredConfig::Bad(_) => None,
    }
}

/// Per frame: seed a new VM's mirror, drain its registrations and writes into the registry, push
/// the registry's writes into the mirror, and trigger every accepted move.
pub fn sync_cvars(
    script: Option<NonSendMut<UiScript>>,
    mut cvars: ResMut<Cvars>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    msaa_formats: Option<Res<wrathplus_world::view::MsaaFormats>>,
    mut seeded: Local<VmMemo<bool>>,
    mut commands: Commands,
) {
    // The live render backend; absent headless, where the registered `""` stands.
    if let Some(adapter) = adapter.as_deref() {
        let backend = adapter.backend.to_str();
        if cvars.get("gxApi") != Some(backend) {
            cvars.own_for_session("gxApi", Some(backend));
        }
    }
    let Some(mut script) = script else {
        // Nothing to mirror into; a later VM is seeded from the table, which carries it all.
        if cvars.has_outbox() {
            cvars.take_outbox();
        }
        if cvars.has_events() {
            for event in cvars.take_events() {
                commands.trigger(event);
            }
        }
        return;
    };
    // The VM's writes before the seed: an addon's `SetCVar` at load is already queued, and seeding
    // first would overwrite the mirror with the older value. Registrations before writes, since an
    // addon declares a row and sets it together.
    let registrations = script.take_cvar_registrations();
    let changes = script.take_cvar_changes();
    if !registrations.is_empty() || !changes.is_empty() {
        for (name, default) in registrations {
            cvars.learn_addon_row(&name, &default);
        }
        for (name, value) in changes {
            cvars.set_from_vm(&name, &value);
        }
    }
    if seeded.claim(&script) {
        // The file's unclaimed entries first, so an addon's `RegisterCVar` starts at the saved
        // value; then the table.
        script.set_cvar_saved_base(cvars.orphans());
        script.seed_cvars(cvars.vm_seed());
        if cvars.has_outbox() {
            cvars.take_outbox(); // the seed just carried everything
        }
        // The Video dropdown's menu: what this device accepts, enumerated by
        // `view::MsaaSupportPlugin`.
        script.set_multisample_formats(
            msaa_formats
                .as_deref()
                .map(|f| {
                    f.formats
                        .iter()
                        .map(|&(color_bits, depth_bits, samples)| {
                            wrathplus_ui::script::MultisampleFormat {
                                color_bits,
                                depth_bits,
                                samples,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
        // `GetVideoCaps`, the seven values `OptionsFrame_Load` destructures. Shaders, trilinear,
        // anisotropy and the hardware cursor are always there (`crate::cursor` composites the
        // reference's cursors into an OS cursor); `max_anisotropy` is reported raw because
        // `OptionsFrame.lua:122` matches it against `ANISOTROPIC_VALUES`. Triple buffering is
        // false: wgpu owns the surface's buffering, so the stock frame hides check button 13
        // (`OptionsFrame.lua:163-165`).
        script.set_video_caps(wrathplus_ui::script::VideoCaps {
            anisotropic: true,
            pixel_shaders: true,
            vertex_shaders: true,
            trilinear: true,
            triple_buffering: false,
            max_anisotropy: *wrathplus_assets::ANISO_RANGE.end(),
            hardware_cursor: true,
        });
    }
    if cvars.has_outbox() {
        for (name, value) in cvars.take_outbox() {
            script.set_cvar_host(&name, &value);
        }
    }
    if cvars.has_events() {
        for event in cvars.take_events() {
            commands.trigger(event);
        }
    }
}

/// The file's header comment.
const HEADER: &str = "\
# WrathPlus local config — CVar values that moved off their defaults.
# Managed by the client; hand edits are read on next launch and preserved on save.
";

/// Dirty and one quiet second since the last change: save, so a crash loses at most that second.
fn save_config_when_quiet(mut cvars: ResMut<Cvars>) {
    if cvars.dirty && cvars.last_change.is_none_or(|t| t.elapsed() >= SAVE_QUIET) {
        write_config(&mut cvars);
    }
}

/// The exit frame: save whatever is still dirty, quiet second or not.
pub fn save_config_on_exit(mut cvars: ResMut<Cvars>) {
    if cvars.dirty {
        write_config(&mut cvars);
    }
}

fn preserved_config_tables(path: &std::path::Path) -> BTreeMap<String, toml::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<LocalConfig>(&text).ok())
        .map(|config| config.other)
        .unwrap_or_default()
}

/// Rewrite `config.toml` atomically from the registry, so a session with no VM saves what it
/// changed.
fn write_config(cvars: &mut Cvars) {
    let Some(path) = crate::local_state::config_path() else {
        cvars.dirty = false; // hermetic/session-only: nothing to write, stop retrying
        return;
    };
    let file = cvars.compose();
    let body = toml::to_string(&LocalConfig {
        cvars: file.clone(),
        other: preserved_config_tables(&path),
    })
    .expect("string map serializes");
    match crate::local_state::write_atomic(&path, &format!("{HEADER}{body}")) {
        Ok(()) => {
            cvars.file = file;
            cvars.dirty = false;
        }
        Err(e) => {
            warn!("config: cannot write {}: {e}", path.display());
            cvars.dirty = false; // don't retry every frame into the same error
        }
    }
}

/// Freeze the texture filter policy for the process and log the mode, so a player's log says which
/// mode the run was in. Its own system after [`load_config`], so it publishes whatever the file
/// held.
fn publish_filter_policy(filter: Res<wrathplus_assets::TexFilterSetting>) {
    wrathplus_assets::publish_tex_filter(*filter);
    let mode = filter.mode();
    let name = match mode {
        3 => "bilinear + nearest-mip select, aniso off",
        4 => "trilinear, aniso off",
        _ => "trilinear + aniso",
    };
    info!(
        "texture filter: mode {mode} ({name}) — trilinear={} anisotropic={}",
        u8::from(filter.trilinear),
        filter.aniso
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_table_survives_cvar_serialization() {
        let text = "[cvars]\nMusicVolume = '0.7'\n[experience]\nprofile = 'plus'\npacks = ['art', 'models']\n[experience.features]\nbloom = false\n";
        let path =
            std::env::temp_dir().join(format!("azb-experience-save-{}.toml", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let preserved = preserved_config_tables(&path);
        std::fs::remove_file(&path).unwrap();
        let mut cfg: LocalConfig = toml::from_str(text).unwrap();
        assert_eq!(preserved, cfg.other);
        cfg.cvars.insert("MusicVolume".into(), "0.5".into());
        let saved = toml::to_string(&cfg).unwrap();
        let back: LocalConfig = toml::from_str(&saved).unwrap();
        assert_eq!(back.other, cfg.other);
        assert_eq!(back.cvars["MusicVolume"], "0.5");
    }

    #[test]
    fn the_toml_round_trips() {
        let cfg = LocalConfig {
            cvars: [("MusicVolume".to_string(), "0.7".to_string())].into(),
            other: BTreeMap::new(),
        };
        let text = format!("{HEADER}{}", toml::to_string(&cfg).unwrap());
        let back: LocalConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.cvars, cfg.cvars);
        let hand = "# my note\n[cvars]\nFarclip = \"500\"\n";
        let parsed: LocalConfig = toml::from_str(hand).unwrap();
        assert_eq!(parsed.cvars.get("Farclip").map(String::as_str), Some("500"));
    }
}
