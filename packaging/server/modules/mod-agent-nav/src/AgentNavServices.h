// mod-agent-nav: where services are. The realm tells an agent where the vendors, repairers, its
// class trainer, flight masters, innkeepers, bankers, auctioneers, mailboxes and quest hubs on its
// map are, as a player knows a town and as Playerbots knows the world: from the creature and
// gameobject spawns the world database holds, read once at startup (mod_agent_nav.cpp), never per
// request. Standard library only, so the WrathPlus client's tests compile this file and check the
// selection against a fixture.
//
//   S <id> <kinds> [radius]        kinds: a comma list of vendor, repair, trainer (the asker's
//                                  class trainer), flightmaster, innkeeper, banker,
//                                  questgiver-hub, trainer_cooking, trainer_first_aid,
//                                  trainer_profession (any profession trainer), auctioneer,
//                                  mailbox; radius in yards (default and most:
//                                  AgentNav.ServiceRadius); the nearest two of the asker's
//                                  class trainers, Cooking and First Aid trainers (those asked
//                                  for) are told however far, on its map (SelectServices)
//   s <id> <i> <n> <kind> <entry> <x> <y> <z> <members> <profession> <event> <house> <name>
//                                  the i-th of n places, nearest first; entry: the creature's
//                                  template entry, a mailbox's gameobject entry; members: the quest
//                                  givers in a hub (1 for anything else); profession: the skill a
//                                  profession trainer teaches, spaces as underscores ("First_Aid"),
//                                  "-" for anything else; event: "-" for a standing spawn, "on" or
//                                  "off" for one a game event spawns (game_event_creature,
//                                  game_event_gameobject), as its events stand when the question
//                                  was asked; house: the auction house an auctioneer serves,
//                                  "alliance", "horde" or "neutral" (Booty Bay, Gadgetzan,
//                                  Everlook; every one when the realm lets both sides trade), "-"
//                                  for anything else; the name is the rest of the line
//   s <id> 0 0                     none
#ifndef MOD_AGENT_NAV_SERVICES_H
#define MOD_AGENT_NAV_SERVICES_H

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

namespace AgentNav
{
// The NPC flags each kind is read from (core: UnitDefines.h NPCFlags; mod_agent_nav.cpp asserts
// them).
constexpr std::uint32_t kFlagQuestGiver = 0x00000002;
constexpr std::uint32_t kFlagTrainer = 0x00000010;
constexpr std::uint32_t kFlagVendors = 0x00000080 | 0x00000100 | 0x00000200 | 0x00000400 | 0x00000800;
constexpr std::uint32_t kFlagRepair = 0x00001000;
constexpr std::uint32_t kFlagFlightMaster = 0x00002000;
constexpr std::uint32_t kFlagInnkeeper = 0x00010000;
constexpr std::uint32_t kFlagBanker = 0x00020000;
constexpr std::uint32_t kFlagAuctioneer = 0x00200000;
constexpr std::uint32_t kServiceFlags = kFlagQuestGiver | kFlagTrainer | kFlagVendors | kFlagRepair |
    kFlagFlightMaster | kFlagInnkeeper | kFlagBanker | kFlagAuctioneer;

// A spawn a player can use: one that cannot be selected (UNIT_FLAG_NOT_SELECTABLE) or is an
// invisible trigger (CREATURE_FLAG_EXTRA_TRIGGER) offers nothing, whatever its NPC flags say. The
// beta's world has 752 "[DND]" spawns with service flags (holiday bunnies, the TAR pedestals in
// Goldshire), all unselectable, plus 67 more unselectable or trigger services; before this they
// filled the answer's 20 places ahead of the real vendors. (Core values; mod_agent_nav.cpp
// asserts them.)
constexpr std::uint32_t kUnitFlagNotSelectable = 0x02000000;
constexpr std::uint32_t kFlagExtraTrigger = 0x00000080;
constexpr bool Usable(std::uint32_t unitFlags, std::uint32_t flagsExtra)
{
    return !(unitFlags & kUnitFlagNotSelectable) && !(flagsExtra & kFlagExtraTrigger);
}
// Goldshire's real vendor (unit_flags 512, flags_extra 2), a TAR pedestal (unit_flags
// 0x82040000 with not-selectable), a holiday bunny (trigger).
static_assert(Usable(512, 2) && !Usable(0x82040000u | kUnitFlagNotSelectable, 0) && !Usable(0, 128),
    "usable spawns");

// A mailbox a player can use: not one that cannot be selected (GO_FLAG_NOT_SELECTABLE, from its
// gameobject_template_addon) nor one with the "Point" icon, which the core refuses
// (Player::GetGameObjectIfCanInteractWith). Mailboxes have no faction check. (Core values;
// mod_agent_nav.cpp asserts them.)
constexpr std::uint32_t kGameObjectTypeMailbox = 19;
constexpr std::uint32_t kGoFlagNotSelectable = 0x00000010;
constexpr bool UsableObject(std::uint32_t goFlags, std::string_view icon)
{
    return !(goFlags & kGoFlagNotSelectable) && icon != "Point";
}
// A stock mailbox (no flags, no icon; one template has 0x640000), an unselectable one, a "Point".
static_assert(UsableObject(0, "") && UsableObject(0x640000, "") && !UsableObject(kGoFlagNotSelectable, "") &&
        !UsableObject(0, "Point"),
    "usable mailboxes");

// The auction house an auctioneer serves (core: AuctionHouseMgr::GetAuctionHouseEntryFromFactionTemplate,
// which mod_agent_nav.cpp asks): its faction template's side, or neutral when it has none or the
// realm lets both sides trade (AllowTwoSide.Interaction.Auction). None for anything else.
enum class House : std::uint8_t
{
    None,
    Alliance,
    Horde,
    Neutral,
};

constexpr std::string_view HouseText(House house)
{
    switch (house)
    {
        case House::Alliance:
            return "alliance";
        case House::Horde:
            return "horde";
        case House::Neutral:
            return "neutral";
        case House::None:
            break;
    }
    return "-";
}
static_assert(HouseText(House::Neutral) == "neutral" && HouseText(House::None) == "-", "house words");

// A quest hub: at least this many quest givers within this many yards of one of them.
constexpr std::size_t kHubMembers = 3;
constexpr float kHubRadius = 40.0f;

// Profession skill lines (core: SharedDefines.h SkillType).
constexpr std::uint32_t kSkillCooking = 185;
constexpr std::uint32_t kSkillFirstAid = 129;

enum Kind : std::uint16_t
{
    KindVendor = 1 << 0,
    KindRepair = 1 << 1,
    KindTrainer = 1 << 2,
    KindFlightMaster = 1 << 3,
    KindInnkeeper = 1 << 4,
    KindBanker = 1 << 5,
    KindQuestHub = 1 << 6,
    KindCookingTrainer = 1 << 7,
    KindFirstAidTrainer = 1 << 8,
    KindProfessionTrainer = 1 << 9,
    KindAuctioneer = 1 << 10,
    KindMailbox = 1 << 11,
};

struct KindName
{
    Kind kind;
    std::string_view name;
};

constexpr std::array<KindName, 12> kKindNames{{
    {KindVendor, "vendor"},
    {KindRepair, "repair"},
    {KindTrainer, "trainer"},
    {KindFlightMaster, "flightmaster"},
    {KindInnkeeper, "innkeeper"},
    {KindBanker, "banker"},
    {KindQuestHub, "questgiver-hub"},
    {KindCookingTrainer, "trainer_cooking"},
    {KindFirstAidTrainer, "trainer_first_aid"},
    {KindProfessionTrainer, "trainer_profession"},
    {KindAuctioneer, "auctioneer"},
    {KindMailbox, "mailbox"},
}};

// A comma list of kind names as a mask; 0 for an empty list or an unknown name.
inline std::uint16_t ParseKinds(std::string_view text)
{
    std::uint16_t mask = 0;
    std::size_t start = 0;
    while (start <= text.size())
    {
        std::size_t const end = std::min(text.find(',', start), text.size());
        std::string_view const word = text.substr(start, end - start);
        auto const found = std::find_if(kKindNames.begin(), kKindNames.end(),
            [&](KindName const& k) { return k.name == word; });
        if (found == kKindNames.end())
            return 0;
        mask |= found->kind;
        start = end + 1;
    }
    return mask;
}

inline std::string_view KindText(std::uint16_t kind)
{
    for (KindName const& k : kKindNames)
        if (k.kind == kind)
            return k.name;
    return "?";
}

// A faction template's reaction fields (core: DBCStructure.h FactionTemplateEntry).
struct Faction
{
    std::uint32_t faction = 0;
    std::uint32_t ourMask = 0;
    std::uint32_t friendlyMask = 0;
    std::uint32_t hostileMask = 0;
    std::array<std::uint32_t, 4> enemies{};
    std::array<std::uint32_t, 4> friends{};
};

// FactionTemplateEntry::IsHostileTo, as the core has it: `a` hostile to `b`.
inline bool HostileTo(Faction const& a, Faction const& b)
{
    if (b.faction)
    {
        for (std::uint32_t const f : a.enemies)
            if (f == b.faction)
                return true;
        for (std::uint32_t const f : a.friends)
            if (f == b.faction)
                return false;
    }
    return (a.hostileMask & b.ourMask) != 0;
}

// One creature spawn that offers a service, or a mailbox (a gameobject spawn).
struct Spawn
{
    // The creature's template entry, or the mailbox's gameobject entry.
    std::uint32_t entry = 0;
    // A mailbox: a gameobject, with no NPC flags and no faction (Faction{}: hostile to nobody).
    bool mailbox = false;
    // The auction house an auctioneer serves; None for anything else.
    House house = House::None;
    std::uint32_t map = 0;
    std::uint32_t phaseMask = 1;
    float x = 0.0f;
    float y = 0.0f;
    float z = 0.0f;
    std::uint32_t npcFlags = 0;
    // The class a class trainer trains (Trainer::Type::Class's requirement); 0 for any other.
    std::uint8_t trainerClass = 0;
    // The profession a trainer teaches (a skill line), and its name; 0 and "" for any other.
    std::uint32_t profession = 0;
    std::string professionName;
    Faction faction;
    std::string name;
    // The game events that spawn it (game_event_creature, game_event_gameobject): a negative id
    // spawns it while that event is NOT active. Empty for a standing spawn.
    std::vector<std::int16_t> events;
};

// Whether one game_event_creature row has its spawn out, given whether its event is active: a
// positive event spawns while active, a negative one while not (GameEventMgr's convention).
constexpr bool EventSpawns(std::int16_t event, bool active)
{
    return event > 0 ? active : !active;
}
static_assert(EventSpawns(5, true) && !EventSpawns(5, false) && EventSpawns(-5, false) && !EventSpawns(-5, true),
    "negative events spawn while the event is not active");

// The place's event field: "-" for a standing spawn; "on" when one of its events has it spawned
// now (`active`: the active event ids, sorted), else "off". Told, not filtered: the asker decides.
inline std::string_view EventText(Spawn const& s, std::vector<std::uint16_t> const& active)
{
    if (s.events.empty())
        return "-";
    for (std::int16_t const e : s.events)
    {
        std::uint16_t const id = static_cast<std::uint16_t>(e < 0 ? -e : e);
        if (EventSpawns(e, std::binary_search(active.begin(), active.end(), id)))
            return "on";
    }
    return "off";
}

// Who asks, and what.
struct ServiceQuery
{
    std::uint16_t kinds = 0;
    std::uint32_t map = 0;
    std::uint32_t phaseMask = 1;
    float x = 0.0f;
    float y = 0.0f;
    float radius = 0.0f;
    std::uint8_t playerClass = 0;
    Faction faction;
    std::size_t cap = 20;
    // The game events active when the question was asked (GameEventMgr's list, read on the world
    // thread then), sorted.
    std::vector<std::uint16_t> activeEvents;
};

struct Place
{
    std::uint16_t kind = 0;
    Spawn const* spawn = nullptr;
    float distance = 0.0f;
    std::size_t members = 1;
    // EventText at the asking: "-", "on" or "off".
    std::string_view event = "-";
    // One of the nearest trainers told at any distance (kKindsToldAnyDistance): kept whatever the
    // radius and the cap.
    bool kept = false;
};

// The kinds a spawn offers to this asker (the hub kind aside: that takes its neighbours).
inline std::uint16_t KindsOf(Spawn const& s, ServiceQuery const& q)
{
    std::uint16_t out = 0;
    if (s.mailbox)
        return KindMailbox & q.kinds;
    if (s.npcFlags & kFlagAuctioneer)
        out |= KindAuctioneer;
    if (s.npcFlags & kFlagVendors)
        out |= KindVendor;
    if (s.npcFlags & kFlagRepair)
        out |= KindRepair;
    if ((s.npcFlags & kFlagTrainer) && s.trainerClass != 0 && s.trainerClass == q.playerClass)
        out |= KindTrainer;
    if (s.npcFlags & kFlagFlightMaster)
        out |= KindFlightMaster;
    if (s.npcFlags & kFlagInnkeeper)
        out |= KindInnkeeper;
    if (s.npcFlags & kFlagBanker)
        out |= KindBanker;
    if ((s.npcFlags & kFlagTrainer) && s.profession != 0)
    {
        out |= KindProfessionTrainer;
        if (s.profession == kSkillCooking)
            out |= KindCookingTrainer;
        if (s.profession == kSkillFirstAid)
            out |= KindFirstAidTrainer;
    }
    return out & q.kinds;
}

// The trainer kinds told beyond the radius: the asker's class trainer, Cooking and First Aid. A
// character trains wherever its trainer stands, and a quiet zone may have none within the radius
// (a Gnome Mage in Loch Modan: the nearest mage trainers are in Ironforge), so the nearest
// kTrainersTold of each kind asked for, on the map and not hostile, are always in the answer,
// however far and whatever the cap; one whose event has it off does not count (nor is it told
// from beyond the radius). Others within the radius are told as usual. The client reads how far
// each is from its `x, y, z`. Profession trainers in general are not: the realm would need the
// asker's professions.
constexpr std::uint16_t kKindsToldAnyDistance = KindTrainer | KindCookingTrainer | KindFirstAidTrainer;
constexpr std::size_t kTrainersTold = 2;

// The places for a query: each kind asked for that a spawn on the asker's map and phase, within
// the radius and not hostile either way, offers (so the other side's auctioneers are left out, as
// the core would not deal with the asker; mailboxes have no faction); a spawn offering several
// kinds is one place per kind. Quest hubs are quest givers with at least kHubMembers within
// kHubRadius, one place per hub (its nearest member). Nearest first, at most `cap`; the nearest
// trainers (kKindsToldAnyDistance) are kept whatever the radius and the cap.
inline std::vector<Place> SelectServices(std::vector<Spawn> const& spawns, ServiceQuery const& q)
{
    std::vector<Place> out;
    std::vector<Place> givers;
    for (Spawn const& s : spawns)
    {
        if (s.map != q.map || !(s.phaseMask & q.phaseMask))
            continue;
        if (HostileTo(s.faction, q.faction) || HostileTo(q.faction, s.faction))
            continue;
        float const d = std::hypot(s.x - q.x, s.y - q.y);
        bool const within = d <= q.radius;
        std::uint16_t const kinds = KindsOf(s, q) & (within ? 0xffff : kKindsToldAnyDistance);
        for (KindName const& k : kKindNames)
            if (kinds & k.kind)
                out.push_back(Place{static_cast<std::uint16_t>(k.kind), &s, d, 1});
        if (within && (q.kinds & KindQuestHub) && (s.npcFlags & kFlagQuestGiver))
            givers.push_back(Place{KindQuestHub, &s, d, 1});
    }
    auto const nearest = [](Place const& a, Place const& b) {
        if (a.distance != b.distance)
            return a.distance < b.distance;
        if (a.kind != b.kind)
            return a.kind < b.kind;
        return a.spawn->entry < b.spawn->entry;
    };
    std::sort(givers.begin(), givers.end(), nearest);
    std::vector<Place const*> hubs;
    for (Place& g : givers)
    {
        auto const near = [&](Spawn const* other) {
            return std::hypot(g.spawn->x - other->x, g.spawn->y - other->y) <= kHubRadius;
        };
        // A member of a hub already given is that hub.
        if (std::any_of(hubs.begin(), hubs.end(), [&](Place const* h) { return near(h->spawn); }))
            continue;
        g.members = static_cast<std::size_t>(
            std::count_if(givers.begin(), givers.end(), [&](Place const& o) { return near(o.spawn); }));
        if (g.members >= kHubMembers)
            hubs.push_back(&g);
    }
    for (Place const* h : hubs)
        out.push_back(*h);
    std::sort(out.begin(), out.end(), nearest);
    for (Place& p : out)
        p.event = EventText(*p.spawn, q.activeEvents);
    // The nearest trainers of each kind told at any distance (those whose event has them off
    // aside) are kept; the rest of those beyond the radius go.
    for (KindName const& k : kKindNames)
    {
        if (!(k.kind & kKindsToldAnyDistance))
            continue;
        std::size_t told = 0;
        for (Place& p : out)
            if (p.kind == k.kind && p.event != "off" && told < kTrainersTold)
            {
                p.kept = true;
                ++told;
            }
    }
    out.erase(std::remove_if(out.begin(), out.end(),
                  [&](Place const& p) { return p.distance > q.radius && !p.kept; }),
        out.end());
    // At most `cap`, nearest first, the kept ones before any other.
    std::size_t const kept =
        static_cast<std::size_t>(std::count_if(out.begin(), out.end(), [](Place const& p) { return p.kept; }));
    std::size_t room = q.cap > kept ? q.cap - kept : 0;
    std::vector<Place> capped;
    for (Place const& p : out)
    {
        if (capped.size() >= q.cap)
            break;
        if (p.kept || room > 0)
        {
            if (!p.kept)
                --room;
            capped.push_back(p);
        }
    }
    return capped;
}

// A name as one field: spaces as underscores, "-" for none.
inline std::string Word(std::string const& text)
{
    if (text.empty())
        return "-";
    std::string out = text;
    for (char& c : out)
        if (c == ' ' || static_cast<unsigned char>(c) < 0x20)
            c = '_';
    return out;
}

// The places as reply lines (AgentNavProtocol.h's payload limit: `max` bytes each).
inline std::vector<std::string> ServiceReply(std::uint16_t id, std::vector<Place> const& places, std::size_t max)
{
    std::vector<std::string> out;
    if (places.empty())
    {
        out.push_back("s " + std::to_string(id) + " 0 0");
        return out;
    }
    for (std::size_t i = 0; i < places.size(); ++i)
    {
        Place const& p = places[i];
        char numbers[96];
        std::snprintf(numbers, sizeof numbers, "%.1f %.1f %.1f", static_cast<double>(p.spawn->x),
            static_cast<double>(p.spawn->y), static_cast<double>(p.spawn->z));
        std::string line = "s " + std::to_string(id) + " " + std::to_string(i + 1) + " " +
            std::to_string(places.size()) + " " + std::string(KindText(p.kind)) + " " +
            std::to_string(p.spawn->entry) + " " + numbers + " " + std::to_string(p.members) + " " +
            Word(p.spawn->professionName) + " " + std::string(p.event) + " " +
            std::string(p.kind == KindAuctioneer ? HouseText(p.spawn->house) : HouseText(House::None)) + " ";
        for (char const c : p.spawn->name)
            line += (static_cast<unsigned char>(c) < 0x20 || c == 0x7f) ? ' ' : c;
        if (line.size() > max)
            line.resize(max);
        out.push_back(line);
    }
    return out;
}
}

#endif
