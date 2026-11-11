-- wp.professions: the secondary professions every character can use, Cooking (food) and First Aid
-- (bandages), learnt, levelled and used without being asked. Loaded for every agent as the global
-- `Professions` after `Goals`; set `on`, `craft` or `learn` (to {} for none) to change what it does.
--
--   Know:   Professions.known() → { [name] = { rank, max } } for every profession on the skill
--           lines (GetSkillLineInfo; collapsed headers are expanded to read them).
--           Professions.book[name] is the last scan of its tradeskill window: rank, max and the
--           recipes with their index, difficulty ("optimal", "medium", "easy", "trivial"),
--           available count, reagents { name, need } and whether a tool such as "Cooking Fire"
--           is required. The window is opened by casting the profession (CastSpellByName), as a
--           player clicks it in the spellbook; Professions.scan(name) reads an open one.
--   Make:   Professions.makeable(kind) → how many food or bandages the bags can make now, from the
--           book's recipes. A recipe whose tool is a fire is made beside one: a spell-focus object
--           in sight, else a Basic Campfire (taught with Cooking); the server's "Requires Cooking
--           Fire" refusal also lights one, as the client never shows a missing focus.
--   Use:    Professions.best_food() → bag, slot, name of the food a rest should eat: our own cooking
--           first (Professions.products), then the highest level we may use;
--           Professions.bandage([unit]) bandages (default ourselves) unless Recently
--           Bandaged, in or out of combat, and says why not.
--   Keep:   Professions.keep(item [, remaining]) → keep, cap: whether a reagent we cook or bandage
--           with (Trade Goods meat or cloth, or a reagent of a scanned recipe) should stay in the
--           bags. With `remaining` (what would be left after selling a stack) it says whether that
--           sale would leave fewer than the cap (`keep_cap`, 40 of each).
--   Train:  at a Cooking or First Aid trainer, Upkeep calls Professions.train(): it learns the next
--           rank when there is one, then the recipes the money covers within `recipe_share` of it,
--           and notes what the trainer cannot teach yet (a rank above its cap, a level).
--
-- Facts it adds to Goals (cheap: bag reads and the cached book): can_cook, can_bandage (a batch,
-- `batch` or what the stock lacks, can be made now), food_low, bandages_low (under `stock`),
-- can_skill_up (a recipe that still gives skill can be made), profession_to_learn (a profession in
-- `learn` is unknown or at its rank cap, and a trainer for it is known: Goals.places kinds
-- "trainer_cooking" and "trainer_first_aid", fed by the realm (wp.services) or remembered from
-- NPCs seen with those titles through Goals.learn_places; the nearest that has not said it has
-- nothing for us).
-- Actions: learn_professions (walk to that trainer and train), craft (out of combat: open the
-- window, light a fire if needed, make the batch). Both make progress, so the runner takes them in
-- the downtime between fights. A planner goal that wants can_skill_up = false (data.profession
-- names one) levels a profession past the stock: craft then makes what still gives skill.
--
-- `Professions.log` keeps the last few things it did.

local P = {
  on = true, craft = true, learn = { "Cooking", "First Aid" },
  stock = { food = 20, bandage = 15 }, batch = 3, keep_cap = 40, recipe_share = 0.25,
  book = {}, reagents = {}, products = {}, notes = {}, log = {},
}

local SECONDARY = {
  Cooking = { kind = "trainer_cooking", makes = "food", subclass = "Meat", titles = { "cook" } },
  ["First Aid"] = { kind = "trainer_first_aid", makes = "bandage", subclass = "Cloth", titles = { "first aid", "physician" } },
}
P.SECONDARY = SECONDARY

local PROFESSIONS = {
  Alchemy = true, Blacksmithing = true, Enchanting = true, Engineering = true, Herbalism = true, Inscription = true,
  Jewelcrafting = true, Leatherworking = true, Mining = true, Skinning = true, Tailoring = true, Cooking = true,
  ["First Aid"] = true, Fishing = true,
}

-- 3.3.5's last rank cap: Grand Master.
local TOP_RANK = 450
local RANK_ORDER = { optimal = 1, medium = 2, easy = 3, trivial = 4 }

local function now() return GetTime() end
local function level() return UnitLevel("player") or 1 end

local function did(text)
  P.log[#P.log + 1] = text
  if #P.log > 6 then table.remove(P.log, 1) end
end

local function signal(name, ...)
  if Agent and Agent.signal then Agent.signal(name, ...) end
end

---------------------------------------------------------------------------------------------------
-- What we know

function P.known()
  local function read()
    local out, collapsed = {}, false
    for i = 1, (GetNumSkillLines and GetNumSkillLines() or 0) do
      local name, header, expanded, rank, _, _, max = GetSkillLineInfo(i)
      if name and header and not expanded then collapsed = true end
      if name and not header and PROFESSIONS[name] then out[name] = { rank = rank or 0, max = max or 0 } end
    end
    return out, collapsed
  end
  local out, collapsed = read()
  if collapsed and ExpandSkillHeader then
    ExpandSkillHeader(0)
    out = read()
  end
  return out
end

-- The spellbook's names, for Basic Campfire.
local function knows_spell(spell)
  if not GetSpellName then return false end
  for i = 1, 1000 do
    local name = GetSpellName(i, "spell")
    if not name then return false end
    if name == spell then return true end
  end
  return false
end

-- Everything in the bags: { bag, slot, name, link, count, class, subclass, minlevel, ilevel, spell }.
local function bags()
  local out = {}
  if not GetContainerNumSlots then return out end
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local link = GetContainerItemLink(bag, slot)
      if link then
        local name, _, _, ilevel, minlevel, class, subclass = GetItemInfo(link)
        local _, count = GetContainerItemInfo(bag, slot)
        out[#out + 1] = { bag = bag, slot = slot, link = link, name = name, count = count or 1, class = class,
          subclass = subclass, minlevel = minlevel or 0, ilevel = ilevel or 0,
          spell = GetItemSpell and GetItemSpell(link) }
      end
    end
  end
  return out
end

local function held(name, items)
  local n = 0
  for _, it in ipairs(items or bags()) do if it.name == name then n = n + it.count end end
  return n
end

-- What an item is for: "food" (its use is Food or Refreshment) or "bandage" (First Aid).
local function use_of(spell)
  if spell == "Food" or spell == "Refreshment" then return "food" end
  if spell == "First Aid" then return "bandage" end
end

function P.count(kind, items)
  local n = 0
  for _, it in ipairs(items or bags()) do if use_of(it.spell) == kind then n = n + it.count end end
  return n
end

---------------------------------------------------------------------------------------------------
-- The tradeskill window

-- Read the open window into P.book[name]: every recipe row (headers expanded first).
function P.scan(name)
  local line, rank, max = GetTradeSkillLine()
  if line ~= name then return nil end
  local function rows()
    local out, folded = {}, false
    for i = 1, (GetNumTradeSkills() or 0) do
      local recipe, kind, available, expanded = GetTradeSkillInfo(i)
      if recipe and kind == "header" and not expanded then folded = true end
      if recipe and kind ~= "header" then
        local r = { index = i, name = recipe, difficulty = kind, available = available or 0, reagents = {}, tools = {} }
        for j = 1, (GetTradeSkillNumReagents(i) or 0) do
          local reagent, _, need = GetTradeSkillReagentInfo(i, j)
          if reagent then
            r.reagents[#r.reagents + 1] = { name = reagent, need = need or 1 }
            P.reagents[reagent] = name
          end
        end
        local tools = { GetTradeSkillTools(i) }
        for k = 1, #tools, 2 do
          r.tools[#r.tools + 1] = tools[k]
          if tostring(tools[k]):lower():find("fire", 1, true) then r.fire = true end
        end
        local link = GetTradeSkillItemLink and GetTradeSkillItemLink(i)
        r.makes = use_of(link and GetItemSpell and GetItemSpell(link)) or SECONDARY[name] and SECONDARY[name].makes
        local product = link and GetItemInfo(link)
        if product then P.products[product] = name end
        out[#out + 1] = r
      end
    end
    return out, folded
  end
  local recipes, folded = rows()
  if folded and ExpandTradeSkillSubClass then
    ExpandTradeSkillSubClass(0)
    recipes = rows()
  end
  P.book[name] = { rank = rank or 0, max = max or 0, recipes = recipes, at = now() }
  return P.book[name]
end

-- How many of a recipe the bags can make, from its reagents.
local function can_make(r, items)
  local n
  for _, re in ipairs(r.reagents) do
    local k = math.floor(held(re.name, items) / math.max(re.need, 1))
    n = n and math.min(n, k) or k
  end
  return n or 0
end

-- The professions we may craft with: known, and with a book read (or reagents of their kind to
-- make it worth reading one).
local function crafts()
  local known, out = P.known(), {}
  for name, s in pairs(SECONDARY) do
    if known[name] then out[#out + 1] = { name = name, info = s, skill = known[name] } end
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

-- How many `kind` ("food" or "bandage") the bags can make now. An unread book counts the reagents
-- of its kind (Trade Goods meat or cloth) as one each, so the window is opened to see.
function P.makeable(kind, items)
  items = items or bags()
  local total = 0
  for _, c in ipairs(crafts()) do
    if c.info.makes == kind then
      local book = P.book[c.name]
      if book then
        for _, r in ipairs(book.recipes) do
          if r.makes == kind then total = total + can_make(r, items) end
        end
      else
        for _, it in ipairs(items) do
          if it.class == "Trade Goods" and it.subclass == c.info.subclass then total = total + it.count end
        end
      end
    end
  end
  return total
end

-- Whether some known recipe that still gives skill can be made (`only`: of that profession).
function P.skill_up(only, items)
  items = items or bags()
  for _, c in ipairs(crafts()) do
    local book = P.book[c.name]
    if book and (not only or only == c.name) then
      for _, r in ipairs(book.recipes) do
        if r.difficulty ~= "trivial" and can_make(r, items) > 0 then return c.name end
      end
    end
  end
end

-- A recipe to make for this job: one we have reagents for, the best skill-up first, the most
-- makeable next. `kind` nil is any that still gives skill.
local function pick(book, kind, items)
  local best, best_n
  for _, r in ipairs(book.recipes) do
    local n = math.min(r.available, can_make(r, items))
    local fits = kind and r.makes == kind or (not kind and r.difficulty ~= "trivial")
    if fits and n > 0 then
      local better = not best or RANK_ORDER[r.difficulty] < RANK_ORDER[best.difficulty]
        or (r.difficulty == best.difficulty and n > best_n)
      if better then best, best_n = r, n end
    end
  end
  return best, best_n
end

---------------------------------------------------------------------------------------------------
-- Using what we made

-- The food a rest should eat: what we cooked first, then the highest level we may use, else nil.
function P.best_food()
  local best
  local function rank(it) return (P.products[it.name or ""] and 1000 or 0) + it.minlevel end
  for _, it in ipairs(bags()) do
    if use_of(it.spell) == "food" and it.minlevel <= level() and (not IsUsableItem or IsUsableItem(it.link))
      and (not best or rank(it) > rank(best) or (rank(it) == rank(best) and it.ilevel > best.ilevel)) then
      best = it
    end
  end
  if best then return best.bag, best.slot, best.name end
end

local function debuffed(name)
  for i = 1, 40 do
    local n = UnitAura("player", i, "HARMFUL")
    if not n then return false end
    if n == name then return true end
  end
  return false
end

-- Bandage `unit` (ourselves by default) with the best bandage we can use. Returns true, or false
-- and why not. Callable in a fight (a warrior's break while the target flees or is snared).
function P.bandage(unit)
  unit = unit or "player"
  local best
  for _, it in ipairs(bags()) do
    if use_of(it.spell) == "bandage" and (not IsUsableItem or IsUsableItem(it.link))
      and (not best or it.ilevel > best.ilevel) then
      best = it
    end
  end
  if not best then return false, "no bandage" end
  if unit == "player" and debuffed("Recently Bandaged") then return false, "recently bandaged" end
  UseContainerItem(best.bag, best.slot, unit)
  did("bandaged with " .. tostring(best.name))
  return true
end

-- Whether an item is a reagent to keep, and how many. See the header.
function P.keep(item, remaining)
  local name, _, _, _, _, class, subclass = GetItemInfo(item)
  name = name or item
  local prof = P.reagents[name]
  if not prof then
    for p, s in pairs(SECONDARY) do
      if class == "Trade Goods" and subclass == s.subclass then prof = p end
    end
  end
  local wanted = false
  for _, p in ipairs(P.learn) do if p == prof then wanted = true end end
  if not prof or not (wanted or P.known()[prof]) then return false, 0 end
  local cap = P.keep_cap
  if remaining then return remaining < cap, cap end
  return held(name) <= cap, cap
end

---------------------------------------------------------------------------------------------------
-- Trainers

-- The profession a title names ("Cooking Trainer", "Cook", "Physician").
function P.title_of(title)
  title = tostring(title or ""):lower()
  for _, name in ipairs({ "First Aid", "Cooking" }) do
    for _, word in ipairs(SECONDARY[name].titles) do
      if title:find(word, 1, true) then return name end
    end
  end
end

-- Add a trainer seen (or talked to) as a Goals.places kind, through Goals.learn_places. One already
-- listed is left be: learning replans, and this runs every planning pass.
local function learn_trainer(kind, u)
  for _, p in ipairs(Goals.places[kind] or {}) do
    if (u.guid and p.guid == u.guid) or (not u.guid and p.name == u.name) then return end
  end
  local pos = AgentPosition and AgentPosition()
  Goals.learn_places({ { kind = kind, name = u.name, x = u.x, y = u.y, z = u.z, guid = u.guid,
    map = pos and pos.map, zone = GetRealZoneText and GetRealZoneText() } })
end

-- Remember the profession trainers in sight as Goals.places kinds.
function P.remember(list)
  if not (Goals and Goals.learn_places) then return end
  for _, u in ipairs(list) do
    local trains = false
    for _, s in ipairs(u.npc or {}) do if s == "professiontrainer" then trains = true end end
    local prof = trains and u.kind == "unit" and not u.dead and P.title_of(u.title)
    if prof then learn_trainer(SECONDARY[prof].kind, u) end
  end
end

-- Whether a profession wants a trainer: unknown, or at its rank cap below the last rank.
local function wants(name)
  local s = P.known()[name]
  return not s or (s.max > 0 and s.max < TOP_RANK and s.rank >= s.max), s
end

-- Whether what we learnt of this trainer says it has nothing for us now.
local function ruled_out(npc, s)
  local n = npc and P.notes[npc]
  if not (n and s) then return false end
  return n.cap == s.max or (n.level ~= nil and n.at_max == s.max and level() < n.level)
end

-- The nearest trainer of a kind on this continent that has not told us it has nothing for us.
local function trainer(kind, s)
  local pos, best, best_d = AgentPosition and AgentPosition(), nil, math.huge
  for _, p in ipairs(Goals.places[kind] or {}) do
    if not (pos and p.map and pos.map ~= p.map) and not ruled_out(p.name, s) then
      local d = pos and math.sqrt((pos.x - p.x) ^ 2 + (pos.y - p.y) ^ 2) or 0
      if d < best_d then best, best_d = p, d end
    end
  end
  return best
end

-- The trainer to visit: { profession, kind, place }, or nil.
function P.wanted()
  if not (P.on and Goals and Goals.places) then return nil end
  for _, name in ipairs(P.learn) do
    local want, s = wants(name)
    if want and SECONDARY[name] and not Goals.cooling("learn_professions:" .. name) then
      local kind = SECONDARY[name].kind
      local p = trainer(kind, s)
      if p then return { profession = name, kind = kind, place = p } end
    end
  end
end

local visit -- this trainer window: { profession, bought = {}, rounds, due }

-- What this trainer teaches of ours: the profession its services name.
local function trainer_profession()
  for i = 1, (GetNumTrainerServices() or 0) do
    local name = GetTrainerServiceInfo(i)
    if SECONDARY[name] then return name end
    local req = GetTrainerServiceSkillReq and GetTrainerServiceSkillReq(i)
    if SECONDARY[req] then return req end
  end
end

local function buy_round()
  local prof = visit.profession
  local money, learned = GetMoney() or 0, {}
  local s = P.known()[prof]
  local rank_seen = false
  -- The rank first (it unlocks the recipes), then recipes within the share of what is left.
  for pass = 1, 2 do
    local budget = pass == 1 and money or math.floor(money * P.recipe_share)
    for i = 1, (GetNumTrainerServices() or 0) do
      local name, sub, category = GetTrainerServiceInfo(i)
      local is_rank = name == prof
      if is_rank and category ~= "used" then rank_seen = true end
      local key = tostring(name) .. "|" .. tostring(sub)
      local cost = GetTrainerServiceCost(i) or 0
      if (pass == 1) == is_rank and category == "available" and not visit.bought[key] and cost <= budget then
        BuyTrainerService(i)
        visit.bought[key] = true
        money, budget = money - cost, budget - cost
        learned[#learned + 1] = name .. (sub and sub ~= "" and (" (" .. sub .. ")") or "")
      elseif is_rank and category == "unavailable" and s and GetTrainerServiceLevelReq then
        local req = GetTrainerServiceLevelReq(i) or 0
        if req > level() then P.notes[visit.npc] = { level = req, at_max = s.max } end
      end
    end
  end
  -- At the cap with no rank above it here: this trainer cannot take us further.
  if s and s.rank >= s.max and not rank_seen and #learned == 0 then P.notes[visit.npc] = { cap = s.max } end
  return learned
end

-- At a tradeskill trainer (called by Upkeep on TRAINER_SHOW and again after each purchase).
function P.train()
  if not P.on then return false end
  local prof = trainer_profession()
  local wanted = false
  for _, p in ipairs(P.learn) do if p == prof then wanted = true end end
  if not wanted then return false end
  local npc = UnitName("npc")
  if not visit or visit.npc ~= npc then visit = { profession = prof, npc = npc, bought = {}, rounds = 0, learned = {} } end
  -- This NPC teaches it: remember where it stands, whatever its title says.
  for _, u in ipairs(AgentNearby and AgentNearby(20) or {}) do
    if u.name == npc and u.kind == "unit" and Goals and Goals.learn_places then
      learn_trainer(SECONDARY[prof].kind, u)
    end
  end
  visit.rounds = visit.rounds + 1
  local learned = buy_round()
  for _, l in ipairs(learned) do visit.learned[#visit.learned + 1] = l end
  if #learned > 0 and visit.rounds < 4 then
    -- The list changes once the server confirms (TRAINER_UPDATE): look again then.
    visit.due = now() + 2
    return true
  end
  did(#visit.learned > 0 and ("learned " .. table.concat(visit.learned, ", ")) or ("nothing to learn at " .. tostring(npc)))
  P.trained_at, P.trained_learned, visit = now(), #visit.learned, nil
  if CloseTrainer then CloseTrainer() end
  return true
end

---------------------------------------------------------------------------------------------------
-- Events: the trainer's list after a purchase, our casts, refusals, food bought

local events = { made = 0 }
P.events = events
local merchant_food

local frame = CreateFrame("Frame")
for _, e in ipairs({ "TRAINER_UPDATE", "TRAINER_CLOSED", "UNIT_SPELLCAST_SUCCEEDED", "UI_ERROR_MESSAGE",
  "MERCHANT_SHOW", "MERCHANT_CLOSED" }) do
  frame:RegisterEvent(e)
end
frame:SetScript("OnEvent", function(_, event, a1, a2)
  if event == "TRAINER_UPDATE" and visit and visit.due then
    P.train()
  elseif event == "TRAINER_CLOSED" then
    if visit and visit.due then P.trained_at, P.trained_learned = now(), #visit.learned end
    visit = nil
  elseif event == "UNIT_SPELLCAST_SUCCEEDED" and a1 == "player" then
    if a2 == "Basic Campfire" then events.lit = now() end
    if events.recipe and a2 == events.recipe then events.made, events.last = events.made + 1, now() end
  elseif event == "UI_ERROR_MESSAGE" and type(a1) == "string" and a1:find("^Requires ") and a1:lower():find("fire", 1, true) then
    events.refused = now()
  elseif event == "MERCHANT_SHOW" then
    merchant_food = P.count("food")
  elseif event == "MERCHANT_CLOSED" and merchant_food then
    local bought = P.count("food") - merchant_food
    merchant_food = nil
    if bought > 0 then
      local d = P.digest()
      signal("FOOD_BOUGHT", bought, d.reagents, d.professions)
    end
  end
end)
local since = 0
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 0.5 then return end
  since = 0
  if visit and visit.due and now() >= visit.due then P.train() end
end)

-- A cooking fire within reach: a spell-focus object (type 8) close by, or the one we lit lately
-- (Basic Campfire burns a minute). Forges are focus objects too: the server's refusal corrects us.
function P.fire_near()
  if events.lit and now() - events.lit < 55 and not (events.refused and events.refused > events.lit) then return true end
  for _, u in ipairs(AgentNearby and AgentNearby(8) or {}) do
    if u.kind == "gameobject" and u.go_type == 8 then return not (events.refused and now() - events.refused < 10) end
  end
  return false
end

---------------------------------------------------------------------------------------------------
-- The goal runner's facts and actions

-- A planner goal that asks for skill-ups (want can_skill_up = false), and the profession it names.
local function leveling()
  for _, g in pairs(Goals.goals) do
    if g.want and g.want.can_skill_up == false and g.status == "active" then
      return true, g.data and g.data.profession
    end
  end
end

-- The job craft would do now: { profession, kind (nil for skill-ups), count }, or nil.
function P.job()
  if not (P.on and P.craft) then return nil end
  local items = bags()
  for _, c in ipairs(crafts()) do
    local kind = c.info.makes
    local short = P.stock[kind] - P.count(kind, items)
    local can = P.makeable(kind, items)
    if short > 0 and can >= math.min(P.batch, short) then
      return { profession = c.name, kind = kind, count = math.min(short, can) }
    end
  end
  local on, only = leveling()
  local prof = on and P.skill_up(only, items)
  if prof then return { profession = prof, count = 5 } end
end

if Goals then
  local G = Goals
  local function batch_of(kind)
    local items = bags()
    local short = P.stock[kind] - P.count(kind, items)
    return P.makeable(kind, items) >= math.max(1, math.min(P.batch, short))
  end
  G.fact("can_cook", function() return P.on and P.craft and P.known().Cooking ~= nil and batch_of("food") end)
  G.fact("can_bandage", function() return P.on and P.craft and P.known()["First Aid"] ~= nil and batch_of("bandage") end)
  G.fact("food_low", function() return P.count("food") < P.stock.food end)
  G.fact("bandages_low", function() return P.count("bandage") < P.stock.bandage end)
  G.fact("can_skill_up", function() return P.on and P.craft and P.skill_up() ~= nil end)
  G.fact("profession_to_learn", function()
    if G.nearby then P.remember(G.nearby()) end
    return P.wanted() ~= nil
  end)

  local learning = {}
  -- Both actions are built in like Goals' own: the zone guard is for actions the planner teaches.
  G.action({
    name = "learn_professions", built_in = true,
    why = "Learn Cooking and First Aid (or their next rank) from a trainer: food and bandages of our own.",
    pre = function(f) return f.profession_to_learn and not f.in_combat end,
    effects = { profession_to_learn = false, progressed = true },
    cost = function()
      local w, pos = P.wanted(), AgentPosition and AgentPosition()
      local p = w and w.place
      return 3 + ((p and pos) and math.min(math.sqrt((pos.x - p.x) ^ 2 + (pos.y - p.y) ^ 2) / 200, 4) or 0)
    end,
    stop = function()
      G.ask_gossip(nil)
      if Move then Move.free() end
    end,
    run = function(args)
      local w = P.wanted()
      assert(w, "no profession trainer known")
      args.subject, args.going = w.profession, w.place.name
      learning = { since = now(), place = w.place, kind = w.kind, profession = w.profession }
      -- Tomas and Michelle Belle greet with gossip first: ask for their training.
      G.ask_gossip("trainer")
    end,
    done = function()
      local over, ok, why
      if P.trained_at and P.trained_at >= learning.since then
        over, ok = true, (P.trained_learned or 0) > 0
        why = not ok and ("nothing to learn of " .. learning.profession .. " from " .. tostring(learning.place.name) .. " now") or nil
      else
        over, ok, why = G.visit(learning)
      end
      if over then G.ask_gossip(nil) end
      return over, ok, why
    end,
    timeout = 900,
  })

  local job = {}
  local function finish(ok, why)
    events.recipe = nil
    if CloseTradeSkill and GetTradeSkillLine() == job.profession then CloseTradeSkill() end
    return true, ok, why
  end

  G.action({
    name = "craft", built_in = true,
    why = "Cook meat into food and make bandages from cloth, up to a stock (skill-ups first).",
    pre = function(f)
      if f.in_combat or f.dead then return false end
      return (f.can_cook and f.food_low) or (f.can_bandage and f.bandages_low) or (f.can_skill_up and leveling()) or false
    end,
    effects = { food_low = false, bandages_low = false, can_skill_up = false, progressed = true },
    cost = 3,
    run = function(args)
      local j = P.job()
      assert(j, "nothing to craft")
      if args.data and args.data.profession and P.known()[args.data.profession] then j.profession = args.data.profession end
      args.subject = j.profession
      job = { profession = j.profession, kind = j.kind, count = j.count, since = now(), step = "open", fires = 0 }
      events.made, events.recipe, events.refused = 0, nil, nil
    end,
    done = function()
      if UnitAffectingCombat("player") then return finish(false, "attacked while crafting") end
      local t = now()
      if job.step == "open" then
        if GetTradeSkillLine() ~= job.profession then
          if not job.opened then
            job.opened = t
            CastSpellByName(job.profession)
          elseif t - job.opened > 5 then
            return finish(false, "the " .. job.profession .. " window did not open")
          end
          return false
        end
        local book = P.scan(job.profession)
        local r, n = pick(book, job.kind, bags())
        if not r then return finish(job.made ~= nil, "nothing to make with " .. job.profession .. " from the bags") end
        job.recipe, job.n = r, math.min(n, job.count)
        if r.fire and not P.fire_near() then job.step = "fire" else job.step = "make" end
      end
      if job.step == "fire" then
        if job.fires >= 2 then return finish(false, "no cooking fire: lit " .. job.fires .. " and still refused") end
        if not knows_spell("Basic Campfire") then return finish(false, job.recipe.name .. " needs a cooking fire and Basic Campfire is not known") end
        job.fires, job.lighting, events.refused = job.fires + 1, t, nil
        CastSpellByName("Basic Campfire")
        job.step = "lighting"
        return false
      end
      if job.step == "lighting" then
        if events.lit and events.lit >= job.lighting then
          job.step = "open"
        elseif t - job.lighting > 15 then
          return finish(false, "Basic Campfire did not light")
        end
        return false
      end
      if job.step == "make" then
        events.recipe, events.made, events.last, events.refused = job.recipe.name, 0, t, nil
        job.made_at = t
        DoTradeSkill(job.recipe.index, job.n)
        job.step = "making"
        return false
      end
      -- Making: count our own casts of the recipe; a fire refusal lights one and starts again.
      if events.refused and events.refused >= job.made_at then
        job.step = "fire"
        return false
      end
      local made = events.made
      if made >= job.n or (t - (events.last or job.made_at) > 8 and not (UnitCastingInfo and UnitCastingInfo("player"))) then
        local s = P.known()[job.profession]
        did(string.format("made %d %s (%s %s/%s)", made, job.recipe.name, job.profession, s and s.rank or "?", s and s.max or "?"))
        job.made = (job.made or 0) + made
        if made == 0 then return finish(false, "made no " .. job.recipe.name) end
        return finish(true)
      end
      return false
    end,
    stop = function() events.recipe = nil if CloseTradeSkill then CloseTradeSkill() end end,
    timeout = 240,
  })
end

-- For the status line and the reflection: "Cooking 12/75, First Aid 1/75; food 6, bandages 0".
function P.summary()
  local known, parts = P.known(), {}
  for _, name in ipairs({ "Cooking", "First Aid" }) do
    local s = known[name]
    parts[#parts + 1] = name .. (s and (" " .. s.rank .. "/" .. s.max) or " not learnt")
  end
  return table.concat(parts, ", ") .. string.format("; food %d, bandages %d", P.count("food"), P.count("bandage"))
end

-- For the harness: secondary professions known, reagents held for them, food held.
function P.digest()
  local items, reagents, n = bags(), 0, 0
  for name in pairs(SECONDARY) do if P.known()[name] then n = n + 1 end end
  for _, it in ipairs(items) do
    for _, s in pairs(SECONDARY) do
      if (it.class == "Trade Goods" and it.subclass == s.subclass) or P.reagents[it.name or ""] then reagents = reagents + it.count break end
    end
  end
  return { professions = n, reagents = reagents, food = P.count("food", items) }
end

return P
