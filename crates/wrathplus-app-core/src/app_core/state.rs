//! Game state that world, net and UI code all read: the spell catalogs and learned latches, the
//! client lifecycle and roster, the open NPC, loot gates and UI pointer flags, the quest giver,
//! the input map, the pet bar, world states, factions, emote sounds and collision heights. Each
//! owner keeps its systems and impls and re-exports the type from its old path.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use bevy::prelude::*;
use wrathplus_formats::{EmoteSoundCatalog, FactionCatalog, SkillLineCatalog, SpellCatalog};
use wrathplus_protocol::types::{
    ActionButton, PetSpells, QuestDetails, QuestGiverList, QuestOfferReward, QuestRequestItems,
};
use wrathplus_protocol::Character;

use super::cooldowns::Cooldowns;
use super::errors::UiError;

/// The spell DBC catalogs, absent without client data; the cast-visual router shares the catalog.
#[derive(Resource)]
pub struct Spells {
    pub catalog: SpellCatalog,
    /// Form id to its `SpellShapeshiftForm.dbc` row: `BonusActionBar` pages the bar
    /// (`GetBonusBarOffset 0x4e7620`), `flags1` holds the stance and cancel bits.
    pub forms: HashMap<u32, wrathplus_formats::ShapeshiftForm>,
    /// `SpellRange.dbc`, the `GetMinMaxRange 0x7ff480` inputs; empty on a failed load.
    pub ranges: wrathplus_formats::SpellRangeCatalog,
    /// `SpellCastTimes.dbc`, read through `CastingTimeIndex` (`GetCastTime 0x6e3340`).
    pub cast_times: wrathplus_formats::SpellCastTimeCatalog,
    /// `SpellDuration.dbc`: the `$d`/`$o` tokens' source (`GetDuration 0x6ea000`).
    pub durations: wrathplus_formats::SpellDurationCatalog,
    /// `SpellRadius.dbc`: the `$a` token's yards.
    pub radii: wrathplus_formats::SpellRadiusCatalog,
}

#[cfg(any(test, feature = "test-support"))]
impl Spells {
    pub fn empty_for_tests() -> Self {
        Spells {
            catalog: wrathplus_formats::SpellCatalog::from_displays(HashMap::new()),
            forms: HashMap::new(),
            ranges: wrathplus_formats::SpellRangeCatalog::default(),
            cast_times: Default::default(),
            durations: Default::default(),
            radii: Default::default(),
        }
    }
}

/// The reference's learned-ability latches: at learn time 12340's `0x542030` stores a spell's id
/// by its `Effect[0]`, and unlearning (`0x53fb37`) clears it. The gather cursor needs the corpse
/// trade's latch (`0x4f7e17`), so a player who never learned that trade gets no gather cursor.
#[derive(Resource, Default)]
pub struct LearnedAbilities {
    /// `[0xbe5d6c + 4×EffectMiscValue[0]]`: each [`super::gather::gather_kind`]'s skinning spell.
    pub gather: [Option<u32>; 4],
    /// `[0xbe5d7c]`: the known `SPELL_EFFECT_SKIN_PLAYER_CORPSE` spell; no cursor reads it yet.
    pub skin_player_corpse: Option<u32>,
    /// `[0xcecad8]`: the known `SPELL_EFFECT_FEED_PET` spell, latched by `0x6ea1d0` (from
    /// `0x5e9e49`); gate 3 of `ui_action::drop_item::drop_item_on_unit` and the spell it casts.
    pub feed_pet: Option<u32>,
}

/// The app's lifecycle: which screen owns the session.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClientState {
    /// Parked pre-logon at the login screen, the IO thread waiting for credentials. The realm list
    /// is no state: `GlueParent.lua`'s `GlueScreenInfo` omits it, a `DIALOG` frame over any screen
    /// ([`crate::realm_select::Realms::shown`]).
    #[default]
    Login,
    /// Parked at character select, the IO thread waiting for a pick.
    CharSelect,
    /// The character-creation screen, still parked at select (the IO thread serves create and
    /// delete in place).
    CharCreate,
    /// A character is in (or entering) the world.
    InWorld,
}

/// The account roster and the pick policy's memory.
#[derive(Resource, Default)]
pub struct Roster {
    pub chars: Vec<Character>,
    /// The selected row, `CharacterSelect.selectedIndex` 0-based. Written only through
    /// [`Roster::select`].
    pub selected: Option<usize>,
    /// Bumped by every [`Roster::select`], changed row or not: the reference's `SelectCharacter`
    /// zeroes the facing unconditionally (`0x472950` writes `0xb4217c` above its already-built
    /// check), so a roster refresh re-selecting the same index re-squares too. Only a row click is
    /// gated, in the stock Lua ([`Roster::click_row`]).
    pub select_seq: u64,
    /// The guid picked; kept in-world so a reconnect's roster is answered with it, cleared by a
    /// logout so the roster is shown.
    pub pending_pick: Option<u64>,
    /// `--character` (or `WOW_CHAR`): select this name on the first roster only
    /// ([`crate::launch_plan`]).
    pub env_char: Option<String>,
    /// `--enter-world` (or `WOW_CHAR`): press Enter World on the selected row, first roster only.
    pub env_enter: bool,
    /// A just-created character's name, whose row gets selected (the reference's
    /// `SELECT_LAST_CHARACTER`, keyed by name to survive the create/enum race).
    pub just_created: Option<String>,
    /// The realm-list entry this session connected to, refreshed with each roster.
    pub realm: Option<wrathplus_protocol::RealmInfo>,
    pub env_read: bool,
}

impl Roster {
    /// A roster with a pick in flight, for tests outside this module.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_pending_pick(chars: Vec<Character>, guid: u64) -> Self {
        Self {
            chars,
            pending_pick: Some(guid),
            ..Self::default()
        }
    }

    /// Note a character the create screen just made, and select its row now: `net::io` emits the
    /// fresh roster before the create result, so no later roster comes. If the row is not in hand
    /// yet, the arm stays for [`apply_roster_policy`].
    pub fn note_created(&mut self, name: String) {
        self.just_created = Some(name);
        self.select_created_by_name();
    }

    /// Select the armed just-created row by name, disarming only on a hit. Case-insensitive:
    /// vmangos's `normalizePlayerName` turns a typed `ZZBULL` into `Zzbull`.
    pub fn select_created_by_name(&mut self) {
        let Some(name) = self.just_created.as_deref() else {
            return;
        };
        if let Some(row) = self
            .chars
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
        {
            self.select(Some(row));
            self.just_created = None;
        }
    }

    /// Select a row, the reference's `SelectCharacter`; every selection goes through here so the
    /// facing reset is never skipped.
    pub fn select(&mut self, row: Option<usize>) {
        self.selected = row;
        self.select_seq = self.select_seq.wrapping_add(1);
    }

    /// A click on a row: `CharacterSelectButton_OnClick` selects only when `id` differs from
    /// `selectedIndex` (`CharacterSelect.lua:305-310`, and `OnDoubleClick` at 312-318), so clicking
    /// the current row keeps its dragged facing.
    pub fn click_row(&mut self, row: usize) {
        if self.selected != Some(row) {
            self.select(Some(row));
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_char(&self) -> Option<&Character> {
        self.selected.and_then(|i| self.chars.get(i))
    }

    /// The pending pick's map, for the loading screen's art before `SMSG_LOGIN_VERIFY_WORLD`.
    pub fn pending_map(&self) -> Option<u32> {
        self.pending_row().map(|c| c.map)
    }

    /// The pending pick's level: the reference suppresses the loading tip when it arrived as 0 in
    /// `SMSG_CHAR_ENUM` (`0x5b42a0` sets `[selChar+0x10a]`); vmangos always sends a real level.
    pub fn pending_level(&self) -> Option<u8> {
        self.pending_row().map(|c| c.level)
    }

    /// The picked character's `(map, wow xyz)`, a round trip before `SMSG_LOGIN_VERIFY_WORLD`; the
    /// streamers aim at it during world entry.
    pub fn pending_entry(&self) -> Option<(u32, [f32; 3])> {
        self.pending_row()
            .map(|c| (c.map, [c.position.x, c.position.y, c.position.z]))
    }

    /// The roster row for the pick in flight: the only description of the character until the
    /// self descriptor streams in, so the in-game UI seats `UnitName("player")` from it.
    pub fn pending_row(&self) -> Option<&Character> {
        self.pending_pick
            .and_then(|g| self.chars.iter().find(|c| c.guid == g))
    }

    /// The roster position of the pick in flight, by guid rather than the live selection: what
    /// the reference persists as `lastCharacterIndex` at Enter World.
    pub fn pending_index(&self) -> Option<usize> {
        let guid = self.pending_pick?;
        self.chars.iter().position(|c| c.guid == guid)
    }
}

/// The `"npc"` unit: the open `ui_session::NpcSession`'s world entity and guid, read by the portrait booth
/// as it reads [`crate::target::Selection`] for `"target"`. Only one session is open in play.
#[derive(Resource, Default, PartialEq, Eq)]
pub struct InteractNpc(pub Option<Entity>, pub Option<u64>);

/// The loot CVars. `autoLootDefault` is not a 1.12 CVar: 1.12 auto-loots only on a shift-click.
/// [`feed_loot`] sweeps the rows engine-side at the open, as the reference does, and a held Shift
/// inverts the setting. `showLootSpam` is 1.12's Detailed Loot Information checkbox (`0xb4e2bc`,
/// registered at `0x48fd1c`, default `"1"`), read only by the loot-roll line composers.
#[derive(Resource)]
pub struct LootConfig {
    pub auto_loot: bool,
    pub show_loot_spam: bool,
}

impl Default for LootConfig {
    fn default() -> Self {
        Self {
            // Off leaves 1.12's shift-click as the only auto-loot.
            auto_loot: false,
            // The reference's registered `"1"`.
            show_loot_spam: true,
        }
    }
}

/// A movement start that closes the loot: the reference's guard `0x60e990`, called by every
/// movement-start emitter, turning included, but not by mouse-look facing. No loot target reaches
/// the range gate `0x493230`, so there is no distance leash. vmangos also releases on movement
/// (`MovementHandler.cpp:1104`), but the close is the client's.
#[derive(Resource, Default)]
pub struct LootMoveStart(pub bool);

/// The loot-target latch, the reference's `[player+0x1d28]` guid: a loot session is open on this
/// object; kneeling is [`LootKneel`]'s question. Armed at the `CMSG_LOOT` send
/// (`0x5df253`/`0x5df40d`), at `SMSG_SPELL_GO` for an `OPEN_LOCK` on a chest (`0x6e831b`), by an
/// admitted response (`0x5eb900`), and at the `CMSG_OPEN_ITEM` send (`0x5edcc0`), which is what
/// admits vmangos' type-1 answer for the item.
///
/// Deviation: our close paths clear it guid-matched, where `CloseInteraction` clears it
/// unconditionally (`0x48f2c9`), so closing an old window keeps a newer `CMSG_LOOT`'s latch.
#[derive(Resource, Default)]
pub struct LootLatch(pub Option<u64>);

impl LootLatch {
    /// Drops the latch only if it still names `guid`.
    pub fn clear_for(&mut self, guid: u64) {
        if self.0 == Some(guid) {
            self.0 = None;
        }
    }
}

/// Whether the pointer is over any UI (the egui dev overlay or a player-UI frame), combined by
/// [`arbitrate_pointer_over_ui`]; gameplay reads it, so it is not the dev plugin's.
#[derive(Resource, Default)]
pub struct PointerOverUi(pub bool);

/// Whether the cursor carries a payload, a `Send` mirror. A click on sky with one held never
/// deselects: the reference gates its nothing-leg `SetSelection(0,0)` on no payload (`0x492d30`).
#[derive(Resource, Default)]
pub struct CursorPayloadHeld(pub bool);

/// The wire packet that opened the current panel.
pub enum QuestView {
    Greeting(QuestGiverList),
    Detail(QuestDetails),
    Progress(QuestRequestItems),
    Reward(QuestOfferReward),
}

/// The open questgiver window; the per-guid dialog statuses survive a close.
#[derive(Resource, Default)]
pub struct QuestGiver {
    pub completed_fanfare: bool,
    /// The panel's source: an NPC, a GameObject, a quest-starter item or a sharing player.
    pub npc: Option<u64>,
    pub view: Option<QuestView>,
    /// The reference's latch `0xbe0824`, written by DETAILS and OFFER_REWARD (`0x500ef0`) and by
    /// REQUEST_ITEMS' `closeOnCancel` (`0x501070`), and zeroed only on world enter (`0x500af0`).
    /// Non-zero turns a plain unit's decline into the silent teardown
    /// ([`decline_leg_native`]). vmangos sends 1 on every DETAILS and OFFER_REWARD; REQUEST_ITEMS
    /// carries 1 on a single-quest auto-open and 0 on `COMPLETE_QUEST`
    /// (`QuestHandler.cpp:390-395`).
    pub close_on_cancel: u32,
    /// The reference's already-acted latch `0xbe0844`, set by `AcceptQuest` and by a re-opening
    /// `DeclineQuest`, which keeps the panel up; both refuse while it is set (`0x5013a8`,
    /// `0x5013fe`) until a panel publish clears it ([`Self::open`]).
    pub acted: bool,
    /// The reference's `0xbe0828`: the chosen reward's item id, latched by `GetQuestReward`
    /// (`0x50161a`) for the turn-in's item line (`0x5dc676`-`0x5dc6c6`), and zeroed by a panel
    /// publish (`0x500d8c`), world enter (`0x500b15`) and the turn-in.
    pub chosen_reward: u32,
    /// Per-guid dialog status from `SMSG_QUESTGIVER_STATUS`, the `!`/`?` marker's value.
    pub statuses: HashMap<u64, u32>,
    pub wrath_statuses: HashMap<u64, u32>,
    /// Messages queued for [`feed_quest`], each a GlobalStrings key and its fills.
    pub messages: Vec<UiError>,
    pub reask: u32,
    /// The dialog statuses dropped at the reference's typemask-8 lookup (GameObject answers, which
    /// vmangos sends), per guid, so a live probe can tell a drop from a query never sent.
    pub refused: HashMap<u64, u32>,
    pub refused_count: u32,
}

impl QuestGiver {
    /// The quest the panel shows; a greeting lists several and names none.
    pub fn shown_quest(&self) -> Option<u32> {
        self.view.as_ref().and_then(|v| match v {
            QuestView::Detail(d) => Some(d.quest_id),
            QuestView::Progress(p) => Some(p.quest_id),
            QuestView::Reward(o) => Some(o.quest_id),
            QuestView::Greeting(_) => None,
        })
    }

    /// Open (or replace) the window with a fresh wire view for `npc`.
    pub fn open(&mut self, npc: u64, view: QuestView) {
        self.npc = Some(npc);
        self.view = Some(view);
        // `0x500bd0` clears the already-acted latch (`0x500d91`) and the chosen reward
        // (`0x500d8c`) on every panel publish.
        self.acted = false;
        self.chosen_reward = 0;
    }

    #[allow(dead_code)]
    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    /// The one-shot turn-in fanfare, which the reference plays from the packet, not from Lua.
    pub fn take_completed_fanfare(&mut self) -> bool {
        std::mem::take(&mut self.completed_fanfare)
    }

    pub fn clear(&mut self) {
        self.npc = None;
        self.view = None;
    }

    pub fn set_status(&mut self, npc: u64, status: u32) {
        self.statuses.insert(npc, status);
    }

    pub fn set_wrath_status(&mut self, npc: u64, status: u8) {
        self.wrath_statuses.insert(npc, u32::from(status));
    }

    pub fn wrath_statuses(&self) -> &HashMap<u64, u32> {
        &self.wrath_statuses
    }

    /// Every stored dialog status, which [`crate::quest_markers`] renders.
    pub fn statuses(&self) -> &HashMap<u64, u32> {
        &self.statuses
    }

    /// Drop the status of every guid `live` rejects: the reference keeps it on the unit
    /// (`unit+0xcb8`), so it dies with the object, and a map needs the prune.
    pub fn retain_statuses(&mut self, live: impl Fn(u64) -> bool) {
        self.statuses.retain(|&npc, _| live(npc));
        self.wrath_statuses.retain(|&npc, _| live(npc));
        // The refusals follow the same lifetime.
        self.refused.retain(|&npc, _| live(npc));
    }

    /// Drop one guid's status when it stops being a questgiver, as the reference's teardown does.
    pub fn clear_status(&mut self, npc: u64) {
        self.statuses.remove(&npc);
        self.wrath_statuses.remove(&npc);
    }

    /// Re-ask every visible questgiver's status, as the reference does after
    /// `SMSG_SET_FACTION_STANDING`, `SMSG_GROUP_LIST`, a turn-in and `SMSG_QUESTUPDATE_*`. A
    /// counter, folded into the marker query's generation, so no bump is lost to ordering.
    pub fn bump_reask(&mut self) {
        self.reask = self.reask.wrapping_add(1);
    }

    pub fn reask_epoch(&self) -> u32 {
        self.reask
    }

    pub fn refuse_status(&mut self, npc: u64, status: u32) {
        self.refused.insert(npc, status);
        self.refused_count = self.refused_count.saturating_add(1);
    }

    pub fn refused_for(&self, npc: u64) -> Option<u32> {
        self.refused.get(&npc).copied()
    }

    pub fn refused_count(&self) -> u32 {
        self.refused_count
    }

    /// One guid's stored dialog status; the markers read the whole map instead.
    #[allow(dead_code)]
    pub fn status(&self, npc: u64) -> Option<u32> {
        self.statuses.get(&npc).copied()
    }

    /// The open panel's title if it shows `quest_id`: the `%s` of a refusal naming the quest.
    pub fn view_title(&self, quest_id: u32) -> Option<String> {
        match self.view.as_ref()? {
            QuestView::Detail(d) if d.quest_id == quest_id => Some(d.title.clone()),
            QuestView::Progress(p) if p.quest_id == quest_id => Some(p.title.clone()),
            QuestView::Reward(o) if o.quest_id == quest_id => Some(o.title.clone()),
            _ => None,
        }
    }

    /// Queue a message for [`feed_quest`]; its key's catalog row picks the surface.
    pub fn push_message(&mut self, msg: UiError) {
        self.messages.push(msg);
    }

    pub fn take_messages(&mut self) -> Vec<UiError> {
        std::mem::take(&mut self.messages)
    }

    pub fn clear_session(&mut self) {
        self.clear();
        self.statuses.clear();
        self.wrath_statuses.clear();
        self.messages.clear();
        self.close_on_cancel = 0;
        self.acted = false;
        self.chosen_reward = 0;
    }
}

/// This frame's binding activity, which engine-side consumers read instead of raw keys: the
/// [`Input`]s the running bodies' Lua calls set.
#[derive(Resource, Default)]
pub struct BindingsState {
    /// Live latches, (base input, command): a press not yet released, whose release runs the
    /// command's `runOnUp` half. The reference replays the press-time chord at key-up (`0x483bd0`);
    /// latching the resolved command is equivalent.
    pub latched: Vec<(Held, String)>,
    /// The held bits a Start set and no Stop has cleared.
    pub held: Vec<Input>,
    /// Inputs whose Start, or whose one-shot, came this frame (the press edge).
    pub just: Vec<Input>,
    /// One-shots fired this frame.
    pub fired: Vec<Input>,
    /// Amount per one-shot this frame: a zoom's yards, else 1.0.
    pub amounts: Vec<(Input, f32)>,
    /// The keys we believe are down: a press of one is a repeat. The reference classifies
    /// auto-repeat off its own list (`0x4248b3`), never the Win32 `lParam` repeat bit.
    /// Normalized like [`latched`](Self::latched) and reconciled against `ButtonInput` every
    /// pass, so a window deactivate empties it and a key held across an alt-tab resumes, as in
    /// the reference. Keyboard only: mouse edges cannot repeat.
    pub down: Vec<KeyCode>,
}

impl BindingsState {
    /// Execute the movement commands supported by Wrath's native, VM-less binding route. Keeping
    /// this command-shaped preserves the same state edges as the Lua binding functions.
    pub fn apply_native_movement(&mut self, command: &str, pressed: bool) -> bool {
        let fired = match command {
            "JUMP" => Some(Input::Jump),
            "ATTACKTARGET" => Some(Input::AttackTarget),
            "TARGETNEARESTENEMY" => Some(Input::TargetNearestEnemy),
            "TARGETPREVIOUSENEMY" => Some(Input::TargetPreviousEnemy),
            _ => None,
        };
        if let Some(input) = fired {
            if pressed {
                self.fired.push(input);
                self.just.push(input);
            }
            // `JUMP` is `JumpOrAscendStart`/`AscendStop`: the key also holds the ascend.
            if input == Input::Jump {
                self.apply_held(Input::Ascend, pressed);
            }
            return true;
        }
        let input = match command {
            "MOVEFORWARD" => Input::MoveForward,
            "MOVEBACKWARD" => Input::MoveBackward,
            "TURNLEFT" => Input::TurnLeft,
            "TURNRIGHT" => Input::TurnRight,
            "STRAFELEFT" => Input::StrafeLeft,
            "STRAFERIGHT" => Input::StrafeRight,
            _ => return false,
        };
        self.apply_held(input, pressed);
        true
    }

    pub fn apply_held(&mut self, input: Input, pressed: bool) {
        if pressed {
            if !self.pressed(input) {
                self.just.push(input);
                self.held.push(input);
            }
        } else {
            self.held.retain(|&held| held != input);
        }
    }
}

impl BindingsState {
    /// Is a held bit set right now (the reference's `[InputControl+4]`)?
    pub fn pressed(&self, i: Input) -> bool {
        self.held.contains(&i)
    }

    /// Did this bit's Start, or this one-shot, come this frame (the key-down edge)?
    pub fn just_pressed(&self, i: Input) -> bool {
        self.just.contains(&i)
    }

    /// Did this one-shot fire this frame?
    pub fn fired(&self, i: Input) -> bool {
        self.fired.contains(&i)
    }

    /// Total amount for a one-shot this frame (0.0 when idle): the camera zoom's yards.
    pub fn amount(&self, i: Input) -> f32 {
        self.amounts
            .iter()
            .filter(|&&(a, _)| a == i)
            .map(|&(_, v)| v)
            .sum()
    }

    /// Both mouse bits held by their Lua Starts, the both-button run: MOVEANDSTEER's body
    /// (`Bindings.xml:3-11`) runs `CameraOrSelectOrMoveStart` and `TurnOrActionStart`.
    pub fn steering(&self) -> bool {
        self.pressed(Input::TurnOrAction) && self.pressed(Input::CameraOrSelectOrMove)
    }

    /// [`Self::steering`] began this frame.
    pub fn steering_began(&self) -> bool {
        self.steering()
            && (self.just_pressed(Input::TurnOrAction)
                || self.just_pressed(Input::CameraOrSelectOrMove))
    }
}

/// The physical input a latch waits on: a keyboard key by its code, not its name, so its release
/// ends the latch whatever the layout names it by then.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Held {
    Key(KeyCode),
    Mouse(MouseButton),
    Wheel,
}

/// One engine input a binding function drives: a held bit of `[InputControl+4]` or a one-shot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// `MoveForwardStart`/`Stop`, bit `0x10`.
    MoveForward,
    /// `MoveBackwardStart`/`Stop`, bit `0x20`.
    MoveBackward,
    /// `TurnLeftStart`/`Stop`, bit `0x100`.
    TurnLeft,
    /// `TurnRightStart`/`Stop`, bit `0x200`.
    TurnRight,
    /// `StrafeLeftStart`/`Stop`, bit `0x40`.
    StrafeLeft,
    /// `StrafeRightStart`/`Stop`, bit `0x80`.
    StrafeRight,
    /// `TurnOrActionStart`/`Stop`, bit `0x1`.
    TurnOrAction,
    /// `CameraOrSelectOrMoveStart`/`Stop`, bit `0x2`.
    CameraOrSelectOrMove,
    /// `PitchUpStart`/`Stop`, bit `0x400`: the swimmer's or flier's pitch keys.
    PitchUp,
    /// `PitchDownStart`/`Stop`, bit `0x800`.
    PitchDown,
    /// `JumpOrAscendStart`/`AscendStop`: the flier's ascend key.
    Ascend,
    /// `SitStandOrDescendStart`/`DescendStop`: the flier's descend key.
    Descend,
    /// `Jump()`.
    Jump,
    /// `ToggleRun()`, the walk toggle.
    ToggleRun,
    /// `ToggleAutoRun()`.
    ToggleAutoRun,
    /// `SitOrStand()`.
    SitOrStand,
    /// `ToggleSheath()`.
    ToggleSheath,
    /// Native ATTACKTARGET binding while the Wrath stock binding VM is not yet qualified.
    AttackTarget,
    /// Native TARGETNEARESTENEMY / reverse direction, using the ordinary target cycler.
    TargetNearestEnemy,
    TargetPreviousEnemy,
    /// `CameraZoomIn([yards])`, its yards in [`BindingsState::amount`].
    CameraZoomIn,
    /// `CameraZoomOut([yards])`, its yards in [`BindingsState::amount`].
    CameraZoomOut,
}

/// The pet action bar's state. A zero `spells.pet_guid` means no bar: the teardown packet carries
/// only that guid.
#[derive(Resource, Default)]
pub struct PetBar {
    /// Lossless native 12340 initialization. Capture only: the native visible feed is separate;
    /// `None` means unavailable, not a known empty spellbook. Never use Vanilla action helpers.
    pub native: Option<wrathplus_protocol::wrath_pet::PetSpells>,
    /// The last `SMSG_PET_SPELLS`, with `SMSG_PET_MODE` and local presses folded in.
    pub spells: PetSpells,
    /// The pet's own cooldowns, the list at `0xcecb04`. Seeded from `SMSG_PET_SPELLS`, topped up
    /// by `SMSG_SPELL_COOLDOWN` for the pet's guid, by the pet's own `SMSG_SPELL_GO` and by a
    /// press's GCD. Each mutation but the seed fires the flush, off [`Cooldowns::generation`]
    /// (`bar::fire_pet_cooldown_events`).
    pub cooldowns: Cooldowns,
    /// The client's `[0xb714b0]`, a local latch: all of `IsPetAttackActive`, and the only thing
    /// that lights Attack (`0x4bdf16`-`0x4bdf22`). Raised only for a possessed unit (`0x4bd420`),
    /// so never on a hunter's bar; lowered by `PetStopAttack` (`0x4bd650`), a new pet (`0x4bc8ce`)
    /// and the old-target clear (`0x493a18`). `PET_ATTACK_*` read the pet's flag `0x800` instead.
    pub attacking: bool,
    /// `PET_BAR_UPDATE` signals, counted. Both state writes signal unconditionally (`0x4bc940`,
    /// `0x4bc960`), and `OnClick`'s `SetChecked(0)` needs that repaint to relight a press on the
    /// current mode, so the count is in the feed's dedup key. Wrapping: only a change is read.
    pub bar_signals: u32,
    /// `[0xb714a8]`: the charm or possess expiry, the packet's duration past its arrival; `None`
    /// when the duration is 0, as for a hunter's or warlock's own pet.
    pub expires: Option<Instant>,
}

impl PetBar {
    /// `PetHasActionBar()`: a nonzero cached guid, with no alive or control check (`0x4bdc20`).
    pub fn has_bar(&self) -> bool {
        self.guid() != 0
    }

    /// The authoritative packet owner's GUID; native state is never copied into the legacy bar.
    pub fn guid(&self) -> u64 {
        self.native
            .as_ref()
            .map_or(self.spells.pet_guid, |pet| pet.guid)
    }

    pub fn bar_disabled(&self) -> bool {
        self.native
            .as_ref()
            .map_or_else(|| self.spells.bar_disabled(), |pet| pet.flags & 0x0800 != 0)
    }

    /// No pet: every field back to its default, the cooldown list emptied with its counters kept
    /// ([`crate::spell::Cooldowns::clear_silent`]), for the teardown packet and the session end.
    pub fn clear(&mut self) {
        let mut cooldowns = std::mem::take(&mut self.cooldowns);
        cooldowns.clear_silent();
        *self = Self {
            cooldowns,
            ..Default::default()
        };
    }
}

/// The live world-state table, keyed and valued by raw wire dwords.
#[derive(Resource, Default)]
pub struct WorldStates {
    pub(crate) values: HashMap<u32, u32>,
    pub(crate) generation: u64,
    pub(crate) scope: Option<(u32, u32)>,
}

impl WorldStates {
    /// Bumped per write and per init, not per pair, the edge readers rebuild on: the reference's
    /// handler re-runs the world-map landmark builder once per packet (`0x48fa0d` to `0x4a67a0`).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The reference's getter (`0x4c5810`): a missing key reads 0 (`0x4c5867`). Signed, as the
    /// reference prints it through `"%d"`.
    pub fn get(&self, key: u32) -> i32 {
        self.values.get(&key).copied().unwrap_or(0) as i32
    }

    /// Every pair, unordered, for instruments only (`capture::ProbeBgPlugin`); the reference
    /// never enumerates the table.
    pub fn pairs(&self) -> impl Iterator<Item = (u32, i32)> + '_ {
        self.values.iter().map(|(&k, &v)| (k, v as i32))
    }

    /// The `(map, area)` of the last init, `[0xb71e84]`/`[0xb71ea8]`; `None` before any init, the
    /// reference's `-1`.
    pub fn scope(&self) -> Option<(u32, u32)> {
        self.scope
    }

    /// An init's leading half (`0x4c5650`): drops every entry, then records the filter. The caller
    /// applies the pairs after.
    pub fn init_scope(&mut self, map: u32, area: u32) {
        self.values.clear();
        self.scope = Some((map, area));
        self.generation = self.generation.wrapping_add(1);
    }

    /// Writes `(id, value)` pairs, the reference's setter (`0x4c5870`) for both opcodes. Init's
    /// trailing `(0, 0)` is written as data; a zero value is stored, not deleted (`edx` is never
    /// tested in `[0x4c5870, 0x4c5a31)`); a repeated key overwrites in place (`0x4c58bb`).
    pub fn write(&mut self, states: &[(u32, u32)]) {
        self.values.extend(states.iter().copied());
        self.generation = self.generation.wrapping_add(1);
    }
}

/// The FactionTemplate.dbc catalog; absent if it failed to load, and the faction legs then read
/// neutral.
#[derive(Resource)]
pub struct Factions(pub FactionCatalog);

impl Factions {
    pub fn catalog(&self) -> &FactionCatalog {
        &self.0
    }

    /// Wrap a catalog, for tests; a running client builds the resource once, from the DBC.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_catalog(catalog: FactionCatalog) -> Self {
        Self(catalog)
    }
}

/// The emote-audio catalog (also the chat sender's `/name` → text-id resolver).
#[derive(Resource)]
pub struct EmoteSounds(pub EmoteSoundCatalog);

impl EmoteSounds {
    /// Resolve a `/command` name (case-insensitive) to its EmotesText id.
    pub fn text_id(&self, name: &str) -> Option<u32> {
        self.0.text_id(name)
    }

    /// An `Emotes.dbc` id's `AnimID`.
    pub fn anim(&self, emote_id: u32) -> Option<u32> {
        self.0.anim(emote_id)
    }

    /// An `Emotes.dbc` row's raw `AnimID` for the `UNIT_NPC_EMOTESTATE` resolver, `0` included.
    pub fn state_anim(&self, emote_id: u32) -> Option<u32> {
        self.0.state_anim(emote_id)
    }

    /// A text emote's `Emotes.dbc` id (`EmotesText.dbc` `EmoteID`); 0 is chat-only (`/thank`).
    pub fn text_emote(&self, text_id: u32) -> Option<u32> {
        self.0.text_emote(text_id)
    }

    /// The `Emotes.dbc` id in one of the five hard-coded gesture slots.
    pub fn gesture(&self, slot: usize) -> Option<u32> {
        self.0.gesture(slot)
    }

    /// An `Emotes.dbc` id's raw `EmoteFlags`, read by the posture-eligibility gate (`0x47db40`).
    pub fn emote_flags(&self, emote_id: u32) -> Option<u32> {
        self.0.emote_flags(emote_id)
    }

    /// The stand state a posture emote (`EmoteSpecProc == 1`) sets, as in `DoEmote`'s state
    /// branch (`0x5ef560` → `0x5ed430`).
    pub fn posture_state(&self, emote_id: u32) -> Option<u32> {
        self.0.posture_state(emote_id)
    }

    /// The `$ESD` anim event's kit for a looping state emote: `EventSoundID` only when
    /// `EmoteSpecProc == 2` (`row[+0x10] == 2` in the handler `0x6239f0`), so a one-shot emote id
    /// in the state field stays silent.
    pub fn state_event_sound(&self, emote_id: u32) -> Option<u32> {
        (self.0.spec_proc(emote_id) == Some(2))
            .then(|| self.0.event_sound(emote_id))
            .flatten()
    }
}

/// A unit's collision height in yards, 12340's `CMovement+0xcc` (`[unit+0x854]`): the `h` whose
/// fractions are the depth lines (swim `0.75·h`, `0x730d90`; splash `0.4·h`, `0x730ded`; foam gate
/// `max(2·h, 1)`, `0x71cc28`). `0x725f50` gives a player, or a creature whose record has
/// `type_flags & 0x20000`, `CreatureModelData.collisionHeight × k × max(SCALE_X, 1)`, `k` the unit
/// model scale of the native display, so a transform keeps the unit's own lines; any other
/// creature keeps the ctor default, unscaled. A player-driven body takes the server's
/// (`SMSG`/`MSG_MOVE_SET_COLLISION_HGT`) after create. Not the movement capsule,
/// [`crate::player::CAPSULE_HEIGHT`].
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct CollisionHeight(pub f32);

impl Default for CollisionHeight {
    /// The `CMovement` ctor's default, never `0.0`, at which every depth line collapses and the
    /// unit swims on dry land.
    fn default() -> Self {
        Self(crate::app_core::movement::DEFAULT_COLLISION_HEIGHT)
    }
}

/// Our target, set the instant we click, as the 1.12 client does without waiting for the server,
/// and cleared on deselect or when it streams out. `CMSG_SET_SELECTION` carries the guid, which the
/// server records in our `UNIT_FIELD_TARGET`.
#[derive(Resource, Default)]
pub struct Selection {
    pub target: Option<Entity>,
    pub guid: Option<u64>,
    /// The last-target pair `[0xb4e2e0]` that `TargetLastTarget` reads: `SetSelection` stamps the
    /// outgoing selection, none included, on every change it makes, a select or a deselect
    /// (`0x49361d`-`0x493628`); a teardown clear (`0x493910` alone) leaves it.
    pub last: Option<u64>,
}

/// Lines a CVar's change callback prints to the console (`0x63cb50`), such as `spellEffectLevel`'s
/// "Spell effect level set to %d.". A console command's own write collects them into its output;
/// any other write (a Lua `SetCVar`, the boot load) sends them to the log, as the reference's go
/// to a console screen nobody has open.
#[derive(Resource, Default)]
pub struct ConsoleEcho {
    pub capture: Option<Vec<String>>,
}

impl ConsoleEcho {
    pub fn print(&mut self, line: String) {
        match &mut self.capture {
            Some(lines) => lines.push(line),
            None => info!("console: {line}"),
        }
    }
}

/// `UNIT_FLAG_IN_COMBAT`, bit 19 of `UNIT_FIELD_FLAGS` (vmangos `UnitDefines.h:564`), read by the
/// client as `shr reg,0x13; test rl,1`. The local player has no separate combat latch: its two
/// hardcoded readers (`0x482f70`, `0x4d6038`) and `UnitAffectingCombat("player")` read this bit.
pub const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;

/// `UNIT_FIELD_FLAGS` bit 21 (vmangos `UnitDefines.h:566`), tested inside `GetWeapon` at
/// `0x5ec2b8` (CGPlayer) and `0x605e58`/`0x605e98` (CGUnit).
pub const UNIT_FLAG_DISARMED: u32 = 0x0020_0000;

/// The action slots (wire 0..119) and the known spells. The bar is client-authoritative:
/// [`drain::drain_action_sets`] writes it directly, and vmangos stores a slot without a reply
/// (`MasterPlayer::addActionButton`).
#[derive(Resource, Default)]
pub struct PlayerActions {
    /// Wire slot (0-based) to its packed action; the Lua action id is slot + 1.
    pub buttons: HashMap<u8, ActionButton>,
    /// The spell book (`SMSG_INITIAL_SPELLS`), ordered on purpose: the reference's first-hit
    /// scans (the lock resolver `0x5f83d0`) pick between equal spells, 6478 and 22810 "Opening",
    /// by array order, which is ascending id: the server sends the book from a `std::map`.
    pub spells: BTreeSet<u32>,
    /// Set by every book or bar change, cleared once the feed re-resolves slot identity, which a
    /// landed item template ([`crate::items::Items::template_epoch`]) also triggers. Item counts
    /// refresh every frame regardless.
    pub dirty: bool,
}

/// The world right-click's GameObject openers, queued one frame for the cast ladder: the click
/// system cannot also hold the ladder's resources, and only the ladder sends a cast, so a chest
/// right-clicked mid-cast takes the reference's silent same-spell bail (`0x6e4d43`).
#[derive(Resource, Default)]
pub struct GoOpenerCasts(pub Vec<GoOpener>);

/// One resolved item-use click, as `ui_items::send_item_use` needs it.
#[derive(Clone, Copy, Debug)]
pub struct ItemUse {
    /// The live instance's guid; `None` falls through to a plain use.
    pub guid: Option<u64>,
    /// Template `StartQuest`: non-zero diverts to the quest offer, ahead of the cast tail.
    pub start_quest: u32,
    /// The wire position (`255` = the player array).
    pub bag_index: u8,
    /// The wire slot, 0-based.
    pub slot: u8,
    /// The template entry: the cooldown store keys item records on `(use_spell, entry)` (the
    /// client's `[eax+8]==spellId && [eax+0xc]==itemID`).
    pub entry: u32,
    /// The template spell block ordinal the server should cast.
    pub spell_index: u8,
    /// The ON_USE spell, `0x5d8c80`'s answer: the first block with `SpellId != 0` and
    /// `SpellTrigger == 0`.
    pub use_spell: Option<u32>,
    /// The GameObject a key is used on, `CGItem::Use`'s target argument.
    pub on_object: Option<u64>,
    /// The template's `ITEM_FLAG_CHARTER` (`0x2000`), a signable petition.
    pub is_charter: bool,
}

/// A right-click's resolved opener ([`crate::target::click::resolve_go_action`]).
#[derive(Clone, Copy, Debug)]
pub enum GoOpener {
    /// A known `OPEN_LOCK` spell, cast with a GameObject target block.
    Spell { spell_id: u32, go_guid: u64 },
    /// A carried key: `CMSG_USE_ITEM` with `TARGET_FLAG_GAMEOBJECT`, the object's guid in
    /// [`ItemUse::on_object`].
    Key(ItemUse),
}

/// One `COMBAT_TEXT_UPDATE` (event `0x21E`, via `0x703f50`), fired at packet parse by every
/// producer, melee included (`0x6255b0` → `0x629d30`); `data`/`extra` are `arg2`/`arg3`. Fired only
/// when the unit the line is about is the player: each emitter tests that unit's class (`0x5efea0`)
/// for 0 before its helper (`0x629ef1`, `0x62d046`, `0x62834b`).
#[derive(Message, Clone)]
pub struct CombatTextEvent {
    pub message_type: &'static str,
    pub data: Option<String>,
    pub extra: Option<String>,
}

/// The player's aura durations by raw slot, each `SMSG_UPDATE_AURA_DURATION` stamped with its
/// arrival: the reference's expiry array `0xbc5f68`. Only the session's end clears it, never an
/// empty slot, whose aura may be a frame behind its packet, nor a worldport. At most 48 entries.
#[derive(Resource, Default)]
pub struct AuraDurations {
    pub by_slot: HashMap<u8, DurationStamp>,
}

impl AuraDurations {
    /// Records one `SMSG_UPDATE_AURA_DURATION`.
    pub fn set(&mut self, slot: u8, remaining_ms: u32, now: f64) {
        let total = f64::from(remaining_ms) / 1000.0;
        if aura_trace_period().is_some() {
            info!(
                "aura trace: SMSG_UPDATE_AURA_DURATION slot {slot} = {remaining_ms} ms @ {now:.2}"
            );
        }
        self.by_slot.insert(
            slot,
            DurationStamp {
                total,
                expires_at: now + total,
                received_at: now,
            },
        );
    }
}

/// The `WRATHPLUS_AURA_TRACE` period in seconds; a set value that is not a positive number means 1 s.
pub fn aura_trace_period() -> Option<f64> {
    static PERIOD: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *PERIOD.get_or_init(|| {
        let v = std::env::var("WRATHPLUS_AURA_TRACE").ok()?;
        Some(
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|p| *p > 0.0)
                .unwrap_or(1.0),
        )
    })
}

pub struct DurationStamp {
    /// Seconds: the full duration on an apply or refresh, only the remainder on a relog
    /// (`Map::ExistingPlayerLogin`) or a cast pushback (`Unit::DelaySpellAuraHolder`).
    pub total: f64,
    /// The real-clock instant it runs out: a server-sent span counts down in real seconds.
    pub expires_at: f64,
    /// The real-clock arrival, the freshness gate against a recycled slot.
    pub received_at: f64,
}

/// `SkillLine.dbc` × `SkillLineAbility.dbc`; absent when the data failed to load.
#[derive(Resource)]
pub struct SkillLines {
    pub catalog: SkillLineCatalog,
}

/// Spells learned mid-session, queued for `LEARNED_SPELL_IN_TAB` (event 510). Filled by the learn
/// and rank-up arms (`crate::spell::net`), never the `SMSG_INITIAL_SPELLS` load, which the
/// reference passes with `0x4b25b0`'s live-mutation flag clear. A queue because the event follows
/// the tab re-sort and `SPELLS_CHANGED` (`0x4b2b5a` before `0x4b2b92`).
#[derive(Resource, Default)]
pub struct LearnedInTab(pub Vec<u32>);

/// The reference's optimistic mirror of our `PLAYER_FLAGS` AFK bit (`[0xb6e5cc]`): written at
/// command time (`0x5eb7ae`, `0x5eb885`), reconciled from the descriptor (`0x5ee9f2`), reseeded
/// at world enter (`0x4989c0`). A `u32` tested for non-zero: the reconcile stores
/// `PLAYER_FLAGS & 2`, so a set mirror can read 2. DND has no mirror, so a second `/dnd` before
/// the server's update re-marks where a second `/afk` clears.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AfkMirror(pub u32);

impl AfkMirror {
    /// Non-zero, as all six of the reference's readers test it.
    pub fn is_afk(self) -> bool {
        self.0 != 0
    }
}

/// A site's ask of the tutorial system, written from wherever the reference's site lives.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TutorialEvent {
    /// `TriggerTutorial(id, delayMs)`: the fire-once leg.
    Trigger { id: u32, delay_ms: u32 },
    /// `SetTutorialFlag(id)` (`0x4b54c0`): the acknowledge-and-send leg.
    Acknowledge { id: u32 },
}

impl TutorialEvent {
    pub const fn trigger(id: u32) -> Self {
        Self::Trigger { id, delay_ms: 0 }
    }
}

/// `Attack`, the auto-attack pseudo-spell: it toggles melee rather than casting. The use path
/// keys on this id; the icon keys on `SPELL_EFFECT_ATTACK`, which only 6603 carries.
pub const SPELL_ATTACK: u32 = 6603;

/// `SMSG_GROUP_LIST`'s first byte for a raid; 0 is a party (vmangos `Group/Group.h:119`).
pub const GROUPTYPE_RAID: u8 = 1;

/// `partypet1..partypet4`, the pet of each [`PARTY_TOKENS`] slot (`0x4e81d0`).
pub const PARTY_PET_TOKENS: [&str; 4] = ["partypet1", "partypet2", "partypet3", "partypet4"];

/// `ItemClass` 2, weapon: the disarm ladder's whole test, so a shield is never the hand it hides.
pub(crate) const ITEM_CLASS_WEAPON: u8 = 2;

/// The one held slot the disarm hides: the main hand if it holds a weapon, else the off hand if it
/// does (`GetWeapon(slot, 0)`, `0x5ec2ae`/`0x5ec262`); the ranged slot is never gated
/// (`0x5ec25e`), as vmangos agrees (`Unit.h:991`). Over classes so the action bar's
/// equipped-item test (`0x5ea5d0`) can use it on raw inventory.
pub fn disarmed_hand(main_class: Option<u8>, off_class: Option<u8>) -> Option<usize> {
    if main_class == Some(ITEM_CLASS_WEAPON) {
        return Some(0);
    }
    if off_class == Some(ITEM_CLASS_WEAPON) {
        return Some(1);
    }
    None
}

/// `(realm, character)` off the roster's login pick, keying the per-character files.
pub fn identity(roster: &Roster) -> Option<(String, String)> {
    let guid = roster.pending_pick?;
    let name = roster.chars.iter().find(|c| c.guid == guid)?.name.clone();
    let realm = roster
        .realm
        .as_ref()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Realm".into());
    Some((realm, name))
}
