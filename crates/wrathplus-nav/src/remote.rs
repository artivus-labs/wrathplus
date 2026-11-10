//! Routes asked of the realm instead of read from local mmaps: the client side of
//! `mod-agent-nav`'s wire protocol (`packaging/server/modules/mod-agent-nav/src/AgentNavProtocol.h`,
//! which states the format). Lines travel as addon messages with prefix [`PREFIX`], whispered to
//! oneself; this module only builds and reads the payloads.

use crate::Route;

/// The addon-message prefix.
pub const PREFIX: &str = "WPNAV";
pub const VERSION: u32 = 1;
/// Prefix, TAB and payload fit one 255-byte chat line.
pub const MAX_PAYLOAD: usize = 255 - PREFIX.len() - 1;
/// Most parts and corners a reply may have, whatever a realm says: a bound on what is kept.
const MAX_PARTS: u16 = 128;
const MAX_POINTS: usize = 1024;

/// What the realm allows, from its answer to the probe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Longest route, in yards start to end in a straight line (0: no limit).
    pub max_distance: f32,
    pub max_points: u32,
    pub max_avoid: u32,
    pub per_second: u32,
    pub per_minute: u32,
}

/// One route request.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u16,
    pub from: [f32; 3],
    pub to: [f32; 2],
    /// `None`: the height is unknown, and the realm starts from `from`'s.
    pub to_z: Option<f32>,
    /// One leg of a longer walk: the end may be moved to the nearest walkable ground.
    pub near: bool,
    /// `to_z` is a guess: the realm may look for the floor on other levels (the client's own
    /// search does the same for a target given without a height).
    pub any_height: bool,
    /// Circles to route around, WoW `[x, y, radius]`.
    pub avoid: Vec<[f32; 3]>,
}

/// A coordinate as the wire carries it: yards to a tenth, never `-0.0`.
fn number(v: f32) -> String {
    let text = format!("{v:.1}");
    if text == "-0.0" {
        "0.0".into()
    } else {
        text
    }
}

/// A request's flags field: `-`, or the letters of what is set.
fn flags(request: &Request) -> String {
    let letters: String = [(request.near, 'n'), (request.any_height, 'h')]
        .iter()
        .filter_map(|(on, c)| on.then_some(*c))
        .collect();
    if letters.is_empty() {
        "-".into()
    } else {
        letters
    }
}

/// The capability probe.
pub fn hello() -> String {
    format!("H {VERSION}")
}

/// A request as payload lines, in sending order: its circles (as many lines as they need), then
/// the request itself.
pub fn encode(request: &Request) -> Vec<String> {
    let mut lines = Vec::new();
    let head = format!("A {}", request.id);
    let mut line = head.clone();
    for c in &request.avoid {
        let circle = format!(" {} {} {}", number(c[0]), number(c[1]), number(c[2]));
        if line.len() + circle.len() > MAX_PAYLOAD {
            lines.push(std::mem::replace(&mut line, head.clone()));
        }
        line.push_str(&circle);
    }
    if line.len() > head.len() {
        lines.push(line);
    }
    lines.push(format!(
        "P {} {} {} {} {} {} {} {}",
        request.id,
        flags(request),
        number(request.from[0]),
        number(request.from[1]),
        number(request.from[2]),
        number(request.to[0]),
        number(request.to[1]),
        request.to_z.map_or("-".into(), number),
    ));
    lines
}

/// Why the realm refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Too many requests; the detail says which limit.
    Rate,
    /// Farther than the realm routes in one request.
    Far,
    /// The route does not start near the character.
    Start,
    /// The realm has no navmesh for this map.
    NoMesh,
    /// Start or end is not on walkable ground.
    NoRoute,
    /// The realm could not read the request.
    Bad,
    /// The realm's queue is full.
    Busy,
    /// A code this client does not know.
    Other,
}

impl Refusal {
    /// The refusal in words, for a walk's note.
    pub fn explain(self, detail: &str) -> String {
        match self {
            Refusal::Rate => format!(
                "the realm refused the route: too many requests (it allows {detail} per character); wait a moment"
            ),
            Refusal::Far => format!("the realm routes at most {detail} yd at a time"),
            Refusal::Start => format!("the realm refused the route: {detail}"),
            Refusal::NoMesh => format!("no navmesh: {detail}"),
            Refusal::NoRoute => format!("no route: {detail}"),
            Refusal::Bad => format!("the realm could not read the route request ({detail})"),
            Refusal::Busy => format!("the realm is busy: {detail}"),
            Refusal::Other => format!("the realm refused the route ({detail})"),
        }
    }
}

/// One part of a route reply.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub id: u16,
    pub part: u16,
    pub parts: u16,
    pub complete: bool,
    pub steep: bool,
    pub truncated: bool,
    /// Tenths of a yard: steps from the corner before (the first of part 1 from the origin).
    pub steps: Vec<[i32; 3]>,
}

/// A line from the realm.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Hello(Limits),
    Part(Part),
    Refused {
        id: u16,
        refusal: Refusal,
        detail: String,
    },
    /// One of our own lines come back: the realm delivered the whisper instead of answering it,
    /// so it has no route service.
    Echo,
}

/// Read a payload from the realm.
pub fn decode(payload: &str) -> Result<Reply, String> {
    let bad = || format!("unreadable realm nav line: {payload:?}");
    let fields: Vec<&str> = payload.split(' ').collect();
    let int = |i: usize| -> Result<u32, String> {
        fields
            .get(i)
            .and_then(|f| f.parse::<u32>().ok())
            .ok_or_else(bad)
    };
    let id = |i: usize| int(i).and_then(|v| u16::try_from(v).map_err(|_| bad()));
    match fields[0] {
        "H" | "A" | "P" => Ok(Reply::Echo),
        "h" => {
            if fields.len() < 7 || int(1)? == 0 {
                return Err(bad());
            }
            Ok(Reply::Hello(Limits {
                max_distance: int(2)? as f32,
                max_points: int(3)?,
                max_avoid: int(4)?,
                per_second: int(5)?,
                per_minute: int(6)?,
            }))
        }
        "e" => {
            let refusal = match *fields.get(2).ok_or_else(bad)? {
                "rate" => Refusal::Rate,
                "far" => Refusal::Far,
                "start" => Refusal::Start,
                "nomesh" => Refusal::NoMesh,
                "noroute" => Refusal::NoRoute,
                "bad" => Refusal::Bad,
                "busy" => Refusal::Busy,
                _ => Refusal::Other,
            };
            let detail = match refusal {
                Refusal::Other => fields[2..].join(" "),
                _ => fields[3..].join(" "),
            };
            Ok(Reply::Refused {
                id: id(1)?,
                refusal,
                detail,
            })
        }
        "r" => {
            let flags = *fields.get(4).ok_or_else(bad)?;
            if flags != "-" && !flags.bytes().all(|b| b"cst".contains(&b)) {
                return Err(bad());
            }
            let steps = fields[5..]
                .iter()
                .map(|token| {
                    let mut axes = token.split(',').map(|a| a.parse::<i32>().ok());
                    match (axes.next(), axes.next(), axes.next(), axes.next()) {
                        (Some(Some(x)), Some(Some(y)), Some(Some(z)), None) => Ok([x, y, z]),
                        _ => Err(bad()),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Reply::Part(Part {
                id: id(1)?,
                part: id(2)?,
                parts: id(3)?,
                complete: flags.contains('c'),
                steep: flags.contains('s'),
                truncated: flags.contains('t'),
                steps,
            }))
        }
        _ => Err(bad()),
    }
}

/// Puts a reply's parts back together, in order.
#[derive(Debug, Default)]
pub struct Assembler {
    next: u16,
    last: [i64; 3],
    points: Vec<[f32; 3]>,
}

impl Assembler {
    /// Take the next part: `Ok(Some(route))` with the last one, `Err` for a part out of turn or
    /// a reply past any sane size (the assembler starts over).
    pub fn push(&mut self, part: &Part) -> Result<Option<Route>, String> {
        let in_turn =
            part.part == self.next + 1 && part.part <= part.parts && part.parts <= MAX_PARTS;
        if !in_turn || self.points.len() + part.steps.len() > MAX_POINTS {
            let at = self.next + 1;
            *self = Assembler::default();
            return Err(format!(
                "the realm's route reply came apart (part {} of {} when part {at} was due)",
                part.part, part.parts
            ));
        }
        self.next = part.part;
        for step in &part.steps {
            for (axis, delta) in step.iter().enumerate() {
                self.last[axis] += i64::from(*delta);
            }
            self.points
                .push(self.last.map(|tenths| tenths as f32 / 10.0));
        }
        if part.part < part.parts {
            return Ok(None);
        }
        let points = std::mem::take(&mut self.points);
        *self = Assembler::default();
        if points.is_empty() {
            return Err("the realm's route reply has no corners".into());
        }
        Ok(Some(Route {
            points,
            complete: part.complete && !part.truncated,
            steep: part.steep,
        }))
    }
}

#[cfg(test)]
#[path = "remote_route_tests.rs"]
mod route_tests;
#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
