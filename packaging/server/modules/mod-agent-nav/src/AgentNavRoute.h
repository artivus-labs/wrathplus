// mod-agent-nav: the route search. Detour and the standard library only: no core calls, so it
// runs on the module's own thread, over the module's own copy of the navmesh tiles, and the
// WrathPlus client's tests compile this same file to compare its routes with the client's own
// (crates/wrathplus-nav/csrc/realm_module_shim.cpp).
//
// The search is the client's (crates/wrathplus-nav: src/lib.rs `route`/`search`,
// csrc/nav_shim.cpp), step for step, so a character routed by the realm walks where one with
// local mmaps would. Keep the two in step.
#ifndef MOD_AGENT_NAV_ROUTE_H
#define MOD_AGENT_NAV_ROUTE_H

#include "AgentNavProtocol.h"
#include "DetourAlloc.h"
#include "DetourCommon.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshQuery.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <map>
#include <set>
#include <string>
#include <vector>

namespace AgentNav
{
// The mmaps file format (core: MapDefines.h; mod_agent_nav.cpp asserts these against it).
constexpr std::uint32_t kMmapMagic = 0x4d4d4150;
constexpr std::uint32_t kMmapVersion = 20;
constexpr std::size_t kMmapHeaderSize = 56;
constexpr float kGridSize = 533.3333f;
// NavTerrainFlag, which is also the polygon area id.
constexpr unsigned short kGround = 0x01, kMagma = 0x02, kSlime = 0x04, kWater = 0x08, kSteep = 0x10;
// What a yard inside an avoided circle costs, and water (mod-playerbots' bot cost).
constexpr float kAvoidCost = 10.0f;
constexpr float kWaterCost = 20.0f;
// A corner within this of the asked end, level, counts as reaching it.
constexpr float kReached = 2.0f;
constexpr int kMaxPolys = 1024;

static_assert(sizeof(dtPolyRef) == 8, "AzerothCore mmaps need 64-bit poly refs");

// The terrain filter, plus circles to keep out of: a step whose midpoint or end lies in one
// costs `mult` times as much, so a route goes round when a fair detour exists and through when
// none does.
struct AvoidFilter : public dtQueryFilter
{
    std::vector<Circle> zones; // Recast order: x is WoW y, y is WoW x
    float mult = 1.0f;

    bool Inside(float x, float z) const
    {
        for (Circle const& c : zones)
        {
            float const dx = x - c.x, dz = z - c.y;
            if (dx * dx + dz * dz < c.r * c.r)
                return true;
        }
        return false;
    }

    float getCost(const float* pa, const float* pb, const dtPolyRef prevRef, const dtMeshTile* prevTile,
        const dtPoly* prevPoly, const dtPolyRef curRef, const dtMeshTile* curTile, const dtPoly* curPoly,
        const dtPolyRef nextRef, const dtMeshTile* nextTile, const dtPoly* nextPoly) const override
    {
        float const cost = dtQueryFilter::getCost(pa, pb, prevRef, prevTile, prevPoly, curRef, curTile, curPoly,
            nextRef, nextTile, nextPoly);
        if (!zones.empty() && (Inside((pa[0] + pb[0]) * 0.5f, (pa[2] + pb[2]) * 0.5f) || Inside(pb[0], pb[2])))
            return cost * mult;
        return cost;
    }
};

struct RouteResult
{
    // "" or the error code of the reply: "nomesh", "noroute".
    std::string error;
    std::vector<std::array<float, 3>> points; // WoW {x, y, z}
    RouteFlags flags;
};

struct RouteOptions
{
    // The end may move this far (yards, level) to walkable ground; 0: the usual 3.
    float endSnap = 0.0f;
    int maxPoints = 256;
    // The end's height is a guess: when no complete route ends there, try other heights
    // (Heights) and take the first that gives one.
    bool anyHeight = false;
};

// The heights tried for an end whose height is a guess, after the guess itself: 15 yards apart,
// ten each way, the nearest first, up then down. A flat point (a quest area, a zone's centre) in
// a town on several levels is rarely at the floor the walk starts from. The client's own search
// (crates/wrathplus-agent/src/nav.rs, `heights`) tries the same.
inline std::vector<float> Heights(float z)
{
    std::vector<float> out;
    for (int i = 1; i <= 10; ++i)
    {
        out.push_back(z + 15.0f * static_cast<float>(i));
        out.push_back(z - 15.0f * static_cast<float>(i));
    }
    return out;
}

inline int Grid(float coord)
{
    return static_cast<int>(std::floor(32.0f - coord / kGridSize));
}

// Navmeshes read from `<dir>/<map>.mmap` and `<dir>/<map><gx><gy>.mmtile`, tiles loaded as
// routes need them and the least recently used dropped past `maxTiles`. One thread only.
class MeshCache
{
public:
    MeshCache(std::string dir, int maxNodes, std::size_t maxTiles)
        : _dir(std::move(dir)), _maxNodes(std::clamp(maxNodes, 256, 65535)), _maxTiles(std::max<std::size_t>(maxTiles, 1))
    {
    }

    ~MeshCache()
    {
        for (auto& entry : _maps)
            Free(entry.second);
    }

    MeshCache(MeshCache const&) = delete;
    MeshCache& operator=(MeshCache const&) = delete;

    std::size_t LoadedTiles() const { return _loaded; }

    RouteResult Route(std::uint32_t map, std::array<float, 3> const& from, std::array<float, 3> const& to,
        std::vector<Circle> const& avoid, RouteOptions const& options)
    {
        RouteResult none;
        MapMesh* mesh = Open(map);
        if (!mesh)
        {
            none.error = "nomesh";
            return none;
        }
        ++_clock;
        int const ax = Grid(from[0]), ay = Grid(from[1]), bx = Grid(to[0]), by = Grid(to[1]);
        for (int gx = std::min(ax, bx) - 1; gx <= std::max(ax, bx) + 1; ++gx)
            for (int gy = std::min(ay, by) - 1; gy <= std::max(ay, by) + 1; ++gy)
                Ensure(map, *mesh, gx, gy);
        Evict();

        AvoidFilter filter;
        filter.setAreaCost(kWater, kWaterCost);
        filter.mult = kAvoidCost;
        for (Circle const& c : avoid)
            filter.zones.push_back(Circle{c.y, c.x, c.r});

        RouteResult const first = Terrain(*mesh, filter, from, to, options);
        if (!options.anyHeight || (first.error.empty() && first.flags.complete))
            return first;
        for (float const z : Heights(to[2]))
        {
            RouteResult other = Terrain(*mesh, filter, from, {to[0], to[1], z}, options);
            if (other.error.empty() && other.flags.complete)
                return other;
        }
        return first;
    }

private:
    struct Tile
    {
        dtTileRef ref = 0;
        std::uint64_t used = 0;
    };

    struct MapMesh
    {
        dtNavMesh* mesh = nullptr;
        dtNavMeshQuery* query = nullptr;
        std::map<std::pair<int, int>, Tile> tiles;
        // Cells with no tile file (open sea, off the map) or an unusable one.
        std::set<std::pair<int, int>> missing;
    };

    static void Free(MapMesh& m)
    {
        dtFreeNavMeshQuery(m.query);
        dtFreeNavMesh(m.mesh);
        m.query = nullptr;
        m.mesh = nullptr;
    }

    static bool ReadFile(std::string const& path, std::vector<unsigned char>& out)
    {
        std::FILE* file = std::fopen(path.c_str(), "rb");
        if (!file)
            return false;
        unsigned char buffer[65536];
        std::size_t got;
        while ((got = std::fread(buffer, 1, sizeof buffer, file)) > 0)
            out.insert(out.end(), buffer, buffer + got);
        std::fclose(file);
        return true;
    }

    std::string Name(std::uint32_t map, int gx, int gy, bool tile) const
    {
        char name[32];
        if (tile)
            std::snprintf(name, sizeof name, "%03u%02d%02d.mmtile", map, gx, gy);
        else
            std::snprintf(name, sizeof name, "%03u.mmap", map);
        return _dir + "/" + name;
    }

    MapMesh* Open(std::uint32_t map)
    {
        auto found = _maps.find(map);
        if (found != _maps.end())
            return found->second.mesh ? &found->second : nullptr;
        MapMesh m;
        std::vector<unsigned char> params;
        if (ReadFile(Name(map, 0, 0, false), params) && params.size() == sizeof(dtNavMeshParams))
        {
            dtNavMeshParams p;
            std::memcpy(&p, params.data(), sizeof p);
            m.mesh = dtAllocNavMesh();
            m.query = dtAllocNavMeshQuery();
            if (!m.mesh || !m.query || dtStatusFailed(m.mesh->init(&p)) || dtStatusFailed(m.query->init(m.mesh, _maxNodes)))
                Free(m);
        }
        // A map without a navmesh is remembered too: asked once, not on every request.
        MapMesh& kept = _maps[map] = m;
        return kept.mesh ? &kept : nullptr;
    }

    void Ensure(std::uint32_t map, MapMesh& m, int gx, int gy)
    {
        std::pair<int, int> const cell(gx, gy);
        auto found = m.tiles.find(cell);
        if (found != m.tiles.end())
        {
            found->second.used = _clock;
            return;
        }
        if (gx < 0 || gx > 63 || gy < 0 || gy > 63 || m.missing.count(cell))
            return;
        std::vector<unsigned char> bytes;
        std::uint32_t magic = 0, version = 0, size = 0;
        if (ReadFile(Name(map, gx, gy, true), bytes) && bytes.size() >= kMmapHeaderSize)
        {
            std::memcpy(&magic, bytes.data(), 4);
            std::memcpy(&version, bytes.data() + 8, 4);
            std::memcpy(&size, bytes.data() + 12, 4);
        }
        if (magic != kMmapMagic || version != kMmapVersion || size == 0 || bytes.size() - kMmapHeaderSize < size)
        {
            m.missing.insert(cell);
            return;
        }
        // Detour keeps the pointer; with DT_TILE_FREE_DATA it frees it when the tile goes.
        unsigned char* data = static_cast<unsigned char*>(dtAlloc(size, DT_ALLOC_PERM));
        if (!data)
            return;
        std::memcpy(data, bytes.data() + kMmapHeaderSize, size);
        Tile tile;
        tile.used = _clock;
        if (dtStatusFailed(m.mesh->addTile(data, static_cast<int>(size), DT_TILE_FREE_DATA, 0, &tile.ref)))
        {
            dtFree(data);
            m.missing.insert(cell);
            return;
        }
        m.tiles[cell] = tile;
        ++_loaded;
    }

    // Drop the least recently used tiles past the limit, never one this request asked for.
    void Evict()
    {
        while (_loaded > _maxTiles)
        {
            MapMesh* owner = nullptr;
            std::map<std::pair<int, int>, Tile>::iterator oldest;
            for (auto& entry : _maps)
                for (auto it = entry.second.tiles.begin(); it != entry.second.tiles.end(); ++it)
                    if (it->second.used != _clock && (!owner || it->second.used < oldest->second.used))
                    {
                        owner = &entry.second;
                        oldest = it;
                    }
            if (!owner)
                return;
            owner->mesh->removeTile(oldest->second.ref, nullptr, nullptr);
            owner->tiles.erase(oldest);
            --_loaded;
        }
    }

    // Keep off steep ground, which a walking player slides back down; only when no gentle route
    // reaches the end is one over steep ground taken.
    static RouteResult Terrain(MapMesh& m, AvoidFilter& filter, std::array<float, 3> const& from,
        std::array<float, 3> const& to, RouteOptions const& options)
    {
        RouteResult gentle = Search(m, filter, from, to, false, options);
        if (gentle.error.empty() && gentle.flags.complete)
            return gentle;
        RouteResult steep = Search(m, filter, from, to, true, options);
        if (steep.error.empty() && steep.flags.complete)
            return steep;
        return gentle.error.empty() ? gentle : steep;
    }

    // findNearestPoly at both ends, findPath, findStraightPath (the client's nav_find_straight_path).
    static dtStatus StraightPath(MapMesh& m, AvoidFilter const& filter, const float* start, const float* end,
        const float* startExtents, const float* endExtents, std::vector<float>& out, int maxPoints, int& count,
        bool& truncated)
    {
        count = 0;
        truncated = false;
        dtPolyRef startRef = 0, endRef = 0;
        float startPoint[3], endPoint[3];
        dtStatus status = m.query->findNearestPoly(start, startExtents, &filter, &startRef, startPoint);
        if (dtStatusFailed(status) || !startRef)
            return DT_FAILURE | DT_INVALID_PARAM;
        status = m.query->findNearestPoly(end, endExtents, &filter, &endRef, endPoint);
        if (dtStatusFailed(status) || !endRef)
            return DT_FAILURE | DT_INVALID_PARAM;
        dtPolyRef polys[kMaxPolys];
        int polyCount = 0;
        status = m.query->findPath(startRef, endRef, startPoint, endPoint, &filter, polys, &polyCount, kMaxPolys);
        if (dtStatusFailed(status) || polyCount == 0)
            return status | DT_FAILURE;
        float last[3];
        dtVcopy(last, endPoint);
        if (polys[polyCount - 1] != endRef) // partial path: clamp the end onto the last polygon
            m.query->closestPointOnPoly(polys[polyCount - 1], endPoint, last, nullptr);
        dtStatus const straight = m.query->findStraightPath(startPoint, last, polys, polyCount, out.data(), nullptr,
            nullptr, &count, maxPoints);
        truncated = (straight & DT_BUFFER_TOO_SMALL) != 0;
        return straight | (status & DT_STATUS_DETAIL_MASK);
    }

    static RouteResult Search(MapMesh& m, AvoidFilter& filter, std::array<float, 3> const& from,
        std::array<float, 3> const& to, bool steep, RouteOptions const& options)
    {
        if (steep)
        {
            filter.setIncludeFlags(kGround | kSteep | kWater);
            filter.setExcludeFlags(kMagma | kSlime);
        }
        else
        {
            filter.setIncludeFlags(kGround | kWater);
            filter.setExcludeFlags(kMagma | kSlime | kSteep);
        }
        float const start[3] = {from[1], from[2], from[0]};
        float const end[3] = {to[1], to[2], to[0]};
        int const maxPoints = std::clamp(options.maxPoints, 2, 512);
        std::vector<float> out(static_cast<std::size_t>(maxPoints) * 3);
        float const side = options.endSnap > 0.0f ? options.endSnap : 3.0f;
        // The server's first search box, its taller second, then a much taller one for a height
        // that is simply wrong (an agent's guess).
        float const heights[3] = {5.0f, 50.0f, 400.0f};
        dtStatus status = DT_FAILURE;
        int count = 0;
        bool truncated = false;
        for (float const height : heights)
        {
            float const startExtents[3] = {3.0f, height, 3.0f};
            float const endExtents[3] = {side, height, side};
            status = StraightPath(m, filter, start, end, startExtents, endExtents, out, maxPoints, count, truncated);
            if (!dtStatusFailed(status) && count > 0)
                break;
        }
        RouteResult result;
        if (dtStatusFailed(status) || count == 0)
        {
            result.error = "noroute";
            return result;
        }
        for (int i = 0; i < count; ++i)
            result.points.push_back({out[i * 3 + 2], out[i * 3], out[i * 3 + 1]});
        std::array<float, 3> const& last = result.points.back();
        bool const reached = Distance2d(last[0], last[1], to[0], to[1]) < kReached;
        result.flags.complete = (status & DT_PARTIAL_RESULT) == 0 && reached;
        result.flags.steep = steep;
        result.flags.truncated = truncated;
        return result;
    }

    std::string _dir;
    int _maxNodes;
    std::size_t _maxTiles;
    std::size_t _loaded = 0;
    std::uint64_t _clock = 0;
    std::map<std::uint32_t, MapMesh> _maps;
};
}

#endif
