//! Where services are, asked of the realm (`mod-agent-nav`'s `AgentNavServices.h` states the
//! format): the request line, and the realm's answer one place a line.

/// The kinds a request may name.
pub const KINDS: [&str; 13] = [
    "vendor",
    "repair",
    "trainer",
    "flightmaster",
    "innkeeper",
    "banker",
    "questgiver-hub",
    "trainer_cooking",
    "trainer_first_aid",
    "trainer_profession",
    "auctioneer",
    "mailbox",
    "portal",
];

/// A services request: `kinds` (from [`KINDS`]; empty: all of them) within `radius` yards (none:
/// the realm's own limit). `Err` names an unknown kind.
pub fn request(id: u16, kinds: &[String], radius: Option<u32>) -> Result<String, String> {
    if let Some(unknown) = kinds.iter().find(|k| !KINDS.contains(&k.as_str())) {
        return Err(format!(
            "unknown service kind {unknown:?} (known: {})",
            KINDS.join(", ")
        ));
    }
    let list = if kinds.is_empty() {
        KINDS.join(",")
    } else {
        kinds.join(",")
    };
    Ok(match radius {
        Some(r) => format!("S {id} {list} {r}"),
        None => format!("S {id} {list}"),
    })
}

/// One place the realm named.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// One of [`KINDS`].
    pub kind: String,
    /// The creature's template entry; a mailbox's gameobject entry.
    pub entry: u32,
    pub name: String,
    /// WoW coordinates, on the asker's map.
    pub pos: [f32; 3],
    /// The quest givers in a hub; 1 for anything else.
    pub members: u32,
    /// The profession a profession trainer teaches ("First Aid"); `None` for anything else.
    pub profession: Option<String>,
    /// A spawn a game event brings out (`game_event_creature`): `Some(true)` while one of its
    /// events has it out, as they stood when the realm answered, `Some(false)` while none does;
    /// `None` for a standing spawn.
    pub event: Option<bool>,
    /// The auction house an auctioneer serves: `"alliance"`, `"horde"` or `"neutral"` (Booty Bay,
    /// Gadgetzan, Everlook; every one on a realm that lets both sides trade); `None` for anything
    /// else. The other side's auctioneers are never named: the realm leaves out what is hostile.
    pub house: Option<String>,
    /// A portal's volume, destination and requirement; `None` for any other kind.
    pub portal: Option<Portal>,
}

/// A portal (`AgentNavPortals.h`): an area trigger that teleports, used by walking into its
/// volume (the client sends `CMSG_AREATRIGGER` on entering it). The place's `pos` is its centre,
/// its `name` the destination map's, its `entry` the `AreaTrigger.dbc` id.
#[derive(Debug, Clone, PartialEq)]
pub struct Portal {
    pub area: Area,
    /// The destination map (`Map.dbc` id) and where on it, WoW coordinates.
    pub to_map: u32,
    pub to: [f32; 3],
    /// The levels the core asks for, as the realm reads them (none when it ignores instance
    /// levels); told, not filtered.
    pub level_min: Option<u32>,
    pub level_max: Option<u32>,
    /// What else it asks, of the asker's side at its difficulty: `"raid"` (a raid group),
    /// `"quest:<id>"`, `"item:<id>"`, `"achievement:<id>"`, `"item_level:<n>"`.
    pub needs: Vec<String>,
}

/// A trigger's volume, centred on the place's `pos`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Area {
    Sphere {
        radius: f32,
    },
    /// Full extents (length, width, height) in yards and the yaw in radians, as `AreaTrigger.dbc`.
    Box {
        size: [f32; 3],
        yaw: f32,
    },
}

fn number(word: &str) -> Option<f32> {
    word.parse().ok().filter(|v: &f32| v.is_finite())
}

fn area(word: &str) -> Option<Area> {
    if let Some((size, yaw)) = word.split_once('@') {
        let mut parts = size.split('x').map(number);
        let size = [parts.next()??, parts.next()??, parts.next()??];
        return parts.next().is_none().then_some(Area::Box {
            size,
            yaw: number(yaw)?,
        });
    }
    Some(Area::Sphere {
        radius: number(word)?,
    })
}

/// `<min>`, `<min>-<max>` or `-<max>` (the "-" of none is read before).
fn levels(word: &str) -> Option<(Option<u32>, Option<u32>)> {
    let (min, max) = word.split_once('-').unwrap_or((word, ""));
    let read = |w: &str| -> Option<Option<u32>> {
        if w.is_empty() {
            Some(None)
        } else {
            w.parse().ok().map(Some)
        }
    };
    let out = (read(min)?, read(max)?);
    (out != (None, None)).then_some(out)
}

/// The four portal fields (area, to, level, needs): `Ok(None)` when all are "-" (not a portal),
/// `Err(())` when they do not read or do not match `kind`.
fn portal(kind: &str, fields: [&str; 4]) -> Result<Option<Portal>, ()> {
    if kind != "portal" {
        return if fields == ["-"; 4] {
            Ok(None)
        } else {
            Err(())
        };
    }
    let [area_word, to, level, needs] = fields;
    let area = area(area_word).ok_or(())?;
    let mut to = to.split(',');
    let to_map = to.next().and_then(|m| m.parse().ok()).ok_or(())?;
    let mut pos = [0f32; 3];
    for axis in &mut pos {
        *axis = to.next().and_then(number).ok_or(())?;
    }
    if to.next().is_some() {
        return Err(());
    }
    let (level_min, level_max) = match level {
        "-" => (None, None),
        word => levels(word).ok_or(())?,
    };
    let needs = match needs {
        "-" => Vec::new(),
        list => list.split(',').map(str::to_string).collect(),
    };
    if needs.iter().any(|n| n.is_empty()) {
        return Err(());
    }
    Ok(Some(Portal {
        area,
        to_map,
        to: pos,
        level_min,
        level_max,
        needs,
    }))
}

/// One line of the realm's answer: the `index`-th of `count` places (`None` with count 0: none).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub id: u16,
    pub index: u16,
    pub count: u16,
    pub place: Option<Place>,
}

/// An `s` line (the verb already read), or `None` when it does not parse.
pub(super) fn decode(payload: &str) -> Option<Line> {
    let mut fields = payload.splitn(18, ' ');
    let _verb = fields.next()?;
    let id = fields.next()?.parse().ok()?;
    let index: u16 = fields.next()?.parse().ok()?;
    let count: u16 = fields.next()?.parse().ok()?;
    if count == 0 {
        return (index == 0 && fields.next().is_none()).then_some(Line {
            id,
            index,
            count,
            place: None,
        });
    }
    let kind = fields.next()?;
    if index == 0 || index > count || !KINDS.contains(&kind) {
        return None;
    }
    let entry = fields.next()?.parse().ok()?;
    let mut pos = [0f32; 3];
    for axis in &mut pos {
        *axis = fields
            .next()?
            .parse()
            .ok()
            .filter(|v: &f32| v.is_finite())?;
    }
    let members = fields.next()?.parse().ok()?;
    let profession = match fields.next()? {
        "-" => None,
        word => Some(word.replace('_', " ")),
    };
    let event = match fields.next()? {
        "-" => None,
        "on" => Some(true),
        "off" => Some(false),
        _ => return None,
    };
    let house = match fields.next()? {
        "-" => None,
        word @ ("alliance" | "horde" | "neutral") => Some(word.to_string()),
        _ => return None,
    };
    let portal = portal(
        kind,
        [
            fields.next()?,
            fields.next()?,
            fields.next()?,
            fields.next()?,
        ],
    )
    .ok()?;
    let name = fields.next().unwrap_or("").trim().to_string();
    Some(Line {
        id,
        index,
        count,
        place: Some(Place {
            kind: kind.to_string(),
            entry,
            name,
            pos,
            members,
            profession,
            event,
            house,
            portal,
        }),
    })
}

/// Puts an answer's lines back together, in order.
#[derive(Debug, Default)]
pub struct Collect {
    places: Vec<Place>,
}

impl Collect {
    /// Take the next line: `Ok(Some(places))` with the last one, `Err` for a line out of turn.
    pub fn push(&mut self, line: Line) -> Result<Option<Vec<Place>>, String> {
        let Some(place) = line.place else {
            return Ok(Some(Vec::new()));
        };
        if usize::from(line.index) != self.places.len() + 1 {
            let due = self.places.len() + 1;
            self.places.clear();
            return Err(format!(
                "the realm's services answer came apart (place {} of {} when {due} was due)",
                line.index, line.count
            ));
        }
        self.places.push(place);
        Ok((line.index == line.count).then(|| std::mem::take(&mut self.places)))
    }
}

#[cfg(test)]
#[path = "remote_services_tests.rs"]
mod tests;
