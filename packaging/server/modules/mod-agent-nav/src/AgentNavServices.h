// mod-agent-nav: where services are. The realm tells an agent where the vendors, repairers, its
// class trainer, flight masters, innkeepers, bankers and quest hubs on its map are, as a player
// knows a town and as Playerbots knows the world: from the creature spawns the world database
// holds, read once at startup (mod_agent_nav.cpp), never per request. Standard library only, so
// the WrathPlus client's tests compile this file and check the selection against a fixture.
//
//   S <id> <kinds> [radius]        kinds: a comma list of vendor, repair, trainer (the asker's
//                                  class trainer), flightmaster, innkeeper, banker,
//                                  questgiver-hub, trainer_cooking, trainer_first_aid,
//                                  trainer_profession (any profession trainer); radius in yards
//                                  (default and most: AgentNav.ServiceRadius)
//   s <id> <i> <n> <kind> <entry> <x> <y> <z> <members> <profession> <name>
//                                  the i-th of n places, nearest first; members: the quest givers
//                                  in a hub (1 for anything else); profession: the skill a
//                                  profession trainer teaches, spaces as underscores ("First_Aid"),
//                                  "-" for anything else; the name is the rest of the line
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
constexpr std::uint32_t kServiceFlags = kFlagQuestGiver | kFlagTrainer | kFlagVendors | kFlagRepair |
    kFlagFlightMaster | kFlagInnkeeper | kFlagBanker;

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
};

struct KindName
{
    Kind kind;
    std::string_view name;
};

constexpr std::array<KindName, 10> kKindNames{{
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

// One creature spawn that offers a service.
struct Spawn
{
    std::uint32_t entry = 0;
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
};

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
};

struct Place
{
    std::uint16_t kind = 0;
    Spawn const* spawn = nullptr;
    float distance = 0.0f;
    std::size_t members = 1;
};

// The kinds a spawn offers to this asker (the hub kind aside: that takes its neighbours).
inline std::uint16_t KindsOf(Spawn const& s, ServiceQuery const& q)
{
    std::uint16_t out = 0;
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

// The places for a query: each kind asked for that a spawn on the asker's map and phase, within
// the radius and not hostile either way, offers; a spawn offering several kinds is one place per
// kind. Quest hubs are quest givers with at least kHubMembers within kHubRadius, one place per
// hub (its nearest member). Nearest first, at most `cap`.
inline std::vector<Place> SelectServices(std::vector<Spawn> const& spawns, ServiceQuery const& q)
{
    std::vector<Place> out;
    std::vector<Place> givers;
    for (Spawn const& s : spawns)
    {
        if (s.map != q.map || !(s.phaseMask & q.phaseMask))
            continue;
        float const d = std::hypot(s.x - q.x, s.y - q.y);
        if (d > q.radius || HostileTo(s.faction, q.faction) || HostileTo(q.faction, s.faction))
            continue;
        std::uint16_t const kinds = KindsOf(s, q);
        for (KindName const& k : kKindNames)
            if (kinds & k.kind)
                out.push_back(Place{static_cast<std::uint16_t>(k.kind), &s, d, 1});
        if ((q.kinds & KindQuestHub) && (s.npcFlags & kFlagQuestGiver))
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
    if (out.size() > q.cap)
        out.resize(q.cap);
    return out;
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
            Word(p.spawn->professionName) + " ";
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
