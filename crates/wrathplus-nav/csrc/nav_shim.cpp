// Minimal C ABI over the vendored Detour (third_party/detour, AzerothCore's copy: DT_POLYREF64,
// 12/21/31 ref bits), for crates/wrathplus-nav.
// All positions are Recast order {y, z, x} of WoW coordinates.
#include <cstring>
#include <cstdint>
#include "DetourNavMesh.h"
#include "DetourNavMeshQuery.h"
#include "DetourCommon.h"
#include "DetourAlloc.h"

static_assert(sizeof(dtPolyRef) == 8, "AzerothCore mmaps need 64-bit poly refs");
static_assert(sizeof(dtNavMeshParams) == 28, "dtNavMeshParams layout");

// The terrain filter, plus circles to keep out of (an agent's hostile mobs): a step whose midpoint
// or end lies in one costs `mult` times as much, so a route goes round when a fair detour exists
// and through when none does. Needs DT_VIRTUAL_QUERYFILTER, which AzerothCore's copy defines.
struct AvoidFilter : public dtQueryFilter {
    static const int MAX_ZONES = 64;
    float zones[MAX_ZONES * 3]; // Recast {x, z, radius}
    int count = 0;
    float mult = 1.0f;
    bool inside(float x, float z) const {
        for (int i = 0; i < count; ++i) {
            float dx = x - zones[i * 3], dz = z - zones[i * 3 + 1], r = zones[i * 3 + 2];
            if (dx * dx + dz * dz < r * r) return true;
        }
        return false;
    }
    float getCost(const float* pa, const float* pb,
                  const dtPolyRef prevRef, const dtMeshTile* prevTile, const dtPoly* prevPoly,
                  const dtPolyRef curRef, const dtMeshTile* curTile, const dtPoly* curPoly,
                  const dtPolyRef nextRef, const dtMeshTile* nextTile, const dtPoly* nextPoly) const override {
        float cost = dtQueryFilter::getCost(pa, pb, prevRef, prevTile, prevPoly, curRef, curTile, curPoly,
                                            nextRef, nextTile, nextPoly);
        if (count && (inside((pa[0] + pb[0]) * 0.5f, (pa[2] + pb[2]) * 0.5f) || inside(pb[0], pb[2])))
            return cost * mult;
        return cost;
    }
};
struct NavShim {
    dtNavMesh* mesh;
    dtNavMeshQuery* query;
    AvoidFilter filter;
};

extern "C" {

// params: raw 28-byte dtNavMeshParams from <map>.mmap. Returns null on failure.
NavShim* nav_create(const uint8_t* params, int params_len, int max_nodes) {
    if (params_len != (int)sizeof(dtNavMeshParams)) return nullptr;
    dtNavMeshParams p;
    std::memcpy(&p, params, sizeof p);
    NavShim* s = new NavShim();
    s->mesh = dtAllocNavMesh();
    s->query = dtAllocNavMeshQuery();
    if (!s->mesh || !s->query || dtStatusFailed(s->mesh->init(&p)) ||
        dtStatusFailed(s->query->init(s->mesh, max_nodes))) {
        dtFreeNavMeshQuery(s->query);
        dtFreeNavMesh(s->mesh);
        delete s;
        return nullptr;
    }
    return s;
}

void nav_destroy(NavShim* s) {
    if (!s) return;
    dtFreeNavMeshQuery(s->query);
    dtFreeNavMesh(s->mesh);
    delete s;
}

// data: Detour tile bytes (the .mmtile payload after the 56-byte MmapTileHeader).
// Detour copies nothing, so we copy into dtAlloc memory and let it own it (DT_TILE_FREE_DATA).
uint32_t nav_add_tile(NavShim* s, const uint8_t* data, int len, uint64_t* out_ref) {
    unsigned char* buf = (unsigned char*)dtAlloc(len, DT_ALLOC_PERM);
    if (!buf) return DT_FAILURE | DT_OUT_OF_MEMORY;
    std::memcpy(buf, data, len);
    dtTileRef ref = 0;
    dtStatus st = s->mesh->addTile(buf, len, DT_TILE_FREE_DATA, 0, &ref);
    if (dtStatusFailed(st)) dtFree(buf);
    if (out_ref) *out_ref = ref;
    return st;
}

uint32_t nav_remove_tile(NavShim* s, uint64_t ref) {
    return s->mesh->removeTile(ref, nullptr, nullptr);
}

void nav_set_filter(NavShim* s, uint16_t include, uint16_t exclude) {
    s->filter.setIncludeFlags(include);
    s->filter.setExcludeFlags(exclude);
}

void nav_set_area_cost(NavShim* s, int area, float cost) { s->filter.setAreaCost(area, cost); }
// zones: count * {x, z, radius} in Recast order; past MAX_ZONES the rest are ignored.
void nav_set_avoid(NavShim* s, const float* zones, int count, float mult) {
    if (count > AvoidFilter::MAX_ZONES) count = AvoidFilter::MAX_ZONES;
    if (count < 0) count = 0;
    std::memcpy(s->filter.zones, zones, sizeof(float) * 3 * count);
    s->filter.count = count;
    s->filter.mult = mult;
}

uint32_t nav_find_nearest_poly(NavShim* s, const float* pos, const float* ext, uint64_t* ref, float* nearest) {
    dtPolyRef r = 0;
    dtStatus st = s->query->findNearestPoly(pos, ext, &s->filter, &r, nearest);
    *ref = r;
    return st;
}

// findPath + findStraightPath. out_points holds max_points*3 floats (Recast order).
uint32_t nav_find_straight_path(NavShim* s, const float* start, const float* end, const float* ext,
                                float* out_points, int max_points, int* out_count, int* out_poly_count) {
    *out_count = 0;
    dtPolyRef sref = 0, eref = 0;
    float sp[3], ep[3];
    dtStatus st = s->query->findNearestPoly(start, ext, &s->filter, &sref, sp);
    if (dtStatusFailed(st) || !sref) return DT_FAILURE | DT_INVALID_PARAM;
    st = s->query->findNearestPoly(end, ext, &s->filter, &eref, ep);
    if (dtStatusFailed(st) || !eref) return DT_FAILURE | DT_INVALID_PARAM;
    const int MAXP = 1024;
    dtPolyRef polys[MAXP];
    int npolys = 0;
    st = s->query->findPath(sref, eref, sp, ep, &s->filter, polys, &npolys, MAXP);
    if (dtStatusFailed(st) || npolys == 0) return st | DT_FAILURE;
    float epos[3];
    dtVcopy(epos, ep);
    if (polys[npolys - 1] != eref) // partial path: clamp the end onto the last poly
        s->query->closestPointOnPoly(polys[npolys - 1], ep, epos, nullptr);
    if (out_poly_count) *out_poly_count = npolys;
    dtStatus st2 = s->query->findStraightPath(sp, epos, polys, npolys, out_points, nullptr, nullptr,
                                              out_count, max_points);
    return st2 | (st & DT_STATUS_DETAIL_MASK);
}

} // extern "C"
