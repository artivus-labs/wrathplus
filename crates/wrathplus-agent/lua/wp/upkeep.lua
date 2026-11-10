-- wp.upkeep: the chores a player does without thinking, so the tiers above need not. Loaded for
-- every agent as the global `Upkeep`; set `on`, `equip`, `sell`, `buy` or `train` false to do one
-- yourself.
--
--   Equip:  out of combat, wear what is in the bags when the slot is empty, or when it is a clear
--           upgrade (a higher item level, no lower quality) of the same armour type. A weapon or
--           shield replaces one of its own kind (a better two-handed sword for a two-handed
--           sword: the same hands, a skill we have); a change of kind (two-hander or sword and
--           board) is the agent's call, and a shield never goes on beside a two-hander.
--   Bags:   out of combat, put a bag found in the bags into an empty bag slot (the largest first),
--           or in place of the smallest bag worn when it is larger. At a merchant whose largest
--           bag would fill an empty slot or beat the smallest worn, buy it when the money left
--           still covers the reserve (training and repairs, `reserve`).
--   Sell:   at a merchant, sell grey (junk) items, and repair if the money covers it. When the
--           junk alone leaves the bags under `full_below` free slots, sell the white items
--           `sell_when_full` gives up too (trade goods, simple gems, gear that is no upgrade),
--           never one named in `keep` (Goals.keep adds names), a quest item or the Hearthstone.
--   Buy:    at a merchant that sells it, food (and drink for a mana user) of the best level we
--           can use, up to `food_stock` in the bags, for at most `food_share` of our money: what
--           Goals' rest eats between fights.
--   Train:  at a class trainer, learn every available rank the money covers, then close the
--           window. Upkeep.trainer() is the nearest trainer of your own class ("Warrior
--           Trainer"); after a level-up the fast tier is offered train_skills when one is in sight.
--           At a Cooking or First Aid trainer, Professions.train() learns instead.
--   Loot:   in a group roll, `loot` = "upgrades" (the default) needs on what it would wear as an
--           upgrade and greeds on the rest; "greed" always greeds; "pass" passes; false leaves the
--           roll to the agent (START_LOOT_ROLL, RollOnLoot). A bind-on-pickup roll it made is
--           confirmed.
--
-- `Upkeep.log` keeps the last few things it did ("equipped Outfitter Belt", ...).

local U = { on = true, equip = true, bags = true, sell = true, buy = true, train = true, loot = "upgrades", log = {},
  food_stock = 20, food_share = 0.34,
  -- Fewer free bag slots than this is full (Goals' bags_full).
  full_below = 2,
  -- Never sold, by name.
  keep = { Hearthstone = true },
  -- What a full bag gives up at a merchant once the junk is gone, by the item's type (GetItemInfo's
  -- class): "sell", "keep", or "unless_upgrade" (sold unless it would be worn as an upgrade). Green
  -- and better is never sold; a type not listed is kept.
  sell_when_full = {
    ["Trade Goods"] = "sell", Gem = "sell", Armor = "unless_upgrade", Weapon = "unless_upgrade",
    Consumable = "keep", Container = "keep", Quest = "keep", Key = "keep", Reagent = "keep",
    Recipe = "keep", Projectile = "keep", Quiver = "keep", Glyph = "keep", Miscellaneous = "keep",
  },
  -- Money a bag purchase leaves: copper per level squared, and twice the next training's cost.
  reserve_per_level = 10, next_training_cost = 0,
}

local function did(text)
  U.log[#U.log + 1] = text
  if #U.log > 5 then table.remove(U.log, 1) end
end

-- Inventory slots per equip location (rings and trinkets take the first free of two).
local SLOTS = {
  INVTYPE_HEAD = { 1 }, INVTYPE_NECK = { 2 }, INVTYPE_SHOULDER = { 3 }, INVTYPE_BODY = { 4 },
  INVTYPE_CHEST = { 5 }, INVTYPE_ROBE = { 5 }, INVTYPE_WAIST = { 6 }, INVTYPE_LEGS = { 7 },
  INVTYPE_FEET = { 8 }, INVTYPE_WRIST = { 9 }, INVTYPE_HAND = { 10 }, INVTYPE_FINGER = { 11, 12 },
  INVTYPE_TRINKET = { 13, 14 }, INVTYPE_CLOAK = { 15 }, INVTYPE_WEAPON = { 16 },
  INVTYPE_2HWEAPON = { 16 }, INVTYPE_WEAPONMAINHAND = { 16 }, INVTYPE_SHIELD = { 17 },
  INVTYPE_WEAPONOFFHAND = { 17 }, INVTYPE_HOLDABLE = { 17 }, INVTYPE_RANGED = { 18 },
  INVTYPE_RANGEDRIGHT = { 18 }, INVTYPE_THROWN = { 18 }, INVTYPE_TABARD = { 19 },
}
local WEAPON_SLOTS = { [16] = true, [17] = true, [18] = true }

-- The heaviest armour a class wears, and the level it starts (3.3.5: mail and plate at 40).
local ARMOUR = { Cloth = 1, Leather = 2, Mail = 3, Plate = 4 }
local WEARS = {
  WARRIOR = { 3, 4 }, PALADIN = { 3, 4 }, DEATHKNIGHT = { 4, 4 }, HUNTER = { 2, 3 },
  SHAMAN = { 2, 3 }, ROGUE = { 2, 2 }, DRUID = { 2, 2 }, PRIEST = { 1, 1 }, MAGE = { 1, 1 },
  WARLOCK = { 1, 1 },
}

local function heaviest()
  local _, class = UnitClass("player")
  local w = WEARS[class or ""] or { 1, 1 }
  return (UnitLevel("player") or 1) >= 40 and w[2] or w[1]
end

-- An item's facts from its link: name, quality, item level, required level, class, subclass, slot.
local function info(link)
  if not link then return nil end
  local name, _, quality, ilevel, minlevel, class, subclass, _, equip, _, price = GetItemInfo(link)
  if not name then return nil end
  return { name = name, link = link, quality = quality or 0, ilevel = ilevel or 0, minlevel = minlevel or 0,
    class = class, subclass = subclass, equip = equip, price = price or 0 }
end

local function wearable(item)
  if not item.equip or not SLOTS[item.equip] then return false end
  if item.minlevel > (UnitLevel("player") or 1) then return false end
  if item.class == "Armor" and ARMOUR[item.subclass] and ARMOUR[item.subclass] > heaviest() then return false end
  return IsEquippableItem == nil or IsEquippableItem(item.link)
end

local function better(item, worn)
  if not worn then return true end
  if item.subclass ~= worn.subclass then return false end
  return item.ilevel > worn.ilevel and item.quality >= worn.quality
end

-- How often each item has been put on: one the game keeps refusing (a bow without the skill) is
-- left in the bags after two tries, not picked up for ever.
local tried = {}

-- The best piece in the bags for each slot it could fill, and one of them worn per pass
-- (equipping changes the bags).
local function equip_pass()
  local main = info(GetInventoryItemLink("player", 16))
  local best = {}
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local item = info(GetContainerItemLink(bag, slot))
      if item and wearable(item) and (tried[item.link] or 0) < 2 then
        for _, inv in ipairs(SLOTS[item.equip]) do
          local worn = info(GetInventoryItemLink("player", inv))
          -- A weapon only replaces one of its own kind (better() asks the same subclass too).
          local blocked = (WEAPON_SLOTS[inv] and worn and worn.equip ~= item.equip)
            or (inv == 17 and main and main.equip == "INVTYPE_2HWEAPON")
          local rival = best[inv]
          if not blocked and better(item, worn)
            and (not rival or item.ilevel > rival.item.ilevel
              or (item.ilevel == rival.item.ilevel and item.quality > rival.item.quality)) then
            best[inv] = { item = item, bag = bag, slot = slot }
          end
        end
      end
    end
  end
  for inv = 1, 19 do
    local pick = best[inv]
    if pick then
      tried[pick.item.link] = (tried[pick.item.link] or 0) + 1
      PickupContainerItem(pick.bag, pick.slot)
      EquipCursorItem(inv)
      did("equipped " .. pick.item.name)
      return true
    end
  end
  return false
end

-- Would this item be worn as an upgrade in some slot?
local function upgrade(item)
  if not item or not wearable(item) then return false end
  for _, inv in ipairs(SLOTS[item.equip]) do
    if better(item, info(GetInventoryItemLink("player", inv))) then return true end
  end
  return false
end

---------------------------------------------------------------------------------------------------
-- Bags: inventory slots 20-23 hold bags 1-4; the backpack (bag 0) is fixed.

local BAG_SLOTS = { 20, 21, 22, 23 }
local HEARTHSTONE = 6948

-- The tooltip's bag line ("6 Slot Bag", CONTAINER_SLOTS) as a pattern capturing the count.
local function slots_pattern()
  if type(CONTAINER_SLOTS) ~= "string" then return nil end
  local p = CONTAINER_SLOTS:gsub("([%(%)%.%+%-%*%?%[%]%^%$])", "%%%1")
  p = p:gsub("%%d", "(%%d+)")
  p = p:gsub("%%s", ".+")
  return "^" .. p .. "$"
end

local scan
-- How many slots a bag item holds, read off its tooltip as a player reads it (3.3.5 has no other
-- way for a bag not worn): `where` is "bag" (bag, slot) or "merchant" (index). nil when unknown.
function U.bag_slots(where, a, b)
  local pattern = slots_pattern()
  if not pattern or not CreateFrame then return nil end
  local ok, n = pcall(function()
    scan = scan or CreateFrame("GameTooltip", "WPUpkeepScan", nil, "GameTooltipTemplate")
    scan:SetOwner(WorldFrame or UIParent, "ANCHOR_NONE")
    scan:ClearLines()
    if where == "bag" then scan:SetBagItem(a, b) else scan:SetMerchantItem(a) end
    for i = 1, (scan:NumLines() or 0) do
      local line = _G["WPUpkeepScanTextLeft" .. i]
      local count = line and line:GetText() and line:GetText():match(pattern)
      if count then return tonumber(count) end
    end
  end)
  if scan then pcall(scan.Hide, scan) end
  return ok and n or nil
end

-- A bag for anything: not a soul, herb or other specialty bag (GetItemFamily 0).
local function general_bag(item)
  if not item or item.equip ~= "INVTYPE_BAG" then return false end
  return not (GetItemFamily and (GetItemFamily(item.link) or 0) ~= 0)
end

-- The bag slots: { inv, bag, size, name } or { inv, bag, empty = true }.
local function worn_bags()
  local out = {}
  for i, inv in ipairs(BAG_SLOTS) do
    local item = info(GetInventoryItemLink("player", inv))
    out[i] = item and { inv = inv, bag = i, size = GetContainerNumSlots(i) or 0, name = item.name }
      or { inv = inv, bag = i, empty = true }
  end
  return out
end

-- General bags lying in the bags, largest first: { bag, slot, item, size } (size 0: unknown).
local function spare_bags()
  local out = {}
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local item = info(GetContainerItemLink(bag, slot))
      if general_bag(item) then
        out[#out + 1] = { bag = bag, slot = slot, item = item, size = U.bag_slots("bag", bag, slot) or 0 }
      end
    end
  end
  table.sort(out, function(a, b) return a.size > b.size end)
  return out
end

local function sized(name, size)
  return name .. (size > 0 and (" (" .. size .. " slots)") or "")
end

-- Tries per bag and slot, so a swap the server refuses is not tried every second.
local bag_tries = {}

-- One bag put on per pass: into an empty slot, else in place of the smallest worn bag when larger.
local function bag_pass()
  local spares = spare_bags()
  if #spares == 0 then return false end
  local target, smallest
  for _, w in ipairs(worn_bags()) do
    if w.empty then
      target = target or w
    elseif not smallest or w.size < smallest.size then
      smallest = w
    end
  end
  for _, s in ipairs(spares) do
    local into = target
      or (smallest and s.size > smallest.size and s.bag ~= smallest.bag and smallest) or nil
    local key = s.item.name .. ">" .. (into and into.inv or 0)
    if into and (bag_tries[key] or 0) < 2 then
      bag_tries[key] = (bag_tries[key] or 0) + 1
      PickupContainerItem(s.bag, s.slot)
      EquipCursorItem(into.inv)
      did("equipped " .. sized(s.item.name, s.size)
        .. (into.empty and "" or (" in place of " .. sized(into.name, into.size))))
      return true
    end
  end
  return false
end

-- Money a purchase must leave: training to come and today's repairs.
function U.reserve()
  local lvl = UnitLevel("player") or 1
  local repair = GetRepairAllCost and GetRepairAllCost() or 0
  return math.max(lvl * lvl * U.reserve_per_level, 2 * (U.next_training_cost or 0)) + (repair or 0)
end

-- At a merchant: the largest bag it sells, bought when it would fill an empty slot no spare bag
-- fills, or beat the smallest bag worn, and the money left covers the reserve.
local function buy_bags(money)
  if not (U.bags and GetMerchantNumItems and BuyMerchantItem) then return end
  local best
  for i = 1, (GetMerchantNumItems() or 0) do
    local item = info(GetMerchantItemLink and GetMerchantItemLink(i))
    if general_bag(item) then
      local _, _, price, _, available = GetMerchantItemInfo(i)
      local size = U.bag_slots("merchant", i) or 0
      if size > 0 and available ~= 0 and price
        and (not best or size > best.size or (size == best.size and price < best.price)) then
        best = { index = i, item = item, size = size, price = price }
      end
    end
  end
  if not best then return end
  local spares, empty, smallest = spare_bags(), 0, nil
  for _, w in ipairs(worn_bags()) do
    if w.empty then
      empty = empty + 1
    elseif not smallest or w.size < smallest.size then
      smallest = w
    end
  end
  local wanted = empty - #spares
  if wanted <= 0 then
    local spare = spares[1] and spares[1].size or 0
    if empty > 0 or not smallest or best.size <= smallest.size or spare >= best.size then return end
    wanted = 1
  end
  local keep, bought = U.reserve(), 0
  while bought < wanted and money - best.price >= keep do
    BuyMerchantItem(best.index, 1)
    money, bought = money - best.price, bought + 1
  end
  if bought > 0 then
    did(string.format("bought %s%s for %dc", bought > 1 and (bought .. " x ") or "", sized(best.item.name, best.size),
      best.price * bought))
  else
    did(string.format("did not buy %s for %dc: keeping %dc for training and repairs", sized(best.item.name, best.size),
      best.price, keep))
  end
end

---------------------------------------------------------------------------------------------------
-- Selling

local function item_id(bag, slot)
  return GetContainerItemID and GetContainerItemID(bag, slot)
end

-- What a full bag does with the item in this slot: "junk" (sold at every visit), "sell" (sold when
-- the bags are full), or "keep" and why.
local function verdict(bag, slot, item)
  if item_id(bag, slot) == HEARTHSTONE then return "keep", "hearthstone" end
  if U.keep[item.name] then return "keep", "kept" end
  if GetContainerItemQuestInfo then
    local quest, id = GetContainerItemQuestInfo(bag, slot)
    if quest or (id and id > 0) then return "keep", "quest" end
  end
  if item.price <= 0 then return "keep", "no sale value" end
  if item.quality == 0 then return "junk" end
  if item.quality > 1 then return "keep", "green or better" end
  local rule = U.sell_when_full[item.class or ""] or "keep"
  if rule == "sell" then return "sell" end
  if rule == "unless_upgrade" then
    if upgrade(item) then return "keep", "an upgrade" end
    return "sell"
  end
  return "keep", string.lower(item.class or "kept")
end

-- Every item in the bags with its verdict, and the empty slots.
local function survey()
  local items, empty = {}, 0
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local item = info(GetContainerItemLink(bag, slot))
      if item then
        local v, why = verdict(bag, slot, item)
        items[#items + 1] = { bag = bag, slot = slot, item = item, verdict = v, why = why }
      elseif not GetContainerItemLink(bag, slot) then
        empty = empty + 1
      end
    end
  end
  return items, empty
end

-- Whether a merchant would take anything from full bags.
function U.sellable()
  for _, e in ipairs(survey()) do
    if e.verdict ~= "keep" then return true end
  end
  return false
end

-- What fills the bags that a merchant visit keeps: "Okra (quest), Scroll of Agility x2 (consumable)".
function U.filling(most)
  local by, order = {}, {}
  for _, e in ipairs(survey()) do
    if e.verdict == "keep" then
      local k = e.item.name
      if not by[k] then
        by[k] = { name = k, n = 0, why = e.why }
        order[#order + 1] = by[k]
      end
      by[k].n = by[k].n + 1
    end
  end
  table.sort(order, function(a, b)
    if a.n ~= b.n then return a.n > b.n end
    return a.name < b.name
  end)
  local out = {}
  for i = 1, math.min(#order, most or 8) do
    local e = order[i]
    out[i] = e.name .. (e.n > 1 and (" x" .. e.n) or "") .. " (" .. e.why .. ")"
  end
  if #order > (most or 8) then out[#out + 1] = (#order - (most or 8)) .. " more" end
  return table.concat(out, ", ")
end

local function sell(list)
  local earned, names, seen = 0, {}, {}
  for _, e in ipairs(list) do
    local _, count = GetContainerItemInfo(e.bag, e.slot)
    UseContainerItem(e.bag, e.slot)
    earned = earned + e.item.price * (count or 1)
    if not seen[e.item.name] then
      seen[e.item.name] = true
      names[#names + 1] = e.item.name
    end
  end
  return earned, names
end

local function sell_and_repair()
  local items, empty = survey()
  local junk, whites = {}, {}
  for _, e in ipairs(items) do
    if e.verdict == "junk" then junk[#junk + 1] = e elseif e.verdict == "sell" then whites[#whites + 1] = e end
  end
  local earned = sell(junk)
  if #junk > 0 then did(string.format("sold %d junk for %dc", #junk, earned)) end
  -- The sales land a moment later: count the slots they free.
  if empty + #junk < U.full_below and #whites > 0 then
    local more, names = sell(whites)
    earned = earned + more
    did(string.format("bags full: sold %d more for %dc (%s)", #whites, more,
      table.concat(names, ", ", 1, math.min(#names, 5)) .. (#names > 5 and ", ..." or "")))
  end
  local money, spent = (GetMoney() or 0) + earned, 0
  if CanMerchantRepair and CanMerchantRepair() then
    local cost = GetRepairAllCost()
    if cost and cost > 0 and cost <= money then
      RepairAllItems()
      money = money - cost
      spent = cost
      did(string.format("repaired for %dc", cost))
    end
  end
  return spent, money
end

-- How many of the bags' items are used as `spell` ("Food", "Drink").
local function stock(spell)
  local n = 0
  for bag = 0, 4 do
    for slot = 1, (GetContainerNumSlots(bag) or 0) do
      local link = GetContainerItemLink(bag, slot)
      if link and GetItemSpell and GetItemSpell(link) == spell then
        local _, count = GetContainerItemInfo(bag, slot)
        n = n + (count or 1)
      end
    end
  end
  return n
end

-- Buy what the merchant has of `spell` ("Food", "Drink"): the highest level we can use, a stack
-- at a time, to `food_stock` in the bags and `budget` copper. Returns what is left of the budget.
local function buy(spell, budget)
  if not (GetMerchantNumItems and GetMerchantItemLink and GetItemSpell and BuyMerchantItem) then return budget end
  local have = stock(spell)
  if have >= U.food_stock / 2 then return budget end
  local pick
  for i = 1, (GetMerchantNumItems() or 0) do
    local link = GetMerchantItemLink(i)
    local item = link and GetItemSpell(link) == spell and info(link)
    if item and item.minlevel <= (UnitLevel("player") or 1) then
      local _, _, price, quantity, available = GetMerchantItemInfo(i)
      price, quantity = price or 0, math.max(quantity or 1, 1)
      if price > 0 and price <= budget and available ~= 0 and (not pick or item.minlevel > pick.item.minlevel) then
        pick = { index = i, item = item, price = price, quantity = quantity }
      end
    end
  end
  if not pick then return budget end
  local bought = 0
  while have + bought < U.food_stock and pick.price <= budget do
    BuyMerchantItem(pick.index, 1)
    bought, budget = bought + pick.quantity, budget - pick.price
  end
  if bought > 0 then did(string.format("bought %d %s", bought, pick.item.name)) end
  return budget
end

-- Returns what the food cost.
local function buy_food(spent)
  local start = math.floor(((GetMoney() or 0) - (spent or 0)) * U.food_share)
  local budget = buy("Food", start)
  if UnitPowerType and UnitPowerType("player") == 0 then budget = buy("Drink", budget) end
  return start - budget
end

-- The nearest trainer of our own class: a class trainer whose title names it ("Warrior Trainer").
function U.trainer(yards)
  local class = UnitClass("player")
  if not class then return nil end
  local best
  for _, u in ipairs(AgentNearby(yards or 150)) do
    local trains = false
    for _, s in ipairs(u.npc or {}) do if s == "classtrainer" then trains = true end end
    if trains and u.title and u.title:lower():find(class:lower(), 1, true) and (not best or u.distance < best.distance) then
      best = u
    end
  end
  return best
end

-- The level we last trained at; nil until a visit this session.
U.trained_level = nil

local function train()
  -- A tradeskill trainer: Cooking and First Aid are Professions' to learn; the rest is the agent's.
  if IsTradeskillTrainer and IsTradeskillTrainer() then
    if Professions and Professions.train then Professions.train() end
    return
  end
  -- Not ours (a priest trainer for a warrior): nothing to learn here.
  if (GetNumTrainerServices() or 0) == 0 then return end
  local learned, money = {}, GetMoney() or 0
  local short, next_level, next_cost = 0, nil, nil -- what is too dear now, and when something new comes
  for i = 1, (GetNumTrainerServices() or 0) do
    local name, rank, category = GetTrainerServiceInfo(i)
    local cost = GetTrainerServiceCost(i) or 0
    if category == "available" and cost <= money then
      BuyTrainerService(i)
      money = money - cost
      learned[#learned + 1] = name .. (rank and rank ~= "" and (" (" .. rank .. ")") or "")
    elseif category == "available" then
      short = short + cost
    elseif category == "unavailable" and GetTrainerServiceLevelReq then
      local level = GetTrainerServiceLevelReq(i) or 0
      if level > (UnitLevel("player") or 0) then
        if not next_level or level < next_level then next_level, next_cost = level, 0 end
        if level == next_level then next_cost = next_cost + cost end
      end
    end
  end
  -- What is still to pay for: the reserve bag purchases leave (`reserve`).
  U.next_training_cost = short + (next_cost or 0)
  local who = UnitName("npc") or "the trainer"
  if #learned > 0 then
    did("trained " .. table.concat(learned, ", "))
  elseif short > 0 then
    did(string.format("nothing affordable at %s: %dc of skills, %dc in hand", who, short, money))
  else
    did("nothing new at " .. who .. (next_level and (" until level " .. next_level) or ""))
  end
  U.trained_level = UnitLevel("player")
  -- Done here: close the window, or follow mode waits on it.
  if CloseTrainer then CloseTrainer() end
end

-- For others (quest rewards): whether the item behind a link would be worn as an upgrade.
function U.upgrade(link)
  return upgrade(info(link))
end

-- 3.3.5 roll types: 0 pass, 1 need, 2 greed.
local ROLL_NAMES = { [0] = "passed on", [1] = "needed", [2] = "greeded" }
local rolled = {}

local function roll(id)
  local _, name, _, _, _, can_need, can_greed = GetLootRollItemInfo(id)
  if not name then return end
  local choice = 0
  if U.loot == "upgrades" and can_need and upgrade(info(GetLootRollItemLink(id))) then
    choice = 1
  elseif (U.loot == "upgrades" or U.loot == "greed") and can_greed then
    choice = 2
  end
  rolled[id] = choice
  RollOnLoot(id, choice)
  did(ROLL_NAMES[choice] .. " " .. name .. (choice == 1 and " (an upgrade)" or ""))
end

local frame = CreateFrame("Frame")
local since, dirty = 0, true
for _, event in ipairs({ "BAG_UPDATE", "PLAYER_LEVEL_UP", "MERCHANT_SHOW", "TRAINER_SHOW", "PLAYER_REGEN_ENABLED",
  "START_LOOT_ROLL", "CONFIRM_LOOT_ROLL" }) do
  frame:RegisterEvent(event)
end
frame:SetScript("OnEvent", function(_, event, a1, a2)
  if not U.on then return end
  if event == "START_LOOT_ROLL" and U.loot then
    roll(a1)
  elseif event == "CONFIRM_LOOT_ROLL" and rolled[a1] == a2 then
    -- Bind on pickup: confirm only a roll this module made.
    ConfirmLootRoll(a1, a2)
  elseif event == "MERCHANT_SHOW" then
    -- Repairs come out of the purse before food does, and food before a bag.
    local spent, money = 0, GetMoney() or 0
    if U.sell then spent, money = sell_and_repair() end
    if U.buy then money = money - buy_food(spent) end
    if U.bags then buy_bags(money) end
  elseif event == "TRAINER_SHOW" and U.train then
    train()
  else
    dirty = true
  end
end)
frame:SetScript("OnUpdate", function(_, dt)
  since = since + dt
  if since < 1 then return end
  since = 0
  if not (U.on and dirty) then return end
  if UnitAffectingCombat("player") or UnitIsDeadOrGhost("player") then return end
  if AgentPosition and not AgentPosition() then return end
  -- Bags first, then gear, until nothing more fits; the bags settle between pieces.
  dirty = (U.bags and bag_pass()) or (U.equip and equip_pass()) or false
end)

-- After a level-up, with our trainer in sight: go and train (the fast tier chooses it).
if Agent and Agent.intent then
  local talking_since
  Agent.intent("train_skills", {
    when = function()
      local t = U.trainer(60)
      return "You have levelled since you last trained: learn your new skills from "
        .. (t and t.name or "your trainer") .. " (" .. (t and t.title or "") .. ")."
    end,
    -- Not in a fight: the walk to the trainer would only fail.
    available = function()
      return U.train and not UnitAffectingCombat("player") and (UnitLevel("player") or 1) > (U.trained_level or 1)
        and U.trainer(60) ~= nil
    end,
    run = function()
      talking_since = GetTime()
      local t = U.trainer(60)
      if t then AgentInteract(t.guid) end
    end,
    -- Over once trained at this level, or after a minute.
    done = function()
      if U.trained_level == UnitLevel("player") then return true, true end
      if talking_since and GetTime() - talking_since > 60 then return true, false, "could not reach the trainer" end
      return false
    end,
  })
end

return U
