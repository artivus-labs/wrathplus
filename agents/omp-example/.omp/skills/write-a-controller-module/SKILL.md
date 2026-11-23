---
name: write-a-controller-module
description: Write, load, test and keep a Lua controller module in the WrathPlus client (save_module, require, AgentReload, propose_module). Use before writing or changing any of your own Lua.
---

# Write a controller module

1. **Search first.** `lua_search` the behaviour you want (e.g. "drink", "rotation", "flee",
   "vendor"). If a built-in does it, tune its fields instead of writing your own. Read what you
   will call with `lua_doc` (signature and source), never guess an argument order.
2. **Shape.** One module, one job. It returns a table with `start()` and `stop()`; `stop()` must
   undo everything `start()` did (unregister events, `SetScript("OnUpdate", nil)`, stop walks or
   fights it began), so the judge can swap it and `AgentReload` can replace it.

   ```lua
   -- mage.drink: drink to <threshold> mana out of combat before the next pull.
   -- When: always on while levelling a mana user. Thresholds: M.below (0.35), M.upto (0.9).
   -- Measured: (fill in from the ledger: trial N, idle share -0.03, deaths/h unchanged).
   local M = { below = 0.35, upto = 0.9 }
   local frame = CreateFrame("Frame")
   local next_check = 0
   function M.start()
     frame:SetScript("OnUpdate", function()
       if GetTime() < next_check then return end
       next_check = GetTime() + 1
       -- read state, act through the built-ins (Survive, Goals, Combat...)
     end)
   end
   function M.stop() frame:SetScript("OnUpdate", nil) end
   M.start()
   return M
   ```
3. **Thresholds are named numbers** on the module table, documented in the header. No hidden
   judgment: when a moment needs judgment (someone talking, an odd mechanic), call
   `Agent.ask(question, context, on, {timeout = 60, default = ...})` and let the planner answer.
4. **Load and test.** `save_module`, then `lua: return require("name")` (first time) or
   `AgentReload("name")`. Read `next` from `events`, let it run, then read
   `events {since: next, names: ["AGENT_", "UI_ERROR_MESSAGE"]}` and the module's own state.
   A Lua error in a frame script shows up when you call into the module; check with a small `lua`
   read.
5. **Wire it in.** Add `require("name")` to `autoexec` so it comes back after a reconnect.
6. **Keep or judge.** A new module or a bug fix: keep it. A change in how you play (pull rules,
   rest bands, routes, rotation order): `propose_module(name, source, claim)` with a claim the
   scorecard can bear out, then leave the module alone until `ledger` shows the verdict.
7. **Write it down.** Update the header's "Measured" line from the ledger and add a line to
   `notes/journal.md`.
