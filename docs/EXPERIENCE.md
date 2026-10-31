# Client experience

Experience is independent of `WOW_CLIENT_PROFILE` (installed data) and the session protocol.
Without configuration, **Classic** is selected: every Plus feature is disabled, even if a feature
entry says `true`. This is the evaluation default. `WOW_EXPERIENCE=classic|plus` overrides only
the profile; invalid values stop boot rather than silently selecting a different experience.

## Plus: player setup

Close the client and edit `config.toml` in your local settings folder. This is
`$BENILLA_HOME/config.toml` when `BENILLA_HOME` is set; otherwise use `wrathplus-config/config.toml`
beside the shipped executable (development builds use the main project's `wrathplus-config/`).
If `benilla-config/` exists at that root, use it instead, even if both names exist; nothing is
automatically moved or merged. Keep any existing `[cvars]` settings and add or update these tables, rather than duplicating them:

```toml
[experience]
profile = "plus"
packs = []                    # optional local mod-pack folder names, later entries win
allow_classic_overrides = false

[experience.features]
asset_overrides = true
shadows = true
fog = true
bloom = true
tone_mapping = true
colour_grading = true
readable_text = false         # opt in for larger stock chat text
ambient_occlusion = false    # opt in; changes antialiasing at startup
```

Restart the client to load the settings. Simply setting `profile = "plus"` enables the six
features shown as `true`; readable text and AO remain off unless requested. Set any feature to
`false` to keep the rest of Plus without that effect. Use the exact names above (including
`colour_grading`); unknown names are rejected.

| Feature | What you should expect |
| --- | --- |
| `asset_overrides` | Replaces original assets with enabled local packs; no visual change with an empty `packs` list. See [Asset packs](#asset-packs) for installation. |
| `shadows` | Adds real-time directional sunlight shadows on supported world geometry. Not every translucent object or effect casts. |
| `fog` | Adds distance/height haze to the world; existing fog settings and interior fog still apply. |
| `bloom` | Adds a restrained glow around bright world areas. |
| `tone_mapping` | Uses a filmic exposure/contrast curve for the world image. |
| `colour_grading` | Cools shadows and slightly desaturates the world for a darker fantasy look. |
| `readable_text` | Raises smaller text in the ten stock chat windows to a 16-unit minimum; it does not enlarge quest text or the whole UI. |
| `ambient_occlusion` | Adds ambient contact shading on terrain and exterior models. Buildings/static WMO and indoor lighting are not included yet; sunlight and point lights are not darkened. |

World postprocessing does not apply to portraits or character-selection panes. AO requires a
supported GPU and MSAA off: enabling it at startup selects MSAA Off plus SMAA High edge smoothing.
Unsupported devices skip AO with a warning. **Restart after changing AO or pack settings**;
AO's antialiasing choice and asset mounts are fixed at startup. Config-file edits for other
features also take effect on the next launch; runtime developer switches do not save changes.
There is no player-facing in-game Plus toggle yet; dev builds have the console switch below.

### Runtime switch (dev builds)

`/console experience` prints the profile and every feature's effective state;
`/console experience classic|plus` switches the profile and `/console experience <feature> on|off`
sets one feature (config spellings, e.g. `colour_grading`). Changes are session-only and reach the
renderer the same frame through `sync_lighting`/`sync_text`, so a side-by-side capture can flip
Classic and Plus in one session (for example `WOW_PROBE_CHAT="/console experience plus"`).
`asset_overrides` and `ambient_occlusion` print `(fixed at startup)`: packs mount and AO's
antialiasing latch at boot. The command is registered only with the `dev` feature, so the player
build keeps the stock console command set.

To return to the stock look, set `profile = "classic"` and restart. Classic masks every graphics
and text feature, even saved `true` entries. Leave `allow_classic_overrides = false` to use only
original archive assets. A launcher environment setting `WOW_EXPERIENCE=classic|plus` takes
precedence over the saved profile, so check it if the selected profile seems unchanged.

Capture runs ignore persistent player configuration, as they do for CVars; an explicit
`WOW_EXPERIENCE=plus` is needed to select Plus for a capture. The sections below describe
implementation limits; live visual quality and Classic pixel equality require manager captures.

`experience::Experience` is the app's Bevy resource and the only feature-query API. Runtime callers
use `set_profile` and `set_feature`; switching back to Classic masks existing feature overrides.
Runtime changes are session-only and do not overwrite the boot configuration. Asset packs are
mounted at boot, not hot-reloaded. Classic asset overrides require a separate explicit
`allow_classic_overrides = true`; that permission never enables graphics features under Classic.

## Asset packs

Put enabled packs in `<home>/mods/<pack>/`, with loose paths mirroring MPQ paths, for example
`mods/my-art/World/KhazModan/Ironforge/PassiveDoodads/Trees/IronforgeLeaves01.blp`.
The `packs` list is ascending priority: its last copy wins, then the original archive chain is the
fallback. Paths compare case-insensitively with either slash separator. Pack names must be single
folder names; traversal, absolute paths, symlinks and ambiguous case-folded names are rejected.
Pack files are local trusted content, not sandboxed plugins. Mount names are snapshotted at boot;
do not move or mutate the pack tree while the client runs. An unreadable winning file is an error,
not a silent fallback to different art. Byte reads log the winning loose file's provenance.

The policy is resolved once by app boot. Classic never even scans configured packs without the
separate `allow_classic_overrides` permission. Ordinary `Chain::open`/`open_profile` calls and bare
UI test harnesses remain archive-only. Packs are not hot-mounted by runtime profile setters;
restart after changing pack configuration.

### Non-stock patch archives

The archive chain mounts Blizzard's 3.3.5a archives alone by default: `patch.MPQ` to `patch-3.MPQ`
and `patch-<locale>.MPQ` to `patch-<locale>-3.MPQ`. A repack's later archives (`patch-4.MPQ`,
`patch-enUS-4.MPQ`, …; often a server's own login screen, art or DBCs) are skipped with a warning
naming each one. To mount them, add to `config.toml`:

```toml
[data]
nonstock_patches = true
```

or launch with `WOW_NONSTOCK_PATCHES=1`. They mount above the stock patches in numeric order, as
the original client would. This is independent of Classic/Plus and of the loose packs above, which
stay the way to ship replacement art; restart after changing it.

### Reader audit

- `Chain::read` (and its `read_file` alias), `contains`, `list` and `list_filtered` share one policy;
  enumeration includes pack-only paths and the winning loose file's size.
- The boot `mpq://` reader receives that policy before Bevy's asset plugin builds. BLP, M2, WMO,
  ADT and WDT loaders use it, including ADT layer byte dependencies and sampler-marked doodad
  textures. Directory enumeration on the Bevy source remains unsupported as before; these
  loaders request explicit paths, while chain clients use `list_filtered`.
- World startup applies the same policy to `WorldAssets`; its DBC/model/texture/audio reads all
  use that chain. The stock UI's independent once-per-process chain gets the policy from an
  explicit boot initializer before the first VM (`ui_script::initialize_asset_overrides`);
  standalone test VMs do not discover player packs.
- Skin compositing no longer uses a direct archive handle. Ambience streams select a loose file
  before taking the original MPQ streaming branch; Classic retains sector-at-a-time MPQ audio.
- Public archive-only asset-source registration functions intentionally keep their original
  defaults; the production launcher calls the explicit override registration function instead.
- Remaining independent chain opens in asset/world/model/UI/animation test modules and the
  `cfg(test)` WMO/light/liquid audit modules deliberately validate original installed data. The
  direct archive handle in the sound mixer is likewise in its installed-data test module.
- Addon-owned loose-file fallback is a separate existing mechanism, after the chain lookup; it
  does not shadow client archive files or enable a mod pack.

### Real-asset pack (installed test and demo pack)

`wrathplus-assets` `override_real_data` builds a pack from installed Wrath files into a temporary
folder: Elwynn's `Lamppost.m2` (with its SKIN/ANIM siblings) becomes `Ballista.m2`, and the first
`Tileset\Elwynn\*grass*.blp` becomes the first `Tileset\Durotar\` texture. Through the
production `register_mpq_source_overrides` and Wrath loaders, the lamp loads with the ballista's
geometry, the terrain texture with the Durotar pixels, `override_path` names the pack file, and an
untouched model (`ElwynnCampfire.m2`) still loads from the archives. To stage the same pack for a
live session (local only, never committed):

```sh
WOW_CLIENT_PROFILE=wrath WOW_LOCALE=enUS WOW_DATA=/Users/shannon/Games/Client/Data \
  AZB_DEMO_PACK_OUT="$BENILLA_HOME/mods/azb-demo" \
  cargo test -p wrathplus-assets --lib write_demo_pack -- --ignored --nocapture
```

then `[experience] profile = "plus"`, `packs = ["azb-demo"]` (or Classic with
`allow_classic_overrides = true`) and restart. Goldshire's road lamps should be ballistas and
Elwynn grass Durotar ground; the log shows `asset override` lines with each pack source.

Live replacement captures (texture/model and Classic before/after) are performed by the manager;
the worker does not launch the client. Synthetic tests cover policy resolution and the full Bevy
texture-loading pipeline without installed Blizzard assets.

## Plus lighting (first stage)

`fog`, `bloom`, `tone_mapping` and `colour_grading` are independent runtime switches. The app
publishes their resolved policy to the renderer each frame; the renderer used without the app
starts with every switch off. Classic masks all four, including saved explicit overrides.
`shadows` is the separate sun-shadow path described under
[Plus shadow implementation](#plus-shadow-implementation--material-extension-seam) below.

The Plus postprocess is a separate shader/pipeline family. It runs on the world image before UI
composition, both for a direct world camera and the backdrop behind the UI. Only `WorldCamera`
views opt in; portrait and glue/character panes retain the original FFX shader. Cache keys include
the three postprocess switches so runtime changes cannot reuse a previous look. Plus filters work
in linear colour, bloom adds a bounded bright-region blur, the filmic curve applies restrained
exposure, and grading cools shadows/desaturates the scene. The final result is encoded for the UI
backdrop or decoded for sRGB presentation as appropriate. Stock death-state colour remains last.
The original Classic FFX shader is unchanged.

Plus fog adds radial extinction with exponential height density to terrain, animated and static
models, liquids and world effects. It respects the existing fog toggle, interior fog span/colour
and material fog policy. Height density is relative to the camera (base eight world units below
it, falloff 0.025), not an assumed continent sea level. WDL's distant hull deliberately retains its
full-fog endpoint to avoid exposing the far seam. Existing light-buffer spare lanes carry the
policy: row 5.z enables it, row 19.z/w supply height base/falloff. Under Classic those lanes remain
zero and the original planar fog arithmetic is taken; new calculations occur only inside the
zero-disabled branch. No light layout or portrait/glue buffer changes are required.

Headless policy/packing and shader checks complement the full app suite; they cannot certify
Classic pixel equality or Plus appearance. The manager performs runtime switches and side-by-side
captures, including portrait/glue isolation, before accepting the rendering result.

### Plus shadow implementation / material extension seam

The world-only shadow sun exists only while the Shadows feature is effective and a world camera
exists. It adds no illumination (illuminance zero), uses layer 0 and two bounded cascades at 30/120
yards. Classic never spawns it. `ModernLighting` writes the receiver gate in shared light row 12.w;
row 12.xyz and all other existing field offsets remain unchanged. Booth buffers leave this gate zero.
Terrain and exterior model receivers attenuate sunlight, not ambient, point lights, emission or
interior probes. Classic retains its original lighting expressions in the zero-gate branch.

`wrathplus-assets/src/materials.rs` dispatches PREPASS_PIPELINE before forward specialization and
uses `shaders/wow_shadow.wgsl`, with the child helper `shadow_material.rs`. Forward attribute and
blend/depth logic must never run on this shadow path. The caster duplicates the complete
WowLight/ModelParams ABI: future two-stage material fields must be mirrored there. Palette skinning,
rig origins and animated UV affine match the visible model. Prepass inputs are position 0, UV 1/2,
color 7 and custom joints/weights 10/11 (not forward UV 2/3 and color 5).

Caster scope is canonical opaque/mask geometry plus source-cutout fade twins (marker bit 10,
224/255). Unknown opaque-source Blend twins are conservatively excluded: a future caller may
supply WOW_SHADOW_SOURCE_OPAQUE only with authored provenance included in its cache key.
Excluded keys: clutter, additive, no-depth-write/test, modulate/modulate2x, zfill, far-side, sky;
excluded mesh lanes: merged fade sphere and merged probe slot. Uniform exclusions are marker
mask 0x3b87 (bits 0/1 no depth, 2 glow, 7/8 multiply, 9 zfill, 11 far-side, 12 env-map, 13 sky),
clutter_fade.w and UI clip anim_slots.w. Authored translucent geometry does not cast. WMO MOCV
alpha is lighting data, not coverage. Shadow coverage never uses player-distance or MeshTag fade.
Merged geometry, clutter and unsupported translucent/effect casters remain outside this first
bounded shadow implementation. Runtime visual/bias quality and Classic pixel equality require
manager-run live captures; worker headless tests do not establish those results.

### Readable chat text (Plus UI proof)

`[experience.features] readable_text = true` opts Plus into a 16-unit minimum font height
on the ten named stock chat windows. It is off by default, including in Plus. Classic masks
it even when explicitly requested. Existing font faces, outlines, shared Font objects, quest
text and unrelated message frames are untouched. Runtime feature/profile setters use the same
registry as graphics. Disabling the feature or returning to Classic restores authored heights.

The engine value changes before the UI tick, not through render-only scaling: Lua `GetFont`,
layout and painting agree. Addon font setters remain immediately authoritative; their raw size
may be visible until the next per-frame policy sync, which adopts it as the new baseline and
never stacks sizes. A setter after the last sync wins when disabling. The bounded check only
looks up ChatFrame1..10, only writes changed sizes, and never creates missing font regions.
On an untouched Classic VM it allocates no policy state and performs no font lookups or writes.
The first proof is chat-only; quest-log overrides await verification of isolated region ownership.


### Offline showcase path (schema / sampler)

The showcase path is separate from the stock server-triggered cinematic player and the eval
capture harness. This first unit implements data validation and CPU sampling only; it does not
register a recording plugin, read environment variables, or change any camera or clock.
The driver and the `WOW_SHOWCASE_PATH` launch knob will follow in a separate unit.

A TOML path fixes `map` and `minute` (light-band minute in `[0, 1440)`) for matched Classic/Plus
scenery clips. `[[keyframes]]` has `at` in seconds, absolute `eye` and `target` in **WoW coordinates**
(yards), and optional `fov_degrees` (vertical, default 60, allowed 10..120). Times start at zero
and increase strictly; the last time is the duration, at most ten minutes and an integral number
of 1/60-second frames. There must be 2..256 keys. Nonfinite/out-of-bounds coordinates, coincident
views (including sampled crossings), unknown fields, and invalid times are refused before recording.

Output is fixed at 1920x1080, 60 fps. N frames sample `[0, duration)` at integer `index / 60`,
not accumulated deltas; the final endpoint is not duplicated. Eye, target and FOV use bounded
cubic ease-in/out interpolation, continuous with zero velocity at each key and no overshoot.
Each waypoint deliberately comes to rest; use two keys for a single uninterrupted flight.
An explicit showcase can record either experience, but ordinary Classic, normal play, and the
existing eval capture remain unchanged. Offline recording contains scenery only, no network actors.

```toml
map = 0
minute = 720.0
[[keyframes]]
at = 0.0
eye = [-9000.0, -100.0, 120.0]
target = [-8950.0, -100.0, 90.0]
[[keyframes]]
at = 10.0
eye = [-8900.0, -100.0, 120.0]
target = [-8850.0, -100.0, 90.0]
fov_degrees = 60.0
```

This is a schema example, not a manager-verified filming route. No runtime image or Classic
pixel-equality claim follows from CPU sampler tests.

### Recording the offline sequence (explicit developer mode)

`WOW_SHOWCASE_PATH=<path.toml>` selects the recorder before plugins build. The app starts directly
in-world with `connect: false`: no network thread starts. It never creates
an avatar or sends camera/player commands. The currently selected experience is retained, including
Classic; no Plus gate applies. `WOW_CAPTURE` and the showcase knob are mutually exclusive. With
no path knob, recorder preparation and registration do nothing and existing eval selection,
window sizing, camera controls and time strategy remain unchanged.

`WOW_SHOWCASE_OUT=<directory>` chooses the sequence directory (default `target/showcase`). It must
be empty; previous clips are never overwritten. The recorder copies the path and writes a manifest
with profile, map, minute, resolution, FPS and expected frame count. `result.toml` distinguishes
completion from failure. `frame000000.png`, etc., are RGB PNGs at exactly 1920×1080; the recorder
refuses an incorrectly sized readback. Window/DPI settings are owned by recording mode, not
`WOW_WIN`, `WOW_DPI`, `gxResolution`, or `RestartGx`. A display too small to host 1080p cannot record.

The camera is pinned after input/stock camera writers and before world streaming. Lighting uses
the authored minute with server-time following off (the engine samples integer minutes, so a
fractional minute is truncated; the manifest records `effective_minute`). Simulation advances once by 1/60 second only
after the previous PNG is saved; streaming/readback/write waits have zero game-clock delta. Each
pose waits for focus residency and presentability, then eight render frames for warm-up. This is
a bounded readiness gate, not a guarantee that every distant shader/asset is visually settled.
Each pose has a 120-second wall-clock deadline covering streaming, readback and disk write;
failures exit nonzero. Offline clips contain scenery, lighting and shadows, not network actors.
Rendering can be slower than realtime; encoding defines the playback rate.

Manager-only launch example (workers never run the client):

```sh
WOW_EXPERIENCE=plus WOW_SHOWCASE_PATH=/tmp/path.toml WOW_SHOWCASE_OUT=/tmp/plus-clip \
  cargo run -p wrathplus
ffmpeg -framerate 60 -start_number 0 -i /tmp/plus-clip/frame%06d.png \
  -c:v libx264 -crf 18 -pix_fmt yuv420p -movflags +faststart /tmp/plus-clip.mp4
```

Use the same path and `WOW_EXPERIENCE=classic` with a different empty output directory for a
matched comparison. Headless tests establish gating, sampler/state/clock contracts only; manager
runs must still verify actual streaming, screenshots, shadow appearance and Classic pixel equality.

### Plus highlight bloom (c5)

The existing world-only `bloom` toggle now uses a smooth linear-light knee (threshold 0.35,
width 0.15), with hue-preserving RGB scaling and a bounded 0.25–0.40 zone-dependent gain.
Dark blurred values below 0.20 contribute nothing. This softens the cutoff on saturated bright
water/spell highlights without changing exposure, tone mapping or colour grading. It is a
screen-colour effect, not an emissive/material mask: bright terrain or sky can bloom too. The
threshold runs after the existing blur, preserving the shared underwater haze input and adding
no render passes or targets. Classic uses the unchanged stock FFX shader; UI and portrait/glue
views do not opt in. Manager clips must verify the actual highlight range and appearance.

Ambient occlusion is opt-in and uses MSAA-off cameras only, never switches sample counts live,
and logs an unsupported request once. A Plus AO request at startup selects MSAA off plus SMAA;
Classic and AO-disabled Plus retain their existing startup selection. See the staged AO scope
and remaining manager-owned visual checks below.

### Curated comparison routes (c6)

`docs/showcase/` contains five 12-second map-0 scenery routes: Northshire Abbey sweep,
Goldshire dusk, Stormwind gate approach, Elwynn lake sunrise, and Westfall farm. Coordinates
are WoW yards, not map percentages. Landmark seeds were checked against the installed Wrath
enUS Azeroth ADTs using `Chain::open_wrath`, `load_tiles_around` and `terrain_height_at` at
101 equally spaced points along each straight eye-XY segment. The cubic sampler follows that
same segment without overshoot. Sampled ground ranges were respectively 79.12–86.94,
55.84–59.02, 74.11–90.97, 52.14–59.57 and 35.32–37.32 yards; eyes use fixed heights of
115, 85, 120, 85 and 65 yards. Underlying MCNK area IDs at the endpoints were respectively
9, 87, 12, 12 and 107. No installed assets or extracted tables are committed.

These are first-pass authored framings, not live-approved clips. Terrain samples do not prove
clearance from trees, WMO roofs or walls, nor certify lake surface visibility. The elevated eyes
leave generous terrain clearance; the manager must inspect landmark framing and occlusion.
The production schema regression validates all five routes and every sampled pose without
installed data. Fixed minutes are authored lighting choices, not observations of sunrise timing.

Manager-only, from the repo root, using an **already-built dev-enabled** binary:

```sh
scripts/showcase-pair.sh /absolute/path/to/wrathplus \
  docs/showcase/northshire-abbey.toml /tmp/northshire-pair
```

The script renders Classic then Plus into separate sequence directories under a **new** root,
clears `WOW_CAPTURE`, preserves the caller's data/locale and other renderer settings, and writes
`encode-side-by-side.sh`. Run that command separately with ffmpeg to produce a 3840×1080,
60-fps hstack (Classic left, Plus right). It does not build, encode, overwrite old output, or
contact a server; a rendering failure stops before the second run/encoder command. Workers do
not execute this script with the client. Plus feature settings still come from the usual config.

### Optional offline showcase actors

The path accepts up to 16 optional `[[actors]]` entries with a nonzero installed
`CreatureDisplayInfo` `display_id`, finite bounded WoW `position = [x, y, z]`, optional `yaw`
(radians, ±one turn, default 0), and `scale` (0.1–4, default 1; multiplier on installed scale).
These are visual placements, not server creature-entry IDs, GUIDs or network units: no object
store, collision, combat, interaction or nameplates. Empty/absent actors keep scenery-only capture.

The recorder uses the existing world display cache and unchanged world batch materials, not
portrait/studio lighting. Every supported actor has a paused Stand at t=0 and global bone tracks
sampled once at t=0. Version one explicitly rejects humanoid customization/equipment, unskinned
or missing Stand models, effects, lights, bone billboards/billboard cards/welded cards, ground quads and animated
material alpha/RGB/UV channels. Missing rows/models/textures and palette exhaustion fail the
recording; no cube, bind-pose fallback or silently omitted part is substituted.

Capture waits for model/recursive assets, **all** effective material image dependencies,
inverse bindposes, propagated instance transforms and finite world rig palettes, then the normal
eight render warm-up frames. Generated resident images need not have AssetServer load state.
Manifest actor entries record normalized requested model/skin paths, authored placement,
installed/effective scale and frozen pose. These paths are **not winning-archive provenance**;
actual override consumption remains in existing reader tracing (`asset override` with path/source).
No actor route is claimed live-verified; the manager owns framing/render checks and client runs.

### Ambient occlusion: first staged slice

`[experience.features] ambient_occlusion = true` opts Plus into Bevy screen-space ambient
occlusion (SSAO). It is **off by default**, and Classic always masks it. Configure before launch:
the boot policy selects **MSAA Off + SMAA High** for the world camera. Disabled Plus and Classic
retain their existing configured AA, including `gxMultisample`/`WOW_MSAA`; no config/CVar is
rewritten. AA is latched until restart. Disabling AO or switching to Classic removes SSAO and
its owned depth/normal prepasses but leaves that boot AA choice latched. Enabling AO at runtime
on a multisampled camera is a logged no-op, not a live MSAA switch. Unsupported GPUs (fewer than
five storage textures per shader stage) and headless renderers likewise do not enable SSAO.
Portraits, glue views and UI cameras are not opted in.

The first slice covers terrain and Bevy-rendered exterior M2 models. SSAO attenuates the separated
ambient contribution before light clamping, not sun, point lights, specular or unlit output.
Indoor lighting is untouched: interior probes contain mixed ambient/direct DC terms that cannot
safely be darkened wholesale. No shared-light or two-stage material ABI is expanded. SSAO uses
Bevy mesh-view binding 16. Camera model depth/normals reuse the visible model shader's transforms,
UV animation, two-stage source cutout, clutter and merged-placement coverage independently of
shadow-caster exclusions. Non-depth, additive/multiply, sky and far-side lanes are excluded.

**Retained static-GX geometry (including buildings) has a dedicated AO camera pass.**
When an MSAA-Off world view has active SSAO resources, it writes camera normals and depth
between Bevy's late prepass and end-prepasses, then refreshes the depth copy SSAO reads.
Colour and camera passes share retained region selection, portal/group admission, live runs,
mesh allocator ranges, vertex kills, authored coplanar order, far clipping, texture wrapping
and source alpha-key coverage. All matching normal and AO-forward pipelines must be ready
before the camera pass contributes occluders. The ordinary forward pass remains in its original
position; there is no renderer substitution. With AO inactive, its original pipeline and binding
layouts remain selected.

Exterior retained WMO surfaces and static M2 models receive ambient-only AO via a separate,
optional texture group; mesh-view binding 16, shared-light and material layouts are unchanged.
Indoor authored/MOCV lighting, interior SH probes, direct sunlight, point lights, emissive and
unlit output are not attenuated. Interior opaque geometry still contributes camera coverage.
WMO model-lane fallbacks do not receive AO. Transparent and other non-prepass lanes are likewise
not certified as AO occluders. Two-stage materials remain outside the first-texture-only retained
GX lane; their camera coverage uses the existing model path.

Headless shader composition and policy tests establish wiring, not appearance or Classic pixel
equality. The manager must verify captures, silhouettes and AA/post-pass compatibility live.

An ignored, on-demand GPU smoke test also validates retained-GX camera-normal and AO-forward
entry points against the production bind-group and vertex layouts on a real device. It submits a
4×4 synthetic triangle for every cutout/two-sided combination, without a window, game client,
network, archives or world plugins. Ordinary test runs leave it ignored, so GPU-less CI is unchanged.
Run explicitly (under the shared heavy-build lock and usual worker build limits):

```sh
cargo test -p wrathplus-world --lib retained_gx_gpu_smoke -- --ignored --nocapture --test-threads 1
```

The explicit run requires a working graphics adapter and fails rather than silently skipping when
none is available. It checks shader/pipeline/binding/draw validation, not image fidelity or AO strength.

### On-demand Plus postprocess GPU validation

The ignored `ffx_plus_gpu_smoke` test initializes a real device without a window, game,
archives, or network. It uses production FFX initialization/layouts and combine specialization:
three Plus filter passes and all seven nonempty bloom/tone-map/colour-grade feature subsets,
each dry/underwater and gamma-output on/off (31 submitted 4×4 fullscreen draws). It uses the
embedded shader libraries and production fullscreen vertex shader, not an inferred bind layout.

```sh
CARGO_TARGET_DIR=/tmp/azb-target-experience CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 \
  lockf -k /tmp/azb-heavy.lock env -u WOW_CLIENT_PROFILE -u WOW_LOCALE WOW_DATA= \
  nice -n 15 cargo test -p wrathplus-world --lib ffx_plus_gpu_smoke -- \
  --ignored --nocapture --test-threads 1
```

Validated on Apple M3 Max/Metal. This checks shader composition, production pipeline layouts,
bindings and draw submission, not visual correctness or Classic pixel equality. Ordinary tests
never initialize this GPU test. The embedding-only Bevy shader-library loader has no RenderApp;
its ClearColor-extraction and compressed-format-support diagnostics are expected, not GPU
validation failures. The test does not exercise the 2D UI combine attachment lane, which Plus
world postprocessing does not select.

### On-demand WoW shadow GPU validation

The ignored `wow_shadowcaster_gpu_smoke` test uses a headless real device and Bevy's production
prepass initialization, material specialization, shader composition and material allocator. It
checks 4×4 `Depth32Float` readback for opaque coverage, alpha-mask pass/discard, the keyed
no-depth-write exclusion and an identity custom-rig palette. It opens no window and loads no
world or network actors. This validates tiny fixture coverage and binding/pipeline compatibility,
not animated skinning correctness, live shadow orientation, world streaming or visual fidelity.
Classic and ordinary GPU-less test runs leave it ignored; no production rendering is changed.

Compile under the shared heavy-build lock, then run the resulting test executable **outside**
that lock. Both stages must have a hard process timeout (for example `timeout --kill-after=10s
300`); never hold the shared lock for an unbounded on-demand GPU run. Preserve the usual worker
build environment (`CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=4`, `CARGO_INCREMENTAL=0`, `nice -n 15`),
and unset `WOW_CLIENT_PROFILE`/`WOW_LOCALE` with `WOW_DATA=`. For example, obtain the executable
from the `compiler-artifact` JSON emitted by `cargo test -p wrathplus-world --lib --no-run
--message-format=json`, then execute:

```sh
env -u WOW_CLIENT_PROFILE -u WOW_LOCALE WOW_DATA= \
  timeout --kill-after=10s 300 nice -n 15 "$WORLD_TEST_BINARY" \
  wow_shadowcaster_gpu_smoke --ignored --nocapture --test-threads 1
```

GPU completion and map callbacks are bounded to 30 seconds inside the test; the outer process
limit also bounds shader/setup work. A successful fixture run on Apple M3 Max / Metal validated
all five cases. As with the postprocess smoke above, the embedding-only shader-library loader
emits expected missing-RenderApp and compressed-format-support diagnostics during setup.
