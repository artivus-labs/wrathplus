//! `AddOn_CanLoad` (`0x5f7e90`) as a pure function: the one load law every surface consults.
//! The first check that fires decides, in the reference's order: missing, the visiting guard (a
//! cycle is loadable), enabled, banned, corrupt, incompatible, version, dependencies, demand.
//!
//! An `## Interface` below 20000 is `INCOMPATIBLE` whatever `checkAddonVersion` says
//! (`0x5f7f1a`); otherwise the version gate is an exact `==` against 30300 (`0x5f7f32`), so 30301
//! is out of date. With `checkAddonVersion` off the reason is reset to none (`0x5f7f56`), so
//! force-load falls through to the dependency and demand checks.
//!
//! The reason renders once (`0x5f4c40`): the addon's own token, else `DEP_` and the deepest
//! failing dependency's token, else nil. `BANNED`, `CORRUPT` and `INSECURE` read the server's
//! `SMSG_ADDON_INFO` signature state, which is not modelled; they are never produced.

/// The `## Interface` build 12340 implements.
pub const CLIENT_INTERFACE: u32 = 30300;

/// One addon as the gate reads it; every addon registry lowers into this.
pub struct GateRow<'a> {
    pub name: &'a str,
    /// This character's enable state (`AddOns.txt`; an addon nobody disabled is enabled).
    pub enabled: bool,
    /// `## Interface` as the client parses it ([`crate::toc::Toc::interface_version`]):
    /// the leading integer, `0` when absent.
    pub interface: u32,
    pub load_on_demand: bool,
    /// Loaded this session. A loaded dependency satisfies the recursion outright (`0x5f7f7f`);
    /// any other recurses with the demand check off (`0x5f7f9c` pushes 1).
    pub loaded: bool,
    /// `## Dependencies` / `## RequiredDeps` / any `## Dep*` (one list in the reference).
    pub dependencies: Vec<&'a str>,
}

/// The arbiter's answer: loadable, or the two out-params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Loadable,
    Refused {
        /// The addon's own reason token (`DISABLED`, `INTERFACE_VERSION`, …).
        reason: Option<&'static str>,
        /// A failing dependency's raw token, rendered as `DEP_<token>`.
        dep: Option<&'static str>,
    },
}

impl Verdict {
    pub fn loadable(self) -> bool {
        self == Verdict::Loadable
    }

    /// The formatter (`0x51e930`) with its callers' nil guard, so `LOADABLE` never renders.
    pub fn token(self) -> Option<String> {
        match self {
            Verdict::Loadable => None,
            Verdict::Refused { reason, dep } => reason
                .map(str::to_owned)
                .or_else(|| dep.map(|d| format!("DEP_{d}"))),
        }
    }
}

/// `AddOn_CanLoad` over a registry: can `rows[index]` load right now?
///
/// `demand_check` is the gate's `dl == 0` (the startup load and the glue's `GetAddOnInfo`,
/// `0x4dc65a`): a LoadOnDemand addon then reports `DEMAND_LOADED` (`0x5f7fc8`). Every in-game
/// verb passes `dl = 1` (`0x51138b`, `0x528a6d`) and has no demand check at all; `version_check`
/// is the live `checkAddonVersion`, read per query.
pub fn can_load(
    rows: &[GateRow],
    index: usize,
    demand_check: bool,
    version_check: bool,
) -> Verdict {
    can_load_for_interface(rows, index, demand_check, version_check, CLIENT_INTERFACE)
}

/// [`can_load`] against an explicit client interface.
pub(crate) fn can_load_for_interface(
    rows: &[GateRow],
    index: usize,
    demand_check: bool,
    version_check: bool,
    client_interface: u32,
) -> Verdict {
    let mut visiting = vec![false; rows.len()];
    walk(
        rows,
        index,
        demand_check,
        version_check,
        client_interface,
        &mut visiting,
    )
}

/// One level of the arbiter, its checks in the reference's order.
fn walk(
    rows: &[GateRow],
    i: usize,
    demand_check: bool,
    version_check: bool,
    client_interface: u32,
    visiting: &mut [bool],
) -> Verdict {
    // Check 2: the visiting guard; a cycle resolves as loadable.
    if visiting[i] {
        return Verdict::Loadable;
    }
    let row = &rows[i];
    // Check 3: enabled.
    if !row.enabled {
        return Verdict::Refused {
            reason: Some("DISABLED"),
            dep: None,
        };
    }
    // Checks 4 and 5, banned and corrupt: no signature state, never produced.
    // Below 20000 is incompatible, with no force-load (`0x5f7f1a`).
    if row.interface < 20000 {
        return Verdict::Refused {
            reason: Some("INCOMPATIBLE"),
            dep: None,
        };
    }
    // Check 6: the version gate, exact `==`; with the check off it falls through (`0x5f7f4c`).
    if row.interface != client_interface && version_check {
        return Verdict::Refused {
            reason: Some("INTERFACE_VERSION"),
            dep: None,
        };
    }
    // Check 7: the dependencies, recursively; a shared out-param makes the deepest failure's raw
    // token the one `DEP_` wraps (`0x5f7f94`).
    visiting[i] = true;
    for dep in &row.dependencies {
        let found = rows.iter().position(|r| r.name.eq_ignore_ascii_case(dep));
        let verdict = match found {
            // Check 1, in the recursion: a name not in the registry is `MISSING`.
            None => Verdict::Refused {
                reason: Some("MISSING"),
                dep: None,
            },
            // A loaded dependency satisfies the walk before any recursion (`0x5f7f89`).
            Some(d) if rows[d].loaded => Verdict::Loadable,
            Some(d) => walk(rows, d, false, version_check, client_interface, visiting),
        };
        if let Verdict::Refused { reason, dep } = verdict {
            visiting[i] = false;
            return Verdict::Refused {
                reason: None,
                dep: reason.or(dep),
            };
        }
    }
    visiting[i] = false;
    // Check 8: the demand gate, `dl == 0` only (`0x5f7fc8`).
    if demand_check && row.load_on_demand {
        return Verdict::Refused {
            reason: Some("DEMAND_LOADED"),
            dep: None,
        };
    }
    Verdict::Loadable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(name: &'a str, deps: Vec<&'a str>) -> GateRow<'a> {
        GateRow {
            name,
            enabled: true,
            interface: CLIENT_INTERFACE,
            load_on_demand: false,
            loaded: false,
            dependencies: deps,
        }
    }

    /// Enabled comes before every later check, so the glue shows a disabled row grey, never red.
    #[test]
    fn a_disabled_addon_reports_only_disabled() {
        let mut a = row("A", vec!["Ghost"]);
        a.enabled = false;
        a.interface = 0; // out of date too
        let rows = vec![a];
        assert_eq!(
            can_load(&rows, 0, false, true).token().as_deref(),
            Some("DISABLED")
        );
    }

    #[test]
    fn the_version_gate_is_exact_and_force_load_erases_it() {
        let mut a = row("A", vec![]);
        a.interface = 30200;
        let rows = vec![a];
        assert_eq!(
            can_load(&rows, 0, false, true).token().as_deref(),
            Some("INTERFACE_VERSION")
        );
        assert_eq!(can_load(&rows, 0, false, false), Verdict::Loadable);

        // Below 20000, a missing `## Interface` (0) included, force-load does not help
        // (`0x5f7f1a`).
        for interface in [11200, 0] {
            let mut b = row("B", vec![]);
            b.interface = interface;
            let rows = vec![b];
            for version_check in [true, false] {
                assert_eq!(
                    can_load(&rows, 0, false, version_check).token().as_deref(),
                    Some("INCOMPATIBLE"),
                    "## Interface: {interface}"
                );
            }
        }
    }

    #[test]
    fn dep_prefix_applies_once_at_any_depth() {
        let a = row("A", vec!["B"]);
        let b = row("B", vec!["C"]);
        let mut c = row("C", vec![]);
        c.enabled = false;
        let rows = vec![a, b, c];
        assert_eq!(
            can_load(&rows, 0, false, true).token().as_deref(),
            Some("DEP_DISABLED"),
            "not DEP_DEP_DISABLED — the shared out-param collapses the nesting"
        );
        // A dependency not installed fails the recursion's check 1.
        let rows = vec![row("A", vec!["Ghost"])];
        assert_eq!(
            can_load(&rows, 0, false, true).token().as_deref(),
            Some("DEP_MISSING")
        );
    }

    #[test]
    fn a_cycle_is_loadable_to_the_query() {
        let a = row("Ping", vec!["Pong"]);
        let b = row("Pong", vec!["Ping"]);
        let rows = vec![a, b];
        assert_eq!(can_load(&rows, 0, false, true), Verdict::Loadable);
    }

    /// In game (`dl = 1`) an enabled, unloaded addon is loadable, LoadOnDemand or not; only the
    /// startup query (`dl = 0`) refuses a LoadOnDemand one, and never its dependencies (`0x5f7f9c`).
    #[test]
    fn the_demand_gate_is_the_startup_querys_alone() {
        let a = row("A", vec![]);
        let mut b = row("B", vec!["C"]);
        b.load_on_demand = true;
        let mut c = row("C", vec![]);
        c.load_on_demand = true;
        let rows = vec![a, b, c];
        for i in 0..3 {
            assert_eq!(
                can_load(&rows, i, false, true),
                Verdict::Loadable,
                "row {i}"
            );
        }
        assert_eq!(can_load(&rows, 0, true, true), Verdict::Loadable);
        assert_eq!(
            can_load(&rows, 1, true, true).token().as_deref(),
            Some("DEMAND_LOADED"),
            "and not DEP_DEMAND_LOADED: the recursion passes dl = 1"
        );
    }

    /// A loaded dependency passes before its own checks run (`0x5f7f89`): disabling it mid-session
    /// does not refuse its dependent.
    #[test]
    fn a_loaded_dependency_satisfies_the_query() {
        let mut a = row("A", vec!["B"]);
        a.load_on_demand = true;
        let mut b = row("B", vec![]);
        b.enabled = false;
        b.loaded = true;
        let rows = vec![a, b];
        assert_eq!(can_load(&rows, 0, false, true), Verdict::Loadable);
    }
}
