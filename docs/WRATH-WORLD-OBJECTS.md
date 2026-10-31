# Game objects, fishing and world presentation

Wire reference: AzerothCore `7f12e89ee5f467a50e62eba1d525eac7dc953d03`: `GameObject.cpp`
(`SendCustomAnim`, fishing in `Use`/`Update`, goober page text, `ModifyHealth`), `Object.cpp`
(`SendObjectDeSpawnAnim`, `SendPlayMusic`, object sounds), `MiscPackets.cpp` (`PlayObjectSound`),
`spell_generic.cpp` (spirit healer confirm), `Map.cpp` (zone light override), `CharacterHandler.cpp`
(dance moves at login), `Player.cpp` (`SendMovieStart`), `MiscHandler.cpp` (`SendSetPhaseShift`),
`SpellHandler.cpp` (`HandleGameObjectUseOpcode`, `HandleGameobjectReportUse`). Codec:
`crates/wrathplus-protocol/src/wrath_world_objects.rs`.

**Use (00B1)**: the shared `GameObjUse` command (right-click on a chest, door, mailbox, herb, ore
vein or fishing bobber, from `target/click`) had no Wrath route, so nothing was sent on 3.3.5. It
now rides the `NativeWorldObjects` route with the full 8-byte guid, as AC reads it. AC answers by
object type (loot, door state, gossip); those replies are their own families.

**Shared (handled)**: 00B3 custom animation (guid, anim) and 0215 despawn animation (guid) to
`net/objects`; 01C8 not hooked and 01C9 escaped (empty) to `ui_loot`; 0222 spirit healer confirm
(the healer's guid; sent by AC's spirit healer spell script) to `death`; 0277 music (kit id) and
0278 object sound (kit id, then the source guid, `PlayObjectSound::Write` order) to `sound`.

**Capture-only** (`NativeWorldObjectsPackets`, sole owner, cleared on Connected and Disconnected,
duplicates kept): 0032 destructible building damage (building, attacker and credited player as
packed guids, the signed change where negative heals, the spell), 01DF game-object page text (the
goober's guid; its template's page needs a page-text query), 0412 zone light override (default and
override `Light.dbc` ids, fade-in ms), 0455 learned dance moves (two raw words; AC sends zeros at
login and nothing consumes them in 3.3.5), 0464 trigger movie (`Movie.dbc` id), 047C phase shift
(the visible phase mask). 1.12 has none of these, so no shared event means them.

**Transport-only** `ClientCommand::NativeWorldObjects`: 0481 report use (full guid; AC credits the
use-game-object achievement criterion). Labelled: a read-only scan of `Wow.exe` for the 0x0481
immediate found only source-line numbers, so when the native client sends it is unresolved and it
has no producer.

Note for the battleground owner: `wrath_battleground` tests describe 0222 as having no AC sender;
`spell_generic.cpp:2772` sends it, and this module now decodes it.

## Entity-parented animation bounds

`wrathplus_world::doodad_anim::ParentSpaceAnimBound` is the engine's public input
for an animated placement carried by another entity, including WMO game-object
props. The caller supplies the parent entity and a bounding sphere in that
parent's local frame: placement translation is already included in `center`,
and placement scale in `radius`. The engine applies the parent's current world
transform and owns animation draw gating, parking and resumption. Callers must
not pre-apply that world transform or reuse a camera-only visibility verdict
when the parent moves. Static terrain placements omit this component and retain
their already-world-space bounds.

This is engine spatial metadata, not a game-session or transport-policy API.
The production-gate regression
`doodad_anim::lazy::tests::moving_parent_bounds_promote_park_and_resume_without_double_placement`
checks moving-parent wake/park/resume and the unchanged static-world case.

## The in-game movie

`A/ui_native/wrath_movie.rs` drains 0464 from the capture and makes the movie pending, as the
reference does (`[0xbd0800]`, id `[0xbd0804]`). The play step (`0x51d710`) fires
`PLAY_MOVIE(name, volume)` from the `Movie.dbc` row: the file name up to its first `.`, and the
row's volume. For an id the table lacks, it finishes straight away.

benilla has no playback path. `MovieFrame:StartMovie` answers nil, as `0x970660` does for a movie
it cannot open, so the stock `MovieFrame_PlayMovie` hides the frame and calls
`GameMovieFinished()`. That (`0x518870`) sends the empty 0465 only while a movie is pending.
AC binds 0465 to `Handle_NULL`. `StopMovie` is a no-op with nothing playing, and `EnableSubtitles`
keeps its flag.

Labelled: the trigger handler (`0x526530`) is untraced, and so are the name's `"%s%s"` prefix
field (`+8`) and the volume's source. The player-control bit that `0x518870` clears is not modeled.
Wow.exe pass: `/tmp/azb-wireb-ghidra/movie-decomp.log`, with Experience's
`/tmp/azb-exp-misc-clean.log`.
