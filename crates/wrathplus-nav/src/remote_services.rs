//! Where services are, asked of the realm (`mod-agent-nav`'s `AgentNavServices.h` states the
//! format): the request line, and the realm's answer one place a line.

/// The kinds a request may name.
pub const KINDS: [&str; 10] = [
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
    /// The creature's template entry.
    pub entry: u32,
    pub name: String,
    /// WoW coordinates, on the asker's map.
    pub pos: [f32; 3],
    /// The quest givers in a hub; 1 for anything else.
    pub members: u32,
    /// The profession a profession trainer teaches ("First Aid"); `None` for anything else.
    pub profession: Option<String>,
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
    let mut fields = payload.splitn(12, ' ');
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
