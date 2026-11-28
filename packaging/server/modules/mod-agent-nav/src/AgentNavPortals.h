// mod-agent-nav: portals, the services kind `portal` (AgentNavServices.h). A portal is an area
// trigger that teleports: the Deeprun Tram's entrances, an instance's door and its way out, the
// Darnassus portal in Rut'theran. None is a unit or a gameobject, so an agent cannot see one; the
// realm knows them all. Each is an `areatrigger_teleport` row (where it leads) with its own volume
// from `areatrigger` (the AreaTrigger.dbc copy ObjectMgr loads: map, centre, a sphere's radius or
// a box), and the instance's access requirement (`dungeon_access_template` and
// `dungeon_access_requirements`, ObjectMgr::GetAccessRequirement), read once at startup
// (mod_agent_nav.cpp). Walking into the volume is how a player uses one: the client sends
// CMSG_AREATRIGGER and the core teleports, when the requirement is met. Standard library only, so
// the WrathPlus client's tests compile this file too.
//
// A portal place's four fields, after `house` (each "-" for any other kind):
//   <area>   the volume: a sphere's radius ("10.0"), or a box "<length>x<width>x<height>@<yaw>"
//            (yards, the yaw in radians), centred on the place's x, y, z
//   <to>     where it leads: "<map>,<x>,<y>,<z>"
//   <level>  the level the core asks for (Player::Satisfy): "<min>", "<min>-<max>", "-<max>", or
//            "-" for none (and when the realm ignores instance levels, Instance.IgnoreLevel)
//   <needs>  what else it asks, a comma list, or "-": "raid" (a raid group, unless the realm's
//            Instance.IgnoreRaid), "quest:<id>" (rewarded), "item:<id>" (carried),
//            "achievement:<id>", "item_level:<n>" (average); those of the asker's side only, as
//            the core checks them, at the asker's dungeon or raid difficulty
// and the place's name is the destination map's (Map.dbc: "Deeprun Tram", "The Deadmines").
// Nothing is filtered by level or need: they are told, and the asker decides.
#ifndef MOD_AGENT_NAV_PORTALS_H
#define MOD_AGENT_NAV_PORTALS_H

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdio>
#include <initializer_list>
#include <string>
#include <string_view>
#include <vector>

namespace AgentNav
{
// Teams as the core numbers them (SharedDefines.h TeamId; mod_agent_nav.cpp asserts them).
constexpr std::uint8_t kTeamAlliance = 0;
constexpr std::uint8_t kTeamHorde = 1;
constexpr std::uint8_t kTeamNeutral = 2;

// The difficulties a requirement is kept for (DBCEnums.h Difficulty: dungeon normal and heroic,
// raid 10 and 25 normal and heroic share 0..3).
constexpr std::size_t kDifficulties = 4;

// One `dungeon_access_requirements` row, as ObjectMgr::LoadAccessRequirements sorts them.
struct Need
{
    enum class Type : std::uint8_t
    {
        Achievement = 0,
        Quest = 1,
        Item = 2,
    };
    Type type = Type::Quest;
    std::uint32_t id = 0;
    // kTeamNeutral: either side; else only that side is asked for it (Player::Satisfy).
    std::uint8_t team = kTeamNeutral;
};

// An instance's requirement at one difficulty (DungeonProgressionRequirements); all 0 and empty
// for none.
struct Access
{
    std::uint8_t levelMin = 0;
    std::uint8_t levelMax = 0;
    std::uint16_t itemLevel = 0;
    std::vector<Need> needs;
};

// Where a portal leads and its volume; the trigger's centre is the Spawn's x, y, z.
struct Teleport
{
    // A sphere's radius; 0 for a box.
    float radius = 0.0f;
    float length = 0.0f;
    float width = 0.0f;
    float height = 0.0f;
    float orientation = 0.0f;
    std::uint32_t toMap = 0;
    float toX = 0.0f;
    float toY = 0.0f;
    float toZ = 0.0f;
    // The destination is a raid: its requirement is read at the asker's raid difficulty, else at
    // its dungeon difficulty.
    bool raidMap = false;
    // A raid group is needed to go in (a raid, unless the realm's Instance.IgnoreRaid).
    bool raidGroup = false;
    // The requirement at each difficulty the asker may have set, after the core's downscaling
    // (GetDownscaledMapDifficultyData: heroic asks of a dungeon with none are normal's).
    std::array<Access, kDifficulties> access{};
};

inline std::string PortalArea(Teleport const& t)
{
    char text[96];
    if (t.radius > 0.0f)
        std::snprintf(text, sizeof text, "%.1f", static_cast<double>(t.radius));
    else
        std::snprintf(text, sizeof text, "%.1fx%.1fx%.1f@%.2f", static_cast<double>(t.length),
            static_cast<double>(t.width), static_cast<double>(t.height), static_cast<double>(t.orientation));
    return text;
}

inline std::string PortalTo(Teleport const& t)
{
    char text[96];
    std::snprintf(text, sizeof text, "%u,%.1f,%.1f,%.1f", static_cast<unsigned>(t.toMap), static_cast<double>(t.toX),
        static_cast<double>(t.toY), static_cast<double>(t.toZ));
    return text;
}

inline std::string PortalLevel(Access const& a)
{
    if (a.levelMin == 0 && a.levelMax == 0)
        return "-";
    std::string out = a.levelMin ? std::to_string(a.levelMin) : "";
    if (a.levelMax)
        out += "-" + std::to_string(a.levelMax);
    return out;
}

constexpr std::string_view NeedWord(Need::Type type)
{
    switch (type)
    {
        case Need::Type::Achievement:
            return "achievement";
        case Need::Type::Item:
            return "item";
        case Need::Type::Quest:
            break;
    }
    return "quest";
}
static_assert(NeedWord(Need::Type::Item) == "item" && NeedWord(Need::Type::Achievement) == "achievement",
    "need words");

// The needs of the asker's side (`team`, TeamId), as Player::Satisfy checks them; "-" for none.
inline std::string PortalNeeds(Teleport const& t, Access const& a, std::uint8_t team)
{
    std::string out;
    auto const add = [&](std::string const& word) { out += (out.empty() ? "" : ",") + word; };
    if (t.raidGroup)
        add("raid");
    for (Need::Type const type : {Need::Type::Quest, Need::Type::Item, Need::Type::Achievement})
        for (Need const& n : a.needs)
            if (n.type == type && (n.team == kTeamNeutral || n.team == team))
                add(std::string(NeedWord(type)) + ":" + std::to_string(n.id));
    if (a.itemLevel)
        add("item_level:" + std::to_string(a.itemLevel));
    return out.empty() ? "-" : out;
}

// A portal place's four fields for this asker: area, to, level, needs.
inline std::string PortalFields(Teleport const& t, std::uint8_t team, std::uint8_t dungeonDifficulty,
    std::uint8_t raidDifficulty)
{
    std::size_t const d = std::min<std::size_t>(t.raidMap ? raidDifficulty : dungeonDifficulty, kDifficulties - 1);
    Access const& a = t.access[d];
    return PortalArea(t) + " " + PortalTo(t) + " " + PortalLevel(a) + " " + PortalNeeds(t, a, team);
}
}

#endif
