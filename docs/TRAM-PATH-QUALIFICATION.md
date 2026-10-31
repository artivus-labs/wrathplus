# Tram path correction — historical qualification, 2026-10-28

Implementation is already on main (`3f230cdd` and subsequent transport PRs #8–9).
This recovered record describes earlier runs and build actions, not a new test or
installation. Historical counts below were not rerun for this documentation recovery.
Attended tram traversal and exit acceptance remain pending; ship visibility/tooltip
acceptance in [TRANSPORT-ITEM-ENCHANT-QUALIFICATION.md](TRANSPORT-ITEM-ENCHANT-QUALIFICATION.md)
is not complete passenger-travel qualification.

## Failed human run

Run `target/azb-live/102530`, reported by Shannon: carriages moved sideways while riding, then the scene changed to the Dalaran-crater area with no ground. The agent did not control the human client or account during investigation.

The log records entry176081 on map369 with stationary anchor `[4.52807,8.43529,7.01107]`, model yaw1.5708 / packed quaternion approximately `[0,0,.70710754,.707106]`, and descriptor GAMEOBJECT_PARENTROTATION `[0,0,1,0]`. At00:27:11.053 the player entered exit trigger2166 at `[71.39,8.62,-3.49]`; the server subsequently announced map0 with transport176081. This is an actual transfer, not merely an incorrect zone label.

## Reference and correction

AzerothCore `core/src/server/game/Entities/Transport/Transport.cpp`, `StaticTransport::RelocateToProgress`, rotates positional keyframes with GAMEOBJECT_PARENTROTATION, adds stationary XYZ, and evaluates orientation separately. Its comment explicitly states that WorldRotation influences model rotation, not the path. `StaticTransport::Create` prefers gameobject_addon parent rotation for the path. Spawn18803 / entry176081 in the repository SQL has model yaw90degrees but addon path rotation180degrees.

The client incorrectly fed the packed model quaternion into the path evaluator, turning this tram's path90degrees from the server's path. `net/objects/transport_seed.rs` now builds the path seed from the descriptor parent quaternion. The stationary network anchor and model heading remain unchanged. Sparse create masks are zero-filled as created GameObject descriptors before reading the four components. No hardcoded tram entry or replacement spawn catalogue is introduced.

Two hermetic regressions exercise the production seed builder and position sampler: distinct parent/model bases, sparse zero components, nonzero phase, stationary base composition, model-yaw independence and identity-path preservation.

## Acceptance boundary

This is a source correction for sideways path translation, not live tram acceptance. The running human executable is not replaced, rebuilt or relaunched by this investigation. A new binary and attended crossing qualification remain necessary after the human client exits.

The exit/worldport symptom is not independently closed. The log supports wrong deck motion carrying the passenger into the exit trigger. Destination handling also needs further qualification: the server re-streams a tram during the map0 transfer, while the client's type11 sampler currently assumes lifts belong to the viewer's map. Fresh self/deck sequencing alone does not prove correct map ownership or destination coordinates. Do not infer that a new deck transform composed at the tram's map369 anchor is a valid map0 passenger pose. No speculative server-state or character-position correction was applied.

Animated orientation tracks and state-controlled lift pauses are separate existing type11 limitations; this correction does not claim to implement them.

## Source verification

Final protocol/net library tests:693 passed. Scoped world/formats library tests:2128 passed,7 ignored. Total2821 passed,7 ignored, zero failures after the final protocol/net rerun. The initial wider run compiled a temporary protocol accessor later removed in favor of a private seed-boundary helper; its API census failed, and the final protocol rerun passed. Four-package lib/tests Clippy with warnings denied passed after preserving the captured yaw under a test-local approximate-constant allowance. Changed-file rustfmt, repository LoC ratchet and git diff whitespace checks passed. Logs `/tmp/azb-tram-path-{all,final,clippy-final,fmt,loc}.log`. Full workspace/launcher/installed-data/live gates not run for this correction; dependency block0.1.6 future-compatibility warning remains. No detached verification work remains.

## Exit handoff repair and rebuilt test binary

Further inspection of the same trace establishes the phantom-deck sequence precisely: fresh self is created at the correct Ironforge world position with a stale tram tail, immediately followed by `ObjectsRemoved` for that tram. AzerothCore `MovementHandler::HandleMoveWorldportAck` explicitly sends this removal and clears the server passenger if its transport is on another map. This is not evidence that the tram legitimately belongs to map0.

The object-removal path now drops matching rider tails. During a pending native transport worldport, it also restores self's preserved living WORLD pose (not the tail), including when NEW_WORLD/self/removal are drained in one update before the player controller consumes the worldport message. The player handoff accepts that detached self and uses the ordinary destination settle/terrain hold, without a second ACK or outgoing transport tail. Normal observed rider removal preserves its latest world transform rather than restoring an old create pose. Genuine crossings still wait for missing self/deck data.

Verification: net208/world1669 tests passed,7 ignored. Final3 helper regressions pass, including same-update worldport announcement plus phantom removal; the player handler regression covers detached world-pose settle/no tail/no duplicate ACK alongside genuine recreation/carry. Strict net/world lib+tests Clippy, explicit changed-file formatting, LoC ratchet and whitespace checks pass. `scripts/check.sh` remains blocked by unrelated existing workspace formatting after branch-wide escalation. Logs `/tmp/azb-tram-exit-{tests,net-final,clippy-final,check,build}.log`.

The native play launcher built successfully and was atomically installed at `target/azb-live/bin/wrathplus` after confirming no client process was running. Build/install SHA256 both `003d321720828285323df69d4aa9b60977e09d9b5ae6f0c7c3fdc376cb883172`. Linker unwind-section-size warning and block0.1.6 future-compatibility warning remain. No client launch, input, account login, realm restart or character relocation occurred. Attended tram traversal and exit visual acceptance remain pending.
