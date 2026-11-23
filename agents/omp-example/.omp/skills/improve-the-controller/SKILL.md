---
name: improve-the-controller
description: The check-in loop. Pick the most costly recurring failure or weakest scorecard objective from the evidence, study its cause in the Lua and the logs, then change the controller (module, threshold, listener, rotation, goal action) or record why not. Use on every check-in.
---

# Improve the controller

The check-in message lists **evidence** since the last one: game errors (`UI_ERROR_MESSAGE`),
failed goal actions with their reasons, refused pulls, how fights ended, the latest fight review.

1. **Rank.** Which problem costs the most? Count times cost: a cast failing 40 times wastes a
   global cooldown each time and drags fights out; a goal action failing every few minutes
   leaves a goal undone; one death outweighs many small errors. Check the scorecard
   (`scorecard`): which objective or need is weakest, and does it trace to the same problem?
2. **Find the cause, in the code.** Do not guess:
   - who acts at that moment: `Goals.log`, `Goals.state`, `Agent.activity()`;
   - the built-in involved: `lua_search` for the error text or the action's name, then `lua_doc`
     the function and read its source (fields you can tune are usually at the top of the module);
   - what the numbers say: `Rotation.stats`, `Combat.history`, `Goals.list()` (blocked goals and
     why), `Goals.facts()`;
   - the events around it: `events {since: 0, names: ["UI_ERROR_MESSAGE", "AGENT_", ...]}`.
3. **Decide the smallest change that addresses the cause**: a threshold on a built-in, a
   `when` condition on a rotation entry, your own rotation, an event listener that reacts, a fact
   or goal action the runner can plan with, a place it could not find (`Goals.learn_places`), a
   moment handed to you with `Agent.ask`. No hidden judgment in the Lua: named numbers.
4. **Do it** (see `write-a-controller-module`): save and load a new module; `propose_module`
   with a claim when it changes how you play; never edit a module that is on trial.
5. **Or record why not** in `notes/journal.md`: the problem, the cause you found, why you left
   it (too rare, needs a change in the client, already on trial), what would change your mind.
6. **Journal** one dated line either way, and next check-in, look at whether that failure's count
   went down.
