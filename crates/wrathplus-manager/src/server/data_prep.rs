//! Prepare the server data from the player's own client with AzerothCore's extractors, which the
//! built server image carries. Each tool runs in a one-off container: no network, the client's
//! Data folder mounted read-only, output only into the Manager's state folder. Blizzard files are
//! never packaged or downloaded. Every part records when it finished, so a stopped run continues.
use super::{command, worker};
use crate::setup::{self, Config};
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, String>;

/// One container at a time; a second start while one still runs fails on this name.
const CONTAINER: &str = "wrathplus-server-data";
const TOOLS: &str = "/opt/acore/bin";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// dbc/, maps/ (and Cameras/): map_extractor.
    Maps,
    /// vmaps/: vmap4_extractor, then vmap4_assembler.
    Vmaps,
    /// mmaps/: mmaps_generator, the navigation meshes bots and agents walk on.
    Mmaps,
}

/// How much of the navigation mesh to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Navmesh {
    All,
    Map(u32),
    Tile(u32, u32, u32),
}

impl Navmesh {
    /// In words, for messages.
    pub(crate) fn describe(self) -> String {
        match self {
            Self::All => "the whole world".into(),
            Self::Map(map) => format!("map {map}"),
            Self::Tile(map, x, y) => format!("map {map}, tile {x},{y}"),
        }
    }
    /// `MAP` or `MAP,X,Y`, as the command line takes it.
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let numbers = value
            .split(',')
            .map(|n| n.trim().parse::<u32>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| format!("Navmesh scope {value:?}: use MAP or MAP,X,Y"))?;
        match numbers[..] {
            [map] => Ok(Self::Map(map)),
            [map, x, y] if x < 64 && y < 64 => Ok(Self::Tile(map, x, y)),
            _ => Err(format!(
                "Navmesh scope {value:?}: use MAP or MAP,X,Y (tiles 0..63)"
            )),
        }
    }
}

impl Part {
    fn marker(self) -> &'static str {
        match self {
            Self::Maps => ".wrathplus-maps-done",
            Self::Vmaps => ".wrathplus-vmaps-done",
            Self::Mmaps => ".wrathplus-mmaps-done",
        }
    }
}

/// Where the Manager writes the server data it prepares.
pub(crate) fn folder(config: &Config) -> PathBuf {
    config.state_root().join("managed-server-data")
}

/// Whether `part` finished in `out`.
pub(crate) fn done(out: &Path, part: Part) -> bool {
    out.join(part.marker()).is_file()
}

/// The client's Data folder (the one holding common.MPQ) from the Client page's selection: that
/// folder itself or the install folder around it.
pub(crate) fn client_data(selected: &Path) -> Result<PathBuf> {
    if selected.as_os_str().is_empty() {
        return Err("Choose your WoW 3.3.5a client folder on the Client page first; the server data is made from it.".into());
    }
    let selected = setup::resolved(selected)?;
    [selected.clone(), selected.join("Data")]
        .into_iter()
        .find(|dir| dir.join("common.MPQ").is_file())
        .ok_or_else(|| format!("{} is not a WoW 3.3.5a Data folder (no common.MPQ). Choose the client's Data folder on the Client page.", selected.display()))
}

/// The `docker run` arguments for one tool. The client is read-only, the container has no network,
/// and its files belong to `user` (the output folder's owner) where the platform has owners.
pub(super) fn run_args(
    data: &Path,
    out: &Path,
    user: Option<(u32, u32)>,
    tool: &str,
    args: &[&str],
) -> Result<Vec<OsString>> {
    let mount = |path: &Path, target: &str, readonly: bool| -> Result<OsString> {
        let text = path.to_string_lossy();
        // --mount is comma-separated; a comma or quote in the path would change its meaning.
        if text.contains([',', '"']) {
            return Err(format!(
                "{} contains a comma or quote, which Docker cannot mount. Move or rename the folder.",
                path.display()
            ));
        }
        let mut value = OsString::from("type=bind,source=");
        value.push(path);
        value.push(format!(",target={target}"));
        if readonly {
            value.push(",readonly");
        }
        Ok(value)
    };
    let mut argv: Vec<OsString> = [
        "run",
        "--rm",
        "--name",
        CONTAINER,
        "--label",
        "wrathplus.manager=server-data",
        "--network",
        "none",
        "--platform",
        "linux/arm64",
        "--workdir",
        "/server-data",
    ]
    .map(OsString::from)
    .into();
    if let Some((uid, gid)) = user {
        argv.extend(["--user".into(), format!("{uid}:{gid}").into()]);
    }
    argv.extend([
        "--mount".into(),
        mount(data, "/client/Data", true)?,
        "--mount".into(),
        mount(out, "/server-data", false)?,
        "--entrypoint".into(),
        format!("{TOOLS}/{tool}").into(),
        command::IMAGE.into(),
    ]);
    argv.extend(args.iter().map(OsString::from));
    Ok(argv)
}

/// Each tool and its arguments for a part, in order.
pub(super) fn tools(part: Part, scope: Navmesh) -> Vec<(&'static str, Vec<String>)> {
    match part {
        Part::Maps => vec![(
            "map_extractor",
            ["-i", "/client", "-o", "/server-data"]
                .map(String::from)
                .into(),
        )],
        Part::Vmaps => vec![
            (
                "vmap4_extractor",
                ["-d", "/client/Data"].map(String::from).into(),
            ),
            (
                "vmap4_assembler",
                ["Buildings", "vmaps"].map(String::from).into(),
            ),
        ],
        Part::Mmaps => {
            let mut args = vec!["--config".into(), format!("{TOOLS}/mmaps-config.yaml")];
            match scope {
                Navmesh::All => {}
                Navmesh::Map(map) => args.push(map.to_string()),
                Navmesh::Tile(map, x, y) => {
                    args.extend([map.to_string(), "--tile".into(), format!("{x},{y}")])
                }
            }
            vec![("mmaps_generator", args)]
        }
    }
}

/// A part's output is there, not just its tool's exit code: map_extractor exits 0 even when it
/// finds no client locale.
pub(super) fn produced(out: &Path, part: Part) -> Result<()> {
    let filled = |dir: &str, extension: &str| {
        std::fs::read_dir(out.join(dir)).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case(extension))
            })
        })
    };
    let missing = match part {
        Part::Maps => [("dbc", "dbc"), ("maps", "map")]
            .into_iter()
            .find(|(dir, ext)| !filled(dir, ext)),
        Part::Vmaps => (!filled("vmaps", "vmtree")).then_some(("vmaps", "vmtree")),
        Part::Mmaps => (!filled("mmaps", "mmtile")).then_some(("mmaps", "mmtile")),
    };
    match missing {
        None => Ok(()),
        Some((dir, ext)) => Err(format!(
            "The extractor finished but wrote no .{ext} files into {dir}/. Check that the client folder is a complete 3.3.5a (12340) install with its locale folder, then see the saved log."
        )),
    }
}

/// Run the parts not yet finished, in order, logging every tool's output. `parts` is
/// Maps+Vmaps for the realm's data, or Mmaps for navigation meshes.
pub(super) fn run(
    config: &Config,
    parts: &[Part],
    scope: Navmesh,
    log: &mut File,
    stage: &dyn Fn(String),
) -> Result<PathBuf> {
    let data = client_data(&config.data)?;
    let mut guard = config.clone();
    guard.data = data.clone();
    let out = folder(config);
    setup::check_output(&guard, &out)?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let out = setup::resolved(&out)?;
    if parts.contains(&Part::Mmaps) && !(done(&out, Part::Maps) && done(&out, Part::Vmaps)) {
        return Err("Prepare the server data first; navigation meshes are made from it.".into());
    }
    let image = worker::capture(command::local_image(), log)?;
    if image.trim().is_empty() {
        return Err("Build the server first: its image carries AzerothCore's map tools.".into());
    }
    let user = owner(&out);
    for (index, part) in parts.iter().enumerate() {
        if done(&out, *part) && (*part != Part::Mmaps || scope == Navmesh::All) {
            writeln!(log, "--- {part:?} already prepared ---").map_err(|e| e.to_string())?;
            continue;
        }
        stage(match part {
            Part::Maps => format!(
                "Step {} of {}: reading maps and game tables from your client",
                index + 1,
                parts.len()
            ),
            Part::Vmaps => format!(
                "Step {} of {}: building line-of-sight data · a few minutes",
                index + 1,
                parts.len()
            ),
            Part::Mmaps => {
                "Making navigation meshes · several hours; it continues where it stopped".into()
            }
        });
        if *part == Part::Vmaps {
            // An interrupted extraction leaves a partial Buildings/ the tool refuses to reuse.
            remove_own(&out, "Buildings")?;
        }
        for (tool, args) in tools(*part, scope) {
            writeln!(log, "\n--- {tool} {} ---", args.join(" ")).map_err(|e| e.to_string())?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut docker = Command::new("docker");
            docker.args(run_args(&data, &out, user, tool, &args)?);
            worker::execute(docker, log)
                .map_err(|e| format!("{tool}: {e} Press the button again to continue."))?;
        }
        produced(&out, *part)?;
        if *part == Part::Vmaps {
            // Intermediate files only; the server and mmaps_generator read vmaps/.
            remove_own(&out, "Buildings")?;
        }
        // A whole-world navmesh is the finished state; a single map or tile is a partial run.
        if *part != Part::Mmaps || scope == Navmesh::All {
            std::fs::write(out.join(part.marker()), b"done\n").map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// Delete one intermediate folder the extractors made inside the Manager's own output folder.
fn remove_own(out: &Path, name: &str) -> Result<()> {
    let path = out.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path).map_err(|e| e.to_string()),
        Ok(_) => Err(format!(
            "{} is not a folder the extractor made; move it away and try again.",
            path.display()
        )),
        Err(_) => Ok(()),
    }
}

#[cfg(unix)]
fn owner(out: &Path) -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(out).ok().map(|m| (m.uid(), m.gid()))
}
#[cfg(not(unix))]
fn owner(_: &Path) -> Option<(u32, u32)> {
    None
}

/// Progress from the tools' own output: map_extractor's "Extract <map> (n/total)" and
/// mmaps_generator's "NN% [Map …] Building tile". The line-of-sight tools print no total.
pub(crate) fn progress(log: &str) -> Option<(u64, u64)> {
    log.rsplit(['\n', '\r']).find_map(|line| {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Extract ") {
            let pair = rest.rsplit_once('(')?.1.split(')').next()?;
            let (done, total) = pair.split_once('/')?;
            let (done, total) = (done.parse().ok()?, total.parse().ok()?);
            return (total > 0 && done <= total).then_some((done, total));
        }
        let (percent, rest) = line.split_once("% [Map ")?;
        let percent = percent.parse().ok()?;
        (rest.contains("Building tile") && percent <= 100).then_some((percent, 100))
    })
}

#[cfg(test)]
#[path = "data_prep_tests.rs"]
mod tests;
