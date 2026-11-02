-- Example agent module: walk in short legs, turning between them, driven by wp.engine.
-- Save it as a module (PUT /files/demo.wander or the save_module tool), then in Lua:
--   require("demo.wander"):start()   -- and WanderBot:stop(), WanderBot:describe()
local E = require("wp.engine")
local V = require("wp.values")
local bot = E.new({ interval = 0.25 })
bot:value("pos", function() return V.position() end, 0.5)
bot:trigger("leg due", { interval = 4, check = function() return true end })
bot:action("stop", { execute = function() MoveForwardStop() return true end })
-- Turning keeps the engine busy for 0.6 s (its duration); then the continuer walks.
bot:action("turn", { execute = function() TurnLeftStart() return true end,
  duration = 0.6, continuers = { "walk" } })
bot:action("walk", {
  execute = function(c) TurnLeftStop() MoveForwardStart() c:set("legs", (c:get("legs") or 0) + 1) return true end })
bot:strategy("wander", { state = "noncombat",
  triggers = { { "leg due", { { "stop", E.HIGH }, { "turn", E.NORMAL } } } } })
bot:enable("+wander")
WanderBot = bot
return bot
