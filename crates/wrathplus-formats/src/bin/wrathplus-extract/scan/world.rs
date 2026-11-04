//! Corpus scans over placed world content: a WMO root's own tables and an ADT region's placements.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use wrathplus_formats::Chain;

use crate::model_key;

/// Dump a WMO root's MODD props with their MODS sets and owning groups, read from the group files'
/// MODR lists. The reference creates a prop only from a visible group's refs (`0x695aa0`), so it
/// never draws a prop no group names (`ORPHAN`); WrathPlus spawns one with no room gate.
pub fn wmodoodads(chain: &mut Chain, raw_path: &str, filter: Option<&str>) -> Result<()> {
    let root_path = raw_path.replace('/', "\\").to_ascii_lowercase();
    let bytes = chain
        .read_file(&root_path)
        .with_context(|| format!("reading WMO root '{root_path}'"))?;
    let root = wrathplus_formats::parse_wmo_root(&bytes)
        .with_context(|| format!("parsing WMO root '{root_path}'"))?;
    let set_names = mods_set_names(&bytes);

    // MODD index -> the groups whose MODR names it.
    let stem = root_path.strip_suffix(".wmo").unwrap_or(&root_path);
    let mut owners: BTreeMap<u16, Vec<u32>> = BTreeMap::new();
    let mut groups_read = 0u32;
    for gi in 0..root.group_count() {
        let group_path = format!("{stem}_{gi:03}.wmo");
        let Ok(gbytes) = chain.read_file(&group_path) else {
            continue;
        };
        groups_read += 1;
        for r in wrathplus_formats::wmo_group_doodad_refs(&gbytes) {
            owners.entry(r).or_default().push(gi);
        }
    }

    println!(
        "{} doodad(s), {} set(s), {} group(s) ({} group file(s) read)",
        root.doodads().len(),
        root.doodad_sets().len(),
        root.group_count(),
        groups_read,
    );
    for (si, s) in root.doodad_sets().iter().enumerate() {
        let name = set_names.get(si).map(String::as_str).unwrap_or("?");
        println!(
            "set {si:>2}  [{:>5}..{:>5})  count {:>5}  {name}",
            s.start,
            s.start + s.count,
            s.count
        );
    }

    let needle = filter.map(str::to_ascii_lowercase);
    let infos = root.group_infos();
    let mut shown = 0u32;
    let mut orphans_total = 0u32;
    let mut orphans_shown = 0u32;
    for (i, d) in root.doodads().iter().enumerate() {
        let orphan = !owners.contains_key(&(i as u16));
        if orphan {
            orphans_total += 1;
        }
        if let Some(n) = &needle {
            if !model_key(&d.model).contains(n.as_str()) {
                continue;
            }
        }
        shown += 1;
        if orphan {
            orphans_shown += 1;
        }
        let sets: Vec<String> = root
            .doodad_sets()
            .iter()
            .enumerate()
            .filter(|(_, s)| (s.start..s.start + s.count).contains(&(i as u32)))
            .map(|(si, _)| si.to_string())
            .collect();
        let owner_cell = match owners.get(&(i as u16)) {
            Some(gs) => gs
                .iter()
                .map(|&g| {
                    let class = match infos.get(g as usize) {
                        Some(gi) if gi.interior => "INT",
                        Some(_) => "EXT",
                        None => "?",
                    };
                    format!("g{g}({class})")
                })
                .collect::<Vec<_>>()
                .join(" "),
            None => "ORPHAN".into(),
        };
        println!(
            "modd {i:>5}  pos ({:>8.2}, {:>8.2}, {:>8.2})  scale {:.3}  color #{:02x}{:02x}{:02x}{:02x}  sets [{}]  {owner_cell}  {}",
            d.position[0], d.position[1], d.position[2],
            d.scale,
            d.color[0], d.color[1], d.color[2], d.color[3],
            sets.join(","),
            d.model,
        );
    }
    match needle {
        Some(_) => eprintln!(
            "{shown} doodad(s) matched ({orphans_shown} ORPHAN); {orphans_total} orphan(s) among all {}",
            root.doodads().len()
        ),
        None => eprintln!(
            "{orphans_total} of {} doodad(s) are ORPHANS (in no group's MODR — the reference never instantiates these)",
            root.doodads().len()
        ),
    }
    Ok(())
}

/// The MODS set names, `char name[20]` at the head of each 32-byte record, read off the raw root
/// (chunk magics are reversed on disk: `SDOM`) since `WmoDoodadSet` keeps only the ranges.
fn mods_set_names(bytes: &[u8]) -> Vec<String> {
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let size = u32::from_le_bytes([
            bytes[off + 4],
            bytes[off + 5],
            bytes[off + 6],
            bytes[off + 7],
        ]) as usize;
        let Some(data_end) = (off + 8).checked_add(size) else {
            return Vec::new();
        };
        if data_end > bytes.len() {
            return Vec::new();
        }
        if &bytes[off..off + 4] == b"SDOM" {
            return bytes[off + 8..data_end]
                .as_chunks::<32>()
                .0
                .iter()
                .map(|rec| {
                    let name = &rec[..20];
                    let end = name.iter().position(|&b| b == 0).unwrap_or(20);
                    String::from_utf8_lossy(&name[..end]).into_owned()
                })
                .collect();
        }
        off = data_end;
    }
    Vec::new()
}

/// Cross-tab every WMO root's MOSB skybox model against its groups flagged `0x40000`
/// ([`wrathplus_formats::WmoGroupInfo::show_skybox`]): across the 815 roots the flag never appears
/// without a MOSB. Which group counts is not in the assets: the reference tests the bit on each
/// group its portal flood visits (`0x6b42e0` in `0x6b41c0`). Only Stratholme's shell (61 of 83
/// groups) and the four unused Caverns of Time shells set it.
pub fn skyboxscan(chain: &mut Chain) -> Result<()> {
    let names = super::wmo_roots(chain, None)?;

    let (mut both, mut mosb_only, mut flag_only, mut neither) = (0u32, 0u32, 0u32, 0u32);
    let mut scanned = 0u32;
    let mut hits: Vec<(String, String, usize, usize)> = Vec::new();
    for name in names {
        let Ok(bytes) = chain.read_file(&name) else {
            continue;
        };
        let Ok(root) = wrathplus_formats::parse_wmo_root(&bytes) else {
            continue; // a group file that slipped the name filter, or a truncated root
        };
        scanned += 1;
        let groups = root.group_infos();
        let flagged = groups.iter().filter(|g| g.show_skybox).count();
        match (root.skybox(), flagged > 0) {
            (Some(sky), true) => {
                both += 1;
                hits.push((name, sky.to_string(), flagged, groups.len()));
            }
            (Some(sky), false) => {
                mosb_only += 1;
                hits.push((
                    name,
                    format!("{sky}  (NO group sets 0x40000)"),
                    0,
                    groups.len(),
                ));
            }
            (None, true) => {
                flag_only += 1;
                hits.push((name, "(no MOSB)".into(), flagged, groups.len()));
            }
            (None, false) => neither += 1,
        }
    }

    hits.sort();
    println!("{:<62} {:>7}  skybox model", "WMO root", "groups");
    for (name, sky, flagged, total) in &hits {
        println!("{name:<62} {flagged:>3}/{total:<3}  {sky}");
    }
    println!();
    println!("{scanned} WMO root(s) scanned");
    println!("  MOSB skybox AND >=1 group with 0x40000 : {both}");
    println!("  MOSB skybox but NO group with 0x40000  : {mosb_only}");
    println!("  group(s) with 0x40000 but NO MOSB      : {flag_only}");
    println!("  neither                                : {neither}");
    if flag_only == 0 {
        println!(
            "\n=> 0x40000 NEVER appears without a MOSB ({flag_only} counter-examples in {scanned} \
             roots), and {mosb_only} root(s) name a skybox no group asks for. So the bit is real and \
             both halves matter — but this census establishes only 'flag implies MOSB'."
        );
        println!(
            "   It does NOT say WHICH group the renderer tests, and reading it as if it did is a \
             mistake. The actual law: 0x40000 is tested \
             inside the portal flood (0x6b42e0 in 0x6b41c0) on the group being VISITED, never on the \
             group the camera stands in — so the predicate is 'any FLOOD-REACHED group carries the \
             bit, and the root names a MOSB'. Stratholme's King's Square is the counter-example that \
             settles it: the camera's own group (39) is EXTERIOR and unflagged, and the reference \
             paints the sky there anyway."
        );
    } else {
        println!(
            "\n=> {flag_only} group(s) set 0x40000 with no MOSB to draw — that would break even the \
             weak 'flag implies MOSB' reading this census rests on; re-derive before building on it."
        );
    }
    Ok(())
}

/// List the MDDF and MODF placements near a world position whose model path contains `filter`.
pub fn placescan(
    chain: &mut Chain,
    map: &str,
    center_x: f32,
    center_y: f32,
    tile_radius: u32,
    filter: &str,
) -> Result<()> {
    let tiles =
        wrathplus_formats::load_tiles_around(chain, map, center_x, center_y, tile_radius)
            .with_context(|| format!("loading tiles around ({center_x}, {center_y}) on {map}"))?;
    eprintln!("{} tile(s) loaded", tiles.len());
    let needle = filter.to_ascii_lowercase();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut hits = 0u32;
    for (_, tile) in &tiles {
        for d in &tile.doodads {
            if seen.insert(d.unique_id) && model_key(&d.model).contains(&needle) {
                hits += 1;
                println!(
                    "MDDF uid {:>8}  pos ({:>9.2}, {:>9.2}, {:>8.2})  rot deg ({:>7.2}, {:>7.2}, {:>7.2})  scale {:.3}  {}",
                    d.unique_id,
                    d.position[0], d.position[1], d.position[2],
                    d.rotation[0], d.rotation[1], d.rotation[2],
                    d.scale,
                    d.model,
                );
            }
        }
        for w in &tile.wmos {
            if seen.insert(w.unique_id) && w.model.to_ascii_lowercase().contains(&needle) {
                hits += 1;
                println!(
                    "MODF uid {:>8}  pos ({:>9.2}, {:>9.2}, {:>8.2})  rot deg ({:>7.2}, {:>7.2}, {:>7.2})  {}",
                    w.unique_id,
                    w.position[0], w.position[1], w.position[2],
                    w.rotation[0], w.rotation[1], w.rotation[2],
                    w.model,
                );
            }
        }
    }
    eprintln!("{hits} placement(s) matched '{filter}'");
    Ok(())
}
