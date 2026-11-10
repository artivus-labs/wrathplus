// A C ABI over mod-agent-nav's own protocol and route code
// (packaging/server/modules/mod-agent-nav/src), for this crate's tests only: the client's
// encoder and decoder are checked against the module's, and the module's routes against the
// client's, from the same source files the realm compiles. Nothing in the client calls this, so
// the linker leaves it out of the game.
#include "AgentNavProtocol.h"
#include "AgentNavRoute.h"

#include <cstdio>
#include <cstring>
#include <string>

namespace
{
int Put(std::string const& text, char* out, int out_len)
{
    if (out_len <= 0)
        return -1;
    std::size_t const n = std::min<std::size_t>(text.size(), static_cast<std::size_t>(out_len) - 1);
    std::memcpy(out, text.data(), n);
    out[n] = 0;
    return static_cast<int>(text.size());
}

std::string Num(float v)
{
    char buffer[32];
    std::snprintf(buffer, sizeof buffer, "%.3f", static_cast<double>(v));
    return buffer;
}

std::string Dump(AgentNav::Message const& m)
{
    switch (m.verb)
    {
        case AgentNav::Verb::Hello:
            return "hello " + std::to_string(m.version);
        case AgentNav::Verb::Avoid:
        {
            std::string out = "avoid " + std::to_string(m.id);
            for (AgentNav::Circle const& c : m.circles)
                out += " " + Num(c.x) + " " + Num(c.y) + " " + Num(c.r);
            return out;
        }
        case AgentNav::Verb::Path:
            break;
    }
    return "path " + std::to_string(m.id) + (m.near ? " near " : " exact ") + (m.anyHeight ? "anyz " : "") + Num(m.from[0]) + " " + Num(m.from[1]) + " " +
        Num(m.from[2]) + " " + Num(m.to[0]) + " " + Num(m.to[1]) + " " + Num(m.to[2]) + (m.hasToZ ? " z" : " noz");
}
}

extern "C" {

// The module's reading of a client line, or "!reason".
int realm_parse(const char* payload, char* out, int out_len)
{
    AgentNav::Message message;
    std::string const wrong = AgentNav::Parse(payload, message);
    return Put(wrong.empty() ? Dump(message) : "!" + wrong + " " + std::to_string(message.id), out, out_len);
}

// The limit check for a route request from a character at `at`: "" or the refusal line.
int realm_check(const char* payload, const float* at, float max_distance, float max_start, char* out, int out_len)
{
    AgentNav::Message message;
    if (!AgentNav::Parse(payload, message).empty() || message.verb != AgentNav::Verb::Path)
        return Put("!parse", out, out_len);
    AgentNav::Limits limits;
    limits.maxDistance = max_distance;
    limits.maxStartDistance = max_start;
    return Put(AgentNav::CheckPath(message, limits, {at[0], at[1], at[2]}), out, out_len);
}

// Client lines (newline-separated A lines) through the pending circles, then the count taken
// for request `take`: "<count>" or "!refusal".
int realm_avoid(const char* lines, int max_avoid, int take, char* out, int out_len)
{
    AgentNav::PendingAvoid pending;
    std::string_view rest(lines);
    while (!rest.empty())
    {
        std::size_t const end = rest.find('\n');
        std::string_view const line = rest.substr(0, end);
        rest = end == std::string_view::npos ? std::string_view() : rest.substr(end + 1);
        AgentNav::Message message;
        if (!AgentNav::Parse(line, message).empty())
            return Put("!parse", out, out_len);
        std::string const why = pending.Add(message, static_cast<std::uint32_t>(max_avoid));
        if (!why.empty())
            return Put("!" + why, out, out_len);
    }
    return Put(std::to_string(pending.Take(static_cast<std::uint16_t>(take)).size()), out, out_len);
}

int realm_hello_reply(char* out, int out_len)
{
    return Put(AgentNav::HelloReply(AgentNav::Limits()), out, out_len);
}

int realm_error_reply(int id, const char* code, const char* detail, char* out, int out_len)
{
    return Put(AgentNav::ErrorReply(static_cast<std::uint16_t>(id), code, detail), out, out_len);
}

// A route as the module's reply lines, newline-separated. flags: 1 complete, 2 steep, 4 truncated.
int realm_route_reply(int id, int flags, const float* points, int count, char* out, int out_len)
{
    std::vector<std::array<float, 3>> route;
    for (int i = 0; i < count; ++i)
        route.push_back({points[i * 3], points[i * 3 + 1], points[i * 3 + 2]});
    AgentNav::RouteFlags f;
    f.complete = (flags & 1) != 0;
    f.steep = (flags & 2) != 0;
    f.truncated = (flags & 4) != 0;
    std::string joined;
    for (std::string const& line : AgentNav::RouteReply(static_cast<std::uint16_t>(id), f, route))
        joined += (joined.empty() ? "" : "\n") + line;
    return Put(joined, out, out_len);
}

// The rate limiter over `count` request times (ms): verdicts 0 ok, 1 per second, 2 per minute.
void realm_rate(const unsigned long long* times, int count, int per_second, int per_minute, unsigned char* verdicts)
{
    AgentNav::RateLimiter limiter;
    for (int i = 0; i < count; ++i)
        verdicts[i] = static_cast<unsigned char>(limiter.Allow(times[i], static_cast<std::uint32_t>(per_second),
            static_cast<std::uint32_t>(per_minute)));
}

void* realm_cache_new(const char* dir, int nodes, int tiles)
{
    return new AgentNav::MeshCache(dir, nodes, static_cast<std::size_t>(tiles));
}

void realm_cache_free(void* cache)
{
    delete static_cast<AgentNav::MeshCache*>(cache);
}

int realm_cache_tiles(void* cache)
{
    return static_cast<int>(static_cast<AgentNav::MeshCache*>(cache)->LoadedTiles());
}

// A route by the module's search. Returns the corner count (points: WoW x, y, z), -1 for
// "nomesh", -2 for "noroute"; flags as realm_route_reply's.
int realm_cache_route(void* cache, unsigned map, const float* from, const float* to, const float* avoid, int avoid_count,
    float end_snap, int max_points, int any_height, float* points, int* flags)
{
    std::vector<AgentNav::Circle> circles;
    for (int i = 0; i < avoid_count; ++i)
        circles.push_back(AgentNav::Circle{avoid[i * 3], avoid[i * 3 + 1], avoid[i * 3 + 2]});
    AgentNav::RouteOptions options;
    options.endSnap = end_snap;
    options.maxPoints = max_points;
    options.anyHeight = any_height != 0;
    AgentNav::RouteResult const route = static_cast<AgentNav::MeshCache*>(cache)->Route(map, {from[0], from[1], from[2]},
        {to[0], to[1], to[2]}, circles, options);
    if (!route.error.empty())
        return route.error == "nomesh" ? -1 : -2;
    for (std::size_t i = 0; i < route.points.size(); ++i)
        std::memcpy(points + i * 3, route.points[i].data(), sizeof(float) * 3);
    *flags = (route.flags.complete ? 1 : 0) | (route.flags.steep ? 2 : 0) | (route.flags.truncated ? 4 : 0);
    return static_cast<int>(route.points.size());
}

} // extern "C"
