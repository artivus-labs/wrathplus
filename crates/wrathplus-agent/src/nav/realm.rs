//! The realm as a route source (`mod-agent-nav`, over addon messages on the game connection), for
//! a client with no local mmaps. Asking is asynchronous: an [`Ask`] goes out, and its [`Outcome`]
//! comes back from [`Remote::receive`] or [`Remote::tick`] some frames later.
//!
//! The transport is two plain ends, so tests drive it without a server: lines to send collect in
//! [`Remote::outbox`], and lines from the realm are handed to [`Remote::receive`].
//!
//! One request is in flight at a time and requests leave at least [`SPACING`] apart; a newer ask
//! of the same kind replaces one still waiting (a chase asks again as its target moves), so a
//! client never sends more than four a second.

use wrathplus_nav::remote::{self, services, Assembler, Limits, Reply, Request};
use wrathplus_nav::Route;

/// How long the realm has to answer the probe sent on entering the world, and how often it is
/// sent before the realm counts as having no route service.
pub(crate) const PROBE_TIMEOUT: f64 = 5.0;
pub(crate) const PROBES: u8 = 3;
/// How long the realm has to answer a route request.
pub(crate) const REPLY_TIMEOUT: f64 = 8.0;
/// The least time between two requests.
pub(crate) const SPACING: f64 = 0.25;
/// Why services cannot be asked of this realm.
pub(crate) const NO_SERVICES: &str =
    "this realm does not say where services are (it has no mod-agent-nav)";
/// A goal past the realm's distance limit is walked in legs this fraction of the limit long.
const LEG: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Realm {
    /// Not in the world: nothing has been asked.
    Unknown,
    /// The probe is out.
    Probing {
        since: f64,
        tries: u8,
    },
    Available(Limits),
    /// The realm has no route service (it delivered our probe back, or never answered).
    Absent,
}

/// Which navmesh a route comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Local,
    Realm,
    None,
}

/// Local mmaps first (no latency, no load on the realm); the realm when it answers.
pub(crate) fn choose(local: bool, realm: &Realm) -> Source {
    match realm {
        _ if local => Source::Local,
        Realm::Probing { .. } | Realm::Available(_) => Source::Realm,
        Realm::Unknown | Realm::Absent => Source::None,
    }
}

/// What a route is for: a walk (its ticket tells a walk since replaced from the current one) or
/// `AgentPath`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Walk {
        ticket: u32,
    },
    Path,
    /// `AgentServices`: not a route, where services are (`kinds`, `radius`).
    Services,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ask {
    pub purpose: Purpose,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub avoid: Vec<[f32; 3]>,
    /// `to`'s height is a guess (see `nav::heights`).
    pub any_height: bool,
    /// For [`Purpose::Services`]: the kinds asked for (empty: all) and the radius (none: the realm's).
    pub kinds: Vec<String>,
    pub radius: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Route {
        ask: Ask,
        route: Route,
        /// The goal is past the realm's distance limit: this is a leg toward it.
        leg: bool,
    },
    Failed {
        ask: Ask,
        note: String,
    },
    Services {
        ask: Ask,
        places: Vec<services::Place>,
    },
}

struct Flight {
    id: u16,
    sent: f64,
    ask: Ask,
    leg: bool,
    assembler: Assembler,
    collect: services::Collect,
}

pub(crate) struct Remote {
    pub realm: Realm,
    /// Payloads to send, oldest first.
    pub outbox: Vec<String>,
    in_world: bool,
    next_id: u16,
    flight: Option<Flight>,
    /// Asks not sent yet: at most one per kind of purpose.
    waiting: Vec<Ask>,
    last_sent: f64,
}

fn level(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl Remote {
    pub(crate) fn new() -> Remote {
        Remote {
            realm: Realm::Unknown,
            outbox: Vec::new(),
            in_world: false,
            next_id: 0,
            flight: None,
            waiting: Vec::new(),
            last_sent: f64::NEG_INFINITY,
        }
    }

    /// Follow the client into and out of the world: entering probes the realm, leaving forgets it
    /// (the next realm may differ) and whatever was asked.
    pub(crate) fn world(&mut self, in_world: bool, now: f64) -> Vec<Outcome> {
        if in_world == self.in_world {
            return Vec::new();
        }
        self.in_world = in_world;
        self.outbox.clear();
        if in_world {
            self.realm = Realm::Probing {
                since: now,
                tries: 1,
            };
            self.outbox.push(remote::hello());
        } else {
            self.realm = Realm::Unknown;
        }
        let asked = self.flight.take().map(|f| f.ask);
        asked
            .into_iter()
            .chain(std::mem::take(&mut self.waiting))
            .map(|ask| Outcome::Failed {
                ask,
                note: "left the world before the realm's route arrived".into(),
            })
            .collect()
    }

    /// Why there is no route, given what is wrong with the local navmesh (`None`: none is set).
    pub(crate) fn no_navmesh(&self, local: Option<&str>) -> String {
        let local = match local {
            None => "no local one is set (start the client with WOW_NAV_MMAPS=<server data>/mmaps)"
                .to_string(),
            Some(e) => format!("the local one failed ({e}; check WOW_NAV_MMAPS)"),
        };
        let realm = match self.realm {
            Realm::Unknown => "the realm is only asked for routes once in the world",
            _ => "this realm does not answer route requests (it has no mod-agent-nav)",
        };
        format!("no navmesh: {realm}, and {local}")
    }

    /// Queue a route request, replacing one of the same kind still waiting.
    pub(crate) fn ask(&mut self, ask: Ask, now: f64) {
        let kind = std::mem::discriminant(&ask.purpose);
        self.waiting
            .retain(|w| std::mem::discriminant(&w.purpose) != kind);
        self.waiting.push(ask);
        self.flush(now);
    }

    /// Send the next waiting ask, when the realm is known, nothing is in flight and the last
    /// request is [`SPACING`] old. Walks go before `AgentPath`.
    fn flush(&mut self, now: f64) {
        let Realm::Available(limits) = self.realm else {
            return;
        };
        if self.flight.is_some() || now - self.last_sent < SPACING || self.waiting.is_empty() {
            return;
        }
        let at = self
            .waiting
            .iter()
            .position(|w| matches!(w.purpose, Purpose::Walk { .. }))
            .unwrap_or(0);
        let ask = self.waiting.remove(at);
        self.next_id = if self.next_id == u16::MAX {
            1
        } else {
            self.next_id + 1
        };
        let leg = match ask.purpose {
            // Kinds were checked when asked (`services::ask`), so the line is always made.
            Purpose::Services => {
                let line = services::request(self.next_id, &ask.kinds, ask.radius);
                self.outbox.extend(line);
                false
            }
            _ => self.send_route(&ask, limits),
        };
        self.last_sent = now;
        self.flight = Some(Flight {
            id: self.next_id,
            sent: now,
            ask,
            leg,
            assembler: Assembler::default(),
            collect: services::Collect::default(),
        });
    }

    /// A route request for `ask` into the outbox; true when it asks for a leg toward a goal past the
    /// realm's distance limit.
    fn send_route(&mut self, ask: &Ask, limits: Limits) -> bool {
        let span = level(ask.from, ask.to);
        let leg = limits.max_distance > 0.0 && span > limits.max_distance;
        let (to, to_z) = if leg {
            let t = limits.max_distance * LEG / span;
            (
                [
                    ask.from[0] + (ask.to[0] - ask.from[0]) * t,
                    ask.from[1] + (ask.to[1] - ask.from[1]) * t,
                ],
                None,
            )
        } else {
            ([ask.to[0], ask.to[1]], Some(ask.to[2]))
        };
        // The nearest circles, when there are more than the realm takes.
        let mut avoid = ask.avoid.clone();
        avoid.sort_by(|a, b| {
            level([a[0], a[1], 0.0], ask.from).total_cmp(&level([b[0], b[1], 0.0], ask.from))
        });
        avoid.truncate(limits.max_avoid as usize);
        self.outbox.extend(remote::encode(&Request {
            id: self.next_id,
            from: ask.from,
            to,
            to_z,
            near: leg,
            // A leg's end is a point on the way, snapped to the nearest ground: no floor to find.
            any_height: ask.any_height && !leg,
            avoid,
        }));
        leg
    }

    fn fail_waiting(&mut self) -> Vec<Outcome> {
        let note = self.no_navmesh(None);
        std::mem::take(&mut self.waiting)
            .into_iter()
            .map(|ask| Outcome::Failed {
                note: match ask.purpose {
                    Purpose::Services => NO_SERVICES.to_string(),
                    _ => note.clone(),
                },
                ask,
            })
            .collect()
    }

    /// A line from the realm (already known to be our own whisper).
    pub(crate) fn receive(&mut self, payload: &str, now: f64) -> Vec<Outcome> {
        let mut out = Vec::new();
        match remote::decode(payload) {
            Ok(Reply::Hello(limits)) => self.realm = Realm::Available(limits),
            // Our own line delivered back: nobody on the realm answers these.
            Ok(Reply::Echo) => {
                if !matches!(self.realm, Realm::Available(_)) {
                    self.realm = Realm::Absent;
                    out = self.fail_waiting();
                }
            }
            Ok(Reply::Part(part)) => {
                if let Some(mut flight) = self.flight.take_if(|f| f.id == part.id) {
                    match flight.assembler.push(&part) {
                        Ok(None) => self.flight = Some(flight),
                        Ok(Some(mut route)) => {
                            route.complete &= !flight.leg;
                            out.push(Outcome::Route {
                                ask: flight.ask,
                                route,
                                leg: flight.leg,
                            });
                        }
                        Err(note) => out.push(Outcome::Failed {
                            ask: flight.ask,
                            note,
                        }),
                    }
                }
            }
            Ok(Reply::Service(line)) => {
                if let Some(mut flight) = self.flight.take_if(|f| f.id == line.id) {
                    match flight.collect.push(line) {
                        Ok(None) => self.flight = Some(flight),
                        Ok(Some(places)) => out.push(Outcome::Services {
                            ask: flight.ask,
                            places,
                        }),
                        Err(note) => out.push(Outcome::Failed {
                            ask: flight.ask,
                            note,
                        }),
                    }
                }
            }
            Ok(Reply::Refused {
                id,
                refusal,
                detail,
            }) => {
                if let Some(flight) = self.flight.take_if(|f| f.id == id) {
                    out.push(Outcome::Failed {
                        ask: flight.ask,
                        note: refusal.explain(&detail),
                    });
                }
            }
            Err(_) => {}
        }
        self.flush(now);
        out
    }

    /// Time passing: the probe and the request in flight time out, and a waiting ask goes out
    /// once it may.
    pub(crate) fn tick(&mut self, now: f64) -> Vec<Outcome> {
        let mut out = Vec::new();
        match self.realm {
            Realm::Probing { since, tries } if now - since > PROBE_TIMEOUT => {
                if tries < PROBES {
                    // A whisper can be lost in the first moments in the world: ask again.
                    self.realm = Realm::Probing {
                        since: now,
                        tries: tries + 1,
                    };
                    self.outbox.push(remote::hello());
                } else {
                    self.realm = Realm::Absent;
                    out = self.fail_waiting();
                }
            }
            _ => {}
        }
        if let Some(flight) = self.flight.take_if(|f| now - f.sent > REPLY_TIMEOUT) {
            out.push(Outcome::Failed {
                ask: flight.ask,
                note: format!(
                    "the realm did not answer the route request within {REPLY_TIMEOUT:.0} s"
                ),
            });
        }
        self.flush(now);
        out
    }

    /// The source in words, for `/status`.
    pub(crate) fn describe(&self) -> String {
        match self.realm {
            Realm::Available(l) => format!(
                "routes come from the realm (mod-agent-nav): up to {:.0} yd a request, {} requests a second and {} a minute",
                l.max_distance, l.per_second, l.per_minute
            ),
            Realm::Probing { .. } => "asking the realm whether it answers route requests".into(),
            // Before the world, nothing is wrong yet: say what happens on entering it.
            Realm::Unknown => "not in the world yet: on entering it the client asks the realm whether it routes walks (or set WOW_NAV_MMAPS=<server data>/mmaps for local routes)".into(),
            Realm::Absent => self.no_navmesh(None),
        }
    }
}

#[cfg(test)]
#[path = "realm_tests.rs"]
mod tests;
