# AzerothCore server-spline surface swimming

## Deliberate compatibility deviation

The maintainer-authorized AC interoperability policy recognizes one server-authored
surface seat even when no live SWIMMING bit was broadcast. It is **not** a global
lowering of the client's native swimming threshold and does not invent a spline
swim flag.

Build 12340 `0x730d10` compares water surface minus feet Z with collision height
multiplied by 0.75 (strict entry; ordinary exit at `0.75*h - 1/36`). The non-local
live-spline start arm (`0x721210` -> `0x6ebf70` -> `0x989660`) sets live SWIMMING.
No native lakebed-based entry rule was established. The existing native predicate
and local controller retain those thresholds.

AC `Entities/Object/Object.cpp::GetMinHeightInWater` and
`common/Utilities/Physics.h` instead seat an unmounted surface swimmer at
`(1 - 4*1040/10202)*h = 0.5922368*h` below water, capped against ground by
`UpdateAllowedPositionZ`. At human height 2.031 this is 1.202833 yards, below
native entry 1.52325 and ordinary exit 1.495472. The reported bot event-log
census had MonsterMove but no decoded UnitMove/SplineMoveMode swimming state.
MonsterMove carries spline flags, not live movement flags. AC's spline CanSwim
capability is not evidence of current swimming.

## Bounded policy

`motion::spline::mark_swimming_creatures` retains its native decision and ORs in
an AC surface-seat decision, only with the ordinary `may_swim` descriptor permit
(and never CANNOT_SWIM):

- The body is server-derived, not local/embodied or a real relayed mover.
- A **live**, grounded, non-deck spline exists; idle/create-only, flying and
  transport-local paths do not acquire the exception.
- Its own room is settled and supplies water (not magma/slime).
- No granted water-walking or hovering mode is present for the exception.
- A resolved finite positive collision height is required; default/unresolved
  height cannot justify the exception.
- Feet immersion is within **1/36 yard** of the AC immersed-height fraction.
  This bounded seat-match tolerance reuses the existing absolute native exit
  band; it is not an additional entry threshold or a permanent swim latch.
- A one-sided solid-body down-ray, from feet + 1/36 yard, must hit support within
  **4 yards** below feet. Normal Y must exceed 0.5; feet must be more than
  1/36 yard above support; surface-to-support depth must strictly exceed 0.75*h.
  Missing/unstreamed support is **not** proof of deep water. This conservative
  reach deliberately leaves deeper-than-probe/missing-bed cases unclassified.

The decision is recomputed from the spline sample before ground clamping.
A positive marker exempts that clamp and feeds the existing animation `unify`
spline leg. No position snapping, relay flags, packet layout, local physics,
height derivation or animation selector changes are made. The earlier
Changed<CollisionHeight> cache repair remains; collider changes invalidate the
cache, and live spline samples revisit it each frame. Live splines and already-marked swimmers are always revisited, including path
removal (the marker keeps eligibility to exit even without a prior clamp memo). Once the exception no longer applies, the unchanged
native depth predicate decides; there is no compatibility-only latch on shore.

## Regression boundary

`motion/ac_swim_tests.rs` executes real spline sampling, indexed room-water
queries, collider ray support, the marker and clamp in a chained schedule, then
production animation-state unification. The same AC feet seat over measured deep
support must swim and retain server altitude, while shallow support and unknown
support must not. Negative controls cover CANNOT_SWIM, missing permit, waterwalk,
local/relay authority, flying/deck paths, lakebed walking, off-seat immersion and
unsettled room. A transition regression releases on shore, permit loss, waterwalk,
path removal and height restamp. Native predicate and collision-height cache tests remain separate.

These are synthetic offline geometry/authority regressions, not a live visual
retest of the reported lake or proof that every remote model has a Swim clip.
