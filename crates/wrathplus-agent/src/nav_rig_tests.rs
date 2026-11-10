//! The test rig for walks and services the realm answers: the Lua functions, the walker driven by
//! a stand-in for click-to-move, and the realm played by handing its lines to `pump`.

use super::walker::{step, Mover, PUSH};
use super::*;
use mlua::{Lua, Table};
use std::sync::atomic::Ordering;

/// The beta's answer to the probe.
pub(super) const HELLO: &str = "h 1 1500 256 64 5 240 2000 20";

/// Click-to-move, teleporting: each click lands on its corner, unless it is the one walled off.
#[derive(Default)]
pub(super) struct Feet {
    pub(super) at: [f32; 3],
    pub(super) wall: Option<[f32; 3]>,
    pub(super) clicks: Vec<[f32; 3]>,
    /// Clicks refused before click-to-move takes one (autoInteract not through yet).
    pub(super) refuse: u8,
    /// Key chunks run, when there is a VM to run them (`vm`).
    pub(super) vm: bool,
    pub(super) keys: Vec<&'static str>,
}

impl Mover for Feet {
    fn walking(&self) -> bool {
        false
    }
    fn position(&self) -> Vec3 {
        to_bevy(self.at)
    }
    fn walk_to_ground(&mut self, to: Vec3) -> bool {
        if self.refuse > 0 {
            self.refuse -= 1;
            return false;
        }
        let to = to_wow(to);
        self.clicks.push(to);
        if self.wall != Some(to) {
            self.at = to;
        }
        true
    }
    fn keys(&mut self, chunk: &str) -> bool {
        if self.vm {
            self.keys
                .push(if chunk == PUSH { "push" } else { "let go" });
        }
        self.vm
    }
}

pub(super) struct Rig {
    pub(super) nav: Handles,
    pub(super) world: world::Handles,
    pub(super) orders: crossbeam_channel::Receiver<world::Order>,
    pub(super) events: Arc<crate::events::EventLog>,
    pub(super) lua: Lua,
    pub(super) feet: Feet,
}

impl Rig {
    /// An agent at (100, 200, 30) on map 0 with no local mmaps, not yet in the world.
    pub(super) fn new() -> Rig {
        let (world, world_link) = world::link();
        let events = Arc::new(crate::events::EventLog::new());
        let (nav, _) = link(
            events.clone(),
            std::time::Instant::now(),
            world.orders.clone(),
        );
        nav.mesh.lock().unwrap().dir = None;
        world.snapshot.lock().unwrap().me = Some(world::Me {
            pos: [100.0, 200.0, 30.0],
            facing: 0.0,
            map: 0,
        });
        let lua = Lua::new();
        world::install(&lua, &world).unwrap();
        install(&lua, &nav, &world).unwrap();
        install_services(&lua, &nav, &world, &crate::zones::link().0).unwrap();
        Rig {
            nav,
            world,
            orders: world_link.orders,
            events,
            lua,
            feet: Feet {
                at: [100.0, 200.0, 30.0],
                ..Default::default()
            },
        }
    }

    /// In the world, on a realm that answers route requests.
    pub(super) fn in_world() -> Rig {
        let rig = Rig::new();
        assert_eq!(rig.realm(&[]), ["H 1"]);
        assert!(rig.realm(&[HELLO]).is_empty());
        rig
    }

    /// A frame of the realm exchange: the realm says `lines`; what the client sends is returned.
    pub(super) fn realm(&self, lines: &[&str]) -> Vec<String> {
        let lines = lines.iter().map(|l| (l.to_string(), "Me".to_string()));
        pump(&self.nav, lines.collect(), Some("Me"))
    }

    pub(super) fn later(&self, seconds: f64) {
        self.nav
            .ahead
            .fetch_add((seconds * 1000.0) as u32, Ordering::Relaxed);
    }

    pub(super) fn walk_status(&self) -> (String, Option<String>) {
        let walk: Table = self.lua.load("return AgentWalk()").eval().unwrap();
        (walk.get("status").unwrap(), walk.get("note").unwrap())
    }

    pub(super) fn status(&self) -> String {
        self.walk_status().0
    }

    /// The walker until it stops asking for clicks.
    pub(super) fn walk(&mut self) {
        for _ in 0..50 {
            step(&self.nav, &mut self.feet);
        }
    }

    pub(super) fn events(&self) -> Vec<String> {
        let read = self.events.read(0, 100, std::time::Duration::ZERO);
        read.events
            .iter()
            .map(|e| format!("{} {}", e["event"].as_str().unwrap(), e["args"]))
            .collect()
    }
}
