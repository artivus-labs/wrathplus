// mod-agent-nav: the wire protocol, request checks and rate limit. Standard library only, so the
// WrathPlus client's tests compile this same file and check their own encoder and decoder against
// it (crates/wrathplus-nav/csrc/realm_module_shim.cpp).
//
// Transport: addon messages on the game connection. The client whispers itself in LANG_ADDON,
//   SendAddonMessage("WPNAV", payload, "WHISPER", UnitName("player"))
// which reaches the realm as "WPNAV\t" + payload; the module swallows it and answers with
// LANG_ADDON whispers from the player to itself, so replies arrive as CHAT_MSG_ADDON with prefix
// WPNAV. A whole line (prefix, TAB, payload) is at most 255 bytes. Payloads are ASCII, fields
// separated by single spaces.
//
// Client to realm (upper-case verbs):
//   H <version>                              capability probe
//   A <id> <x> <y> <r> [<x> <y> <r>]...      circles to route around, for the request <id> that
//                                            follows; several A lines add up
//   P <id> <flags> <fx> <fy> <fz> <tx> <ty> <tz|->
//                                            a route on the character's own map. flags: "-" or
//                                            letters from n (the end may be moved to the nearest
//                                            walkable ground: one leg of a longer walk) and h (tz
//                                            is a guess: look for the floor on other levels too).
//                                            tz "-": use fz.
// Coordinates are WoW yards as decimals ("-8907.2"); ids are 1..65535.
//
// Realm to client (lower-case verbs, so a realm without the module, which simply delivers the
// whisper back, can never be mistaken for an answer):
//   h <version> <max distance> <max points> <max circles> <per second> <per minute>
//   r <id> <part> <parts> <flags> <dx,dy,dz>...
//                                            the corners, in tenths of a yard: the first of part 1
//                                            is absolute, every later one the step from the corner
//                                            before it (running on across parts). flags: "-" or
//                                            letters from c (complete: it reaches the end), s (it
//                                            crosses steep ground), t (cut at the corner limit)
//   e <id> <code> [detail]                   rate | far | start | nomesh | noroute | bad | busy
#ifndef MOD_AGENT_NAV_PROTOCOL_H
#define MOD_AGENT_NAV_PROTOCOL_H

#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <string>
#include <string_view>
#include <vector>

namespace AgentNav
{
constexpr std::string_view kPrefix = "WPNAV";
constexpr std::uint32_t kVersion = 1;
// The chat handler drops anything longer; a stock client cannot send or show more either.
constexpr std::size_t kMaxWire = 255;
constexpr std::size_t kMaxPayload = kMaxWire - kPrefix.size() - 1;
// The world's coordinates end at +-17066.7; anything past this is not a position.
constexpr float kMaxCoordinate = 20000.0f;
constexpr float kMaxRadius = 100.0f;

struct Circle
{
    float x = 0.0f;
    float y = 0.0f;
    float r = 0.0f;
};

// What a realm allows, with the defaults mod_agent_nav.conf.dist documents.
struct Limits
{
    std::uint32_t perSecond = 5;
    std::uint32_t perMinute = 240;
    float maxDistance = 1500.0f;
    float maxStartDistance = 100.0f;
    std::uint32_t maxWaypoints = 256;
    std::uint32_t maxAvoid = 64;
};

enum class Verb
{
    Hello,
    Avoid,
    Path
};

struct Message
{
    Verb verb = Verb::Hello;
    std::uint32_t version = 0;
    // Set as soon as it is read, so a refusal can name the request.
    std::uint16_t id = 0;
    std::vector<Circle> circles;
    bool near = false;
    bool anyHeight = false;
    std::array<float, 3> from{};
    std::array<float, 3> to{};
    bool hasToZ = false;
};

inline std::vector<std::string_view> Fields(std::string_view text)
{
    std::vector<std::string_view> out;
    std::size_t start = 0;
    while (true)
    {
        std::size_t const end = text.find(' ', start);
        out.push_back(text.substr(start, end == std::string_view::npos ? end : end - start));
        if (end == std::string_view::npos)
            return out;
        start = end + 1;
    }
}

// Plain decimals only: an optional sign, digits, at most three decimals. No exponent, hex,
// "nan" or "inf", whatever the C library's locale would make of them.
inline bool ParseNumber(std::string_view text, float limit, float& out)
{
    if (text.empty() || text.size() > 12)
        return false;
    std::size_t i = 0;
    bool const negative = text[0] == '-';
    if (negative)
        ++i;
    double value = 0.0;
    std::size_t digits = 0;
    for (; i < text.size() && text[i] >= '0' && text[i] <= '9'; ++i, ++digits)
        value = value * 10.0 + (text[i] - '0');
    if (i < text.size())
    {
        if (text[i] != '.')
            return false;
        ++i;
        double scale = 0.1;
        std::size_t decimals = 0;
        for (; i < text.size() && text[i] >= '0' && text[i] <= '9'; ++i, ++decimals, scale *= 0.1)
            value += (text[i] - '0') * scale;
        if (i != text.size() || decimals == 0 || decimals > 3)
            return false;
    }
    if (digits == 0 || value > limit)
        return false;
    out = static_cast<float>(negative ? -value : value);
    return true;
}

inline bool ParseCount(std::string_view text, std::uint32_t limit, std::uint32_t& out)
{
    if (text.empty() || text.size() > 6)
        return false;
    std::uint32_t value = 0;
    for (char const c : text)
    {
        if (c < '0' || c > '9')
            return false;
        value = value * 10 + static_cast<std::uint32_t>(c - '0');
    }
    if (value > limit)
        return false;
    out = value;
    return true;
}

// A client line into a message. Returns "" or what is wrong with it (the detail of `e <id> bad`).
inline std::string Parse(std::string_view payload, Message& out)
{
    out = Message();
    if (payload.empty() || payload.size() > kMaxPayload)
        return "length";
    for (char const c : payload)
        if (c < 0x20 || c > 0x7e)
            return "characters";
    std::vector<std::string_view> const f = Fields(payload);
    if (f[0] == "H")
    {
        out.verb = Verb::Hello;
        if (f.size() != 2 || !ParseCount(f[1], 65535, out.version) || out.version == 0)
            return "hello";
        return "";
    }
    if (f[0] != "A" && f[0] != "P")
        return "verb";
    std::uint32_t id = 0;
    if (f.size() < 2 || !ParseCount(f[1], 65535, id) || id == 0)
        return "id";
    out.id = static_cast<std::uint16_t>(id);
    if (f[0] == "A")
    {
        out.verb = Verb::Avoid;
        if (f.size() < 5 || (f.size() - 2) % 3 != 0)
            return "circles";
        for (std::size_t i = 2; i < f.size(); i += 3)
        {
            Circle c;
            if (!ParseNumber(f[i], kMaxCoordinate, c.x) || !ParseNumber(f[i + 1], kMaxCoordinate, c.y) ||
                !ParseNumber(f[i + 2], kMaxRadius, c.r) || c.r <= 0.0f)
                return "circle";
            out.circles.push_back(c);
        }
        return "";
    }
    out.verb = Verb::Path;
    if (f.size() != 9)
        return "fields";
    if (f[2] != "-")
    {
        if (f[2].size() > 2)
            return "flags";
        for (char const c : f[2])
        {
            bool& flag = c == 'n' ? out.near : out.anyHeight;
            if ((c != 'n' && c != 'h') || flag)
                return "flags";
            flag = true;
        }
    }
    for (std::size_t i = 0; i < 3; ++i)
        if (!ParseNumber(f[3 + i], kMaxCoordinate, out.from[i]))
            return "from";
    if (!ParseNumber(f[6], kMaxCoordinate, out.to[0]) || !ParseNumber(f[7], kMaxCoordinate, out.to[1]))
        return "to";
    out.to[2] = out.from[2];
    if (f[8] != "-")
    {
        if (!ParseNumber(f[8], kMaxCoordinate, out.to[2]))
            return "to";
        out.hasToZ = true;
    }
    return "";
}

// The circles sent ahead of a request. A new id starts over; "" or the refusal's detail.
struct PendingAvoid
{
    std::uint16_t id = 0;
    std::vector<Circle> circles;

    std::string Add(Message const& message, std::uint32_t maxAvoid)
    {
        if (message.id != id)
        {
            id = message.id;
            circles.clear();
        }
        if (circles.size() + message.circles.size() > maxAvoid)
        {
            circles.clear();
            id = 0;
            return "circles: at most " + std::to_string(maxAvoid);
        }
        circles.insert(circles.end(), message.circles.begin(), message.circles.end());
        return "";
    }

    // The circles for request `request` (none when they were sent for another), and forget them.
    std::vector<Circle> Take(std::uint16_t request)
    {
        std::vector<Circle> out;
        if (request == id)
            out.swap(circles);
        circles.clear();
        id = 0;
        return out;
    }
};

inline float Distance2d(float ax, float ay, float bx, float by)
{
    return std::hypot(ax - bx, ay - by);
}

inline std::string ErrorReply(std::uint16_t id, std::string_view code, std::string_view detail = "")
{
    std::string out = "e " + std::to_string(id) + " " + std::string(code);
    if (!detail.empty())
        out += " " + std::string(detail);
    if (out.size() > kMaxPayload)
        out.resize(kMaxPayload);
    return out;
}

// Whether a route request is within the limits, for a character standing at `at`. Returns ""
// or the whole refusal line.
inline std::string CheckPath(Message const& message, Limits const& limits, std::array<float, 3> const& at)
{
    float const start = Distance2d(message.from[0], message.from[1], at[0], at[1]);
    if (limits.maxStartDistance > 0.0f && start > limits.maxStartDistance)
        return ErrorReply(message.id, "start",
            "routes start within " + std::to_string(static_cast<int>(limits.maxStartDistance)) + " yd of the character");
    float const span = Distance2d(message.from[0], message.from[1], message.to[0], message.to[1]);
    if (limits.maxDistance > 0.0f && span > limits.maxDistance)
        return ErrorReply(message.id, "far", std::to_string(static_cast<int>(limits.maxDistance)));
    return "";
}

inline std::string HelloReply(Limits const& limits)
{
    return "h " + std::to_string(kVersion) + " " + std::to_string(static_cast<int>(limits.maxDistance)) + " " +
        std::to_string(limits.maxWaypoints) + " " + std::to_string(limits.maxAvoid) + " " +
        std::to_string(limits.perSecond) + " " + std::to_string(limits.perMinute);
}

struct RouteFlags
{
    bool complete = false;
    bool steep = false;
    bool truncated = false;
};

inline long Tenths(float v)
{
    return std::lround(static_cast<double>(v) * 10.0);
}

// A route as reply lines, each within kMaxPayload. `points` are WoW {x, y, z}.
inline std::vector<std::string> RouteReply(std::uint16_t id, RouteFlags flags, std::vector<std::array<float, 3>> const& points)
{
    // "r 65535 999 999 cst " is the longest header.
    constexpr std::size_t header = 20;
    std::vector<std::string> bodies(1);
    std::array<long, 3> last{0, 0, 0};
    for (std::array<float, 3> const& p : points)
    {
        std::array<long, 3> const now{Tenths(p[0]), Tenths(p[1]), Tenths(p[2])};
        std::string const token = std::to_string(now[0] - last[0]) + "," + std::to_string(now[1] - last[1]) + "," +
            std::to_string(now[2] - last[2]);
        last = now;
        if (!bodies.back().empty() && bodies.back().size() + 1 + token.size() > kMaxPayload - header)
            bodies.emplace_back();
        if (!bodies.back().empty())
            bodies.back() += ' ';
        bodies.back() += token;
    }
    std::string letters;
    if (flags.complete)
        letters += 'c';
    if (flags.steep)
        letters += 's';
    if (flags.truncated)
        letters += 't';
    if (letters.empty())
        letters = "-";
    std::vector<std::string> out;
    for (std::size_t i = 0; i < bodies.size(); ++i)
    {
        std::string line = "r " + std::to_string(id) + " " + std::to_string(i + 1) + " " +
            std::to_string(bodies.size()) + " " + letters;
        if (!bodies[i].empty())
            line += " " + bodies[i];
        out.push_back(line);
    }
    return out;
}

// Route requests one character may make: `perSecond` in any second, `perMinute` in any minute
// (0: no limit). A refused request is not counted, so waiting is always enough to be let in again.
class RateLimiter
{
public:
    enum class Verdict
    {
        Ok,
        PerSecond,
        PerMinute
    };

    Verdict Allow(std::uint64_t nowMs, std::uint32_t perSecond, std::uint32_t perMinute)
    {
        while (!_stamps.empty() && _stamps.front() + 60000 <= nowMs)
            _stamps.pop_front();
        if (perMinute != 0 && _stamps.size() >= perMinute)
            return Verdict::PerMinute;
        if (perSecond != 0)
        {
            std::uint32_t recent = 0;
            for (auto it = _stamps.rbegin(); it != _stamps.rend() && *it + 1000 > nowMs; ++it)
                ++recent;
            if (recent >= perSecond)
                return Verdict::PerSecond;
        }
        _stamps.push_back(nowMs);
        // With no per-minute limit, the per-second count still only needs the last second.
        while (perMinute == 0 && _stamps.size() > perSecond + 1)
            _stamps.pop_front();
        return Verdict::Ok;
    }

private:
    std::deque<std::uint64_t> _stamps;
};
}

#endif
