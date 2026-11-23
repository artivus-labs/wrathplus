---
name: reflect
description: Review how the character is doing from the scorecard, needs and the judge's ledger, find the weakest objective, and propose module changes with claims. Use for a scheduled reflection or after a bad stretch (deaths, a stall, a regressing need).
---

# Reflect

1. **Measure.** `scorecard {sample: true}`: read `current`, `needs` (every gap above 0 is
   behind) and `trends` (15/60/240 minutes and all). Note the weakest objectives: deaths/h,
   idle share, a need whose gap grows, xp/h falling, no exploration for hours.
2. **Evidence.** `ledger`: what is on trial now, what was promoted, rolled back or inconclusive,
   and why (per-objective deltas and tolerances). Do not re-propose a rejected idea unchanged.
3. **Look at what happened.** `events {since: 0, names: ["AGENT_", "PLAYER_DEAD",
   "UI_ERROR_MESSAGE", "CHAT_MSG_WHISPER"], limit: 100}` for recent failures, refusals and
   deaths; `lua: return Goals.list(), Goals.state, Goals.log` for what the runner is stuck on.
   `modules` and `lua_doc` for the code involved.
4. **Choose at most two changes**, each aimed at a weak objective without giving up another:
   e.g. a death cause (pull size, rest band, flee threshold), idle time (a goal that keeps
   blocking), a need (a profession not trained, gear not upgraded, no travel to a fitting zone).
   Prefer tuning named thresholds over rewriting logic.
5. **Propose.** `propose_module(name, source, claim)` per change, with a claim the scorecard can
   check ("fewer deaths per hour, xp/h no lower"). One trial runs at a time; others queue.
6. **Record.** A dated entry in `notes/journal.md`: what you saw (numbers), what you proposed and
   why. If you learnt a procedure, write or improve a skill.
7. **Report** in a few lines: the weakest objectives, the evidence, what you proposed, and what
   the planner should do meanwhile.
