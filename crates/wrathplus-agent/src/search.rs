//! What an agent can call before it writes its own: `GET /lua/search?q=&kind=&limit=` and
//! `GET /lua/doc?name=`. Plain case-insensitive matching on names, doc text and source lines,
//! name hits first; deterministic and cheap, no index beyond the sources themselves.
//!
//! Kinds:
//! - `builtin`: the functions of the built-in modules (`lua/wp/*.lua`, as `Quests.pick_up` or
//!   `wp.values.health`) and the `Agent*` functions registered from Rust ([`AGENT_FNS`]), each with
//!   its signature line, doc comment and `file:line`.
//! - `own`: the agent's saved modules: the module's name, and the lines that match, with a snippet.
//! - `api`: the global functions in the UI VM, which the server asks the game loop for
//!   ([`crate::lua::globals`]); a stock function we bind carries the comment above its binding,
//!   its `file:line` and the binding's source ([`crate::stock_api`]).
//! - `framexml`: lines of the stock interface's Lua and XML off the player's own install, with
//!   the lines around them ([`crate::framexml`]); never shipped, read only when asked.

use serde_json::{json, Value};
use std::path::Path;

/// One thing found: a function (builtin or api) or a line of the agent's own module.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub kind: &'static str,
    pub name: String,
    pub signature: String,
    pub doc: String,
    /// `file:line` (`wp/quest.lua:146`, `world.rs:412`, the agent's `grind/loop.lua:3`).
    pub at: String,
    /// The function's text, for `/lua/doc` (Lua built-ins only).
    pub source: Option<String>,
}

impl Entry {
    pub(crate) fn json(&self, with_source: bool) -> Value {
        let mut v = json!({"kind": self.kind, "name": self.name, "signature": self.signature,
            "doc": self.doc, "at": self.at});
        if let (true, Some(src)) = (with_source, &self.source) {
            v["source"] = json!(src);
        }
        v
    }
}

/// The `Agent*` functions registered from Rust: name, signature, doc, and the file that registers
/// it (its text, to find the line). `search_tests` keeps this list and the registrations in step.
pub(crate) const AGENT_FNS: &[(&str, &str, &str, &str, &str)] = &[
    ("AgentNearby", "AgentNearby([yards]) -> list, note", "Every unit, player, game object and corpse the client knows, nearest first: { guid, kind, name, title, entry, x, y, z, distance, level, health, health_max, dead, reaction, creature_type, in_combat, lootable, tapped, tapped_by_me, target, npc, quest, go_type, quests }. The second value says why the list is empty or far away while a cinematic holds the client.", "world.rs", include_str!("world.rs")),
    ("AgentFindNpc", "AgentFindNpc(service[, yards]) -> unit | nil", "The nearest living NPC in sight offering a service (\"repair\", \"vendor\", \"classtrainer\", \"innkeeper\", \"flightmaster\"...), from its NPC flags; an unknown service is an error naming the known ones.", "world.rs", include_str!("world.rs")),
    ("AgentPosition", "AgentPosition() -> { x, y, z, facing, map } | nil", "Where the character stands, in world coordinates; nil out of the world.", "world.rs", include_str!("world.rs")),
    ("AgentCorpse", "AgentCorpse() -> { x, y, z, map, distance } | nil", "Our corpse, where to run as a ghost (inside an instance, its entrance); distance when on the same map.", "world.rs", include_str!("world.rs")),
    ("AgentTarget", "AgentTarget(guid) -> sent", "Target the object with that \"0x...\" guid.", "world.rs", include_str!("world.rs")),
    ("AgentFace", "AgentFace(guid) -> sent", "Turn to face the object, without walking.", "world.rs", include_str!("world.rs")),
    ("AgentCinematic", "AgentCinematic() -> why | nil", "Why nothing is near or walkable while a cinematic holds the client, else nil.", "world.rs", include_str!("world.rs")),
    ("AgentInteract", "AgentInteract(guid) -> sent", "Talk to, loot, use or attack the object; from farther than a few yards it walks the navmesh route there first (the straight approach without one, or under a cinematic). At a vendor the merchant window opens a moment later.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentPath", "AgentPath(x, y[, z] | guid) -> route | { pending = true } | nil, why", "A route preview: { complete, length, points }. When the realm routes, { pending = true } first; AGENT_PATH_READY fires when it answers and the next call returns the route.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentMoveTo", "AgentMoveTo(x, y[, z] | guid) -> route | { pending = true } | nil, why", "Walk a navmesh route (doors, stairs; without z the floor is looked for). Ends as AGENT_WALK_DONE(status); AgentWalk() follows it. Refused, with the reason, under a cinematic.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentStop", "AgentStop()", "Stop the walk.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentWalk", "AgentWalk() -> { status, remaining, goal, note, route, interact, cinematic }", "The walk: status idle, routing, walking, arrived, stopped, failed or stuck; note says why; route the corners ahead; interact the guid it ends talking to.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentAvoid", "AgentAvoid({ { x, y, radius }, ... }) -> count", "Circles every later route goes round when a fair detour exists; AgentAvoid({}) clears.", "nav/api.rs", include_str!("nav/api.rs")),
    ("AgentServices", "AgentServices([kinds[, radius]]) -> places | { pending = true } | nil, why", "Where services are on your map, asked of the realm: vendor, repair, trainer, flightmaster, innkeeper, banker, questgiver-hub, trainer_cooking, trainer_first_aid, trainer_profession, auctioneer, mailbox. { pending = true } first; after AGENT_SERVICES_READY the next call returns the places, nearest first: { kind, entry, name, x, y, z, map, zone, distance, members, profession, event, event_active, house }. event = true marks a spawn a game event brings out (event_active: whether its event had it out when the realm answered; off: likely not there). house (auctioneers only): the auction house it serves, alliance, horde or neutral (Booty Bay, Gadgetzan, Everlook); the other side's are not told. A mailbox is a game object (entry: its gameobject entry). Within the realm's radius (2000 yd), except the nearest two class, Cooking and First Aid trainers on your map, told however far.", "nav/services.rs", include_str!("nav/services.rs")),
    ("AgentQuestPOI", "AgentQuestPOI(questId) -> { { objective, map, x, y, points }, ... } | nil", "A logged quest's map areas: objective is the objective's index, -1 where it is handed in; x, y the area's centre. Nil until the server answers.", "quest_poi.rs", include_str!("quest_poi.rs")),
    ("AgentZones", "AgentZones() -> { { id, name, map, faction, city, min, max, x, y }, ... }", "The zones from the client's own tables, with the level band each suits and a point in the middle.", "zones.rs", include_str!("zones.rs")),
    ("AgentPoint", "AgentPoint(u, v[, frame]) -> { x, y, z, hit, distance, range, bearing, turn, pitch, frame } | nil, why[, { bearing, turn, pitch, frame }]", "What is at pixel (u, v) of a screenshot (its frame number, else the newest; pixels from the image's top-left, as sent): the camera ray of that frame met against the world now. hit is terrain, wmo (a building), doodad (a tree, a fence) or other; distance is from you, range from the camera; bearing is the direction from you as AgentPosition().facing counts it, turn how far to turn from your facing (positive left). Units and water are not met. Nothing met (sky, past the loaded world): nil, why and the ray's bearing. From a frame script only the bearing.", "point.rs", include_str!("point.rs")),
    ("AgentSignal", "AgentSignal(name, ...)", "Put AGENT_<NAME> with the arguments on the event stream, to wake the tiers above.", "lib.rs", include_str!("lib.rs")),
    ("AgentReload", "AgentReload(name) -> module", "Load a module again after a change (save_module does not).", "env.rs", include_str!("env.rs")),
    ("AgentModules", "AgentModules() -> names", "The agent's own saved modules.", "env.rs", include_str!("env.rs")),
];

/// The 1-based line of the first line of `text` containing `needle`, else 1.
fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|l| l.contains(needle))
        .map_or(1, |i| i + 1)
}

fn ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `function T.name(params)`, `function T:name(params)` or `T.name = function(params)`: the
/// function's name and parameters, for the module table `T`.
fn definition(line: &str, table: &str) -> Option<(String, String)> {
    let (name, rest) = if let Some(rest) = line.strip_prefix("function ") {
        let rest = rest.strip_prefix(table)?;
        let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(':'))?;
        let (name, params) = rest.split_once('(')?;
        (name.trim(), params)
    } else {
        let rest = line.strip_prefix(table)?.strip_prefix('.')?;
        let (name, rhs) = rest.split_once('=')?;
        let params = rhs.trim_start().strip_prefix("function")?.trim_start();
        (name.trim(), params.strip_prefix('(')?)
    };
    ident(name).then(|| {
        (
            name.to_string(),
            rest.split_once(')').map_or("", |p| p.0).to_string(),
        )
    })
}

/// The comment lines just above line `i`, without their dashes.
fn comment_above(lines: &[&str], i: usize) -> String {
    let mut out: Vec<&str> = lines[..i]
        .iter()
        .rev()
        .take_while(|l| l.trim_start().starts_with("--"))
        .map(|l| l.trim_start().trim_start_matches('-').trim())
        .collect();
    out.reverse();
    out.join(" ")
}

/// The module header's entry for `qualified` (`--   Quests.pick_up()   take every ...`) and its
/// continuation lines, the deeper-indented ones that follow.
fn header_entry(lines: &[&str], qualified: &str) -> String {
    let header: Vec<&str> = lines
        .iter()
        .take_while(|l| l.starts_with("--"))
        .copied()
        .collect();
    let text = |l: &str| l.trim_start_matches('-').to_string();
    let Some(at) = header.iter().position(|l| {
        let t = text(l);
        t.trim_start().starts_with(qualified)
            && !t.trim_start()[qualified.len()..]
                .starts_with(|c: char| c.is_alphanumeric() || c == '_')
    }) else {
        return String::new();
    };
    let mut out = vec![text(header[at]).trim().to_string()];
    for l in &header[at + 1..] {
        let t = text(l);
        if t.len() - t.trim_start().len() < 6 || t.trim().is_empty() {
            break;
        }
        out.push(t.trim().to_string());
    }
    out.join(" ")
}

/// The function's text from its definition to the `end` that closes it at the same indentation
/// (the definition line alone when it closes there). `search_tests` checks every built-in's span.
pub(crate) fn body(lines: &[&str], i: usize) -> String {
    let def = lines[i].trim_end();
    if def.ends_with(" end") || def.ends_with(")end") {
        return def.to_string();
    }
    let indent = lines[i].len() - lines[i].trim_start().len();
    let end = lines[i + 1..]
        .iter()
        .take(120)
        .position(|l| l.len() - l.trim_start().len() == indent && l.trim_start().starts_with("end"))
        .map_or((i + 40).min(lines.len()), |j| i + j + 2);
    lines[i..end.min(lines.len())].join("\n")
}

/// The public functions of one Lua module: its table is what it returns (`return Q`), named by
/// its global (`Quests`) or, without one, by the module (`wp.values`).
pub(crate) fn lua_functions(
    kind: &'static str,
    module: &str,
    global: Option<&str>,
    file: &str,
    src: &str,
) -> Vec<Entry> {
    let lines: Vec<&str> = src.lines().collect();
    let Some(table) = lines.iter().rev().find_map(|l| {
        l.strip_prefix("return ")
            .map(str::trim)
            .filter(|t| ident(t))
    }) else {
        return Vec::new();
    };
    let prefix = global.unwrap_or(module);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((name, params)) = definition(line, table) else {
            continue;
        };
        let qualified = format!("{prefix}.{name}");
        let mut doc = comment_above(&lines, i);
        if doc.is_empty() {
            doc = header_entry(&lines, &qualified);
        }
        out.push(Entry {
            kind,
            signature: format!("{qualified}({params})"),
            name: qualified,
            doc,
            at: format!("{file}:{}", i + 1),
            source: Some(body(&lines, i)),
        });
    }
    out
}

/// Every built-in function: the modules' and the Rust-registered `Agent*` ones.
pub(crate) fn builtins() -> Vec<Entry> {
    let mut out: Vec<Entry> = AGENT_FNS
        .iter()
        .map(|(name, signature, doc, file, text)| Entry {
            kind: "builtin",
            name: name.to_string(),
            signature: signature.to_string(),
            doc: doc.to_string(),
            at: format!("{file}:{}", line_of(text, &format!("\"{name}\""))),
            source: None,
        })
        .collect();
    for (module, global, src) in crate::env::built_ins() {
        let file = format!("wp/{}.lua", module.trim_start_matches("wp."));
        out.extend(lua_functions("builtin", module, *global, &file, src));
    }
    out
}

/// The agent's own modules: the module itself (by name) and every line that matches `q`.
pub(crate) fn own(dir: &Path, q: &str) -> Vec<(u8, Entry)> {
    let mut out = Vec::new();
    for module in crate::env::list(dir) {
        let Ok(src) = std::fs::read_to_string(crate::env::module_path(dir, &module)) else {
            continue;
        };
        let file = format!("{}.lua", module.replace('.', "/"));
        let lines: Vec<&str> = src.lines().collect();
        let name_hit = rank_name(&module.to_lowercase(), q);
        if let Some(score) = name_hit {
            out.push((
                score,
                own_entry(&module, &file, 1, lines.first().copied().unwrap_or("")),
            ));
        }
        for (i, line) in lines.iter().enumerate() {
            if !q.is_empty() && line.to_lowercase().contains(q) {
                let def = line.trim_start().starts_with("function") || line.contains("= function");
                out.push((
                    if def { 3 } else { 5 },
                    own_entry(&module, &file, i + 1, line),
                ));
            }
        }
    }
    out
}

fn own_entry(module: &str, file: &str, line: usize, text: &str) -> Entry {
    Entry {
        kind: "own",
        name: module.to_string(),
        signature: String::new(),
        doc: text.trim().chars().take(160).collect(),
        at: format!("{file}:{line}"),
        source: None,
    }
}

/// How well a name answers `q` (lower case): 0 exactly, 1 as a prefix (or of its last part), 2
/// containing it; `None` not at all. An empty `q` matches every name, last.
fn rank_name(name: &str, q: &str) -> Option<u8> {
    let last = name.rsplit('.').next().unwrap_or(name);
    if q.is_empty() {
        Some(2)
    } else if name == q || last == q {
        Some(0)
    } else if name.starts_with(q) || last.starts_with(q) {
        Some(1)
    } else {
        name.contains(q).then_some(2)
    }
}

/// A built-in's score for `q`: its name first, then its doc (3), then its source (4).
fn rank_builtin(e: &Entry, q: &str) -> Option<u8> {
    rank_name(&e.name.to_lowercase(), q)
        .or_else(|| e.doc.to_lowercase().contains(q).then_some(3))
        .or_else(|| {
            e.source
                .as_ref()
                .filter(|s| s.to_lowercase().contains(q))
                .map(|_| 4)
        })
}

/// Every kind, in rank order; all are searched by default. `framexml` comes last: it shows how the
/// stock UI uses a function once the builtin, own and api hits have said what exists.
pub(crate) const KINDS: [&str; 4] = ["builtin", "own", "api", "framexml"];

/// [`search_in`] over the player's own stock UI ([`crate::framexml::installed`]).
pub(crate) fn search(
    q: &str,
    kinds: &[&str],
    scripts: &Path,
    api: impl FnOnce() -> Vec<String>,
    limit: usize,
) -> Value {
    search_in(q, kinds, scripts, api, crate::framexml::installed, limit)
}

/// Search `kinds` for `q`; `api` is the VM's global function names, asked of the game loop only
/// when that kind is wanted, and `stock_ui` the stock interface's files, read only when
/// `framexml` is. A stock function matches by name or by the doc of its binding
/// ([`crate::stock_api`]). Best first: by score, then kind (builtin, own, api, framexml), then
/// name; a `framexml` hit carries the lines around it as `source`.
pub(crate) fn search_in<'a>(
    q: &str,
    kinds: &[&str],
    scripts: &Path,
    api: impl FnOnce() -> Vec<String>,
    stock_ui: impl FnOnce() -> &'a [wrathplus_app_ui::ui_script::StockFile],
    limit: usize,
) -> Value {
    let q = q.trim().to_lowercase();
    let mut hits: Vec<(u8, usize, Entry)> = Vec::new();
    if kinds.contains(&"builtin") {
        for e in builtins() {
            if let Some(s) = rank_builtin(&e, &q) {
                hits.push((s, 0, e));
            }
        }
    }
    if kinds.contains(&"own") {
        hits.extend(own(scripts, &q).into_iter().map(|(s, e)| (s, 1, e)));
    }
    if kinds.contains(&"api") {
        for name in api() {
            // The Agent* functions are built-ins, with their docs.
            if AGENT_FNS.iter().any(|f| f.0 == name) {
                continue;
            }
            let entry = crate::stock_api::entry(&name)
                .unwrap_or_else(|| crate::stock_api::bare(name.clone()));
            let score = rank_name(&name.to_lowercase(), &q)
                .or_else(|| (!q.is_empty() && entry.doc.to_lowercase().contains(&q)).then_some(3));
            if let Some(s) = score {
                hits.push((s, 2, entry));
            }
        }
    }
    if kinds.contains(&"framexml") {
        hits.extend(
            crate::framexml::search(stock_ui(), &q)
                .into_iter()
                .map(|(s, e)| (s, 3, e)),
        );
    }
    hits.sort_by(|a, b| (a.0, a.1, &a.2.name, &a.2.at).cmp(&(b.0, b.1, &b.2.name, &b.2.at)));
    let total = hits.len();
    let results: Vec<Value> = hits
        .iter()
        .take(limit)
        .map(|(_, k, e)| e.json(*k == 3))
        .collect();
    json!({"query": q, "kinds": kinds, "total": total, "results": results})
}

/// [`doc_in`] over the player's own stock UI.
pub(crate) fn doc(
    name: &str,
    scripts: &Path,
    in_api: impl FnOnce(&str) -> bool,
) -> Result<Value, Value> {
    doc_in(name, scripts, in_api, crate::framexml::installed)
}

/// What one name is: a built-in (with its source when it is Lua), an own module (its source), a
/// global of the VM (`in_api`: with our binding's doc and source when we bind it), or the stock
/// UI's (a function it defines, a file, `file:line`). `Err` carries suggestions.
pub(crate) fn doc_in<'a>(
    name: &str,
    scripts: &Path,
    in_api: impl FnOnce(&str) -> bool,
    stock_ui: impl FnOnce() -> &'a [wrathplus_app_ui::ui_script::StockFile],
) -> Result<Value, Value> {
    let lower = name.trim().to_lowercase();
    let all = builtins();
    if let Some(e) = all.iter().find(|e| e.name.to_lowercase() == lower) {
        return Ok(e.json(true));
    }
    if crate::env::valid_module(name) {
        if let Ok(src) = std::fs::read_to_string(crate::env::module_path(scripts, name)) {
            return Ok(
                json!({"kind": "own", "name": name, "at": format!("{}.lua:1", name.replace('.', "/")), "source": src}),
            );
        }
    }
    let by_last: Vec<&Entry> = all
        .iter()
        .filter(|e| {
            e.name
                .rsplit('.')
                .next()
                .is_some_and(|l| l.to_lowercase() == lower)
        })
        .collect();
    match by_last.as_slice() {
        [one] => return Ok(one.json(true)),
        [] => {}
        many => {
            let names: Vec<&str> = many.iter().map(|e| e.name.as_str()).collect();
            return Err(
                json!({"error": format!("{name} names several functions: ask for one"), "candidates": names}),
            );
        }
    }
    let global = in_api(name);
    if global {
        if let Some(e) = crate::stock_api::entry(name.trim()) {
            return Ok(e.json(true));
        }
    }
    if let Some(found) = crate::framexml::doc(stock_ui(), name) {
        return Ok(found);
    }
    if global {
        return Ok(
            json!({"kind": "api", "name": name, "doc": "A global function of the UI's Lua that WrathPlus does not bind itself: Lua's own library, or one an addon or module defined. Its behaviour is the 3.3.5a client's."}),
        );
    }
    let suggestions = search(&lower, &["builtin", "own"], scripts, Vec::new, 5)["results"]
        .as_array()
        .map(|r| r.iter().map(|e| e["name"].clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    Err(json!({"error": format!("nothing is named {name}"), "suggestions": suggestions}))
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
