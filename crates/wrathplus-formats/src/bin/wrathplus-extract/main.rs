//! `wrathplus-extract`: a CLI over the 3.3.5a (build 12340) MPQ patch chain, where later archives
//! override earlier ones. This file holds the clap commands, their dispatch and the shared
//! helpers; the subcommands live in modules by concern.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use wrathplus_formats::open_chain;

mod chaincensus;
mod charatlas;
mod charprocs;
mod kitanim;
mod m2info;
mod scan;
mod spellvis;

/// Read WoW 3.3.5a asset archives (MPQ).
#[derive(Parser)]
#[command(
    name = "wrathplus-extract",
    version,
    about,
    allow_negative_numbers = true
)]
struct Cli {
    /// A single `.MPQ` file, or a `Data` directory (opens the full patch chain).
    path: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List files across the chain, sorted by name.
    List,
    /// Extract one file by its internal path, writing raw bytes to `output`.
    Extract {
        /// Internal path inside the archive (forward or back slashes accepted).
        internal_path: String,
        /// Output file to write the extracted bytes to.
        output: PathBuf,
    },
    /// Decode a BLP texture from the archive and write it as a PNG.
    Blp {
        /// Internal path to the `.blp` (forward or back slashes accepted).
        internal_path: String,
        /// Output `.png` file.
        output: PathBuf,
        /// Also write every mip (`<stem>.mip<N>.png`) and census each: transparent and opaque
        /// texels, their luma, and how many sit below 128, which darken under alpha-blind Mod2x.
        #[arg(long)]
        mips: bool,
    },
    /// Composite one character's body atlas and report what painted what.
    Charatlas {
        /// `ChrRaces`: 1 human, 2 orc, 3 dwarf, 4 night elf, 5 undead, 6 tauren, 7 gnome, 8 troll.
        #[arg(long)]
        race: u8,
        /// 0 male · 1 female.
        #[arg(long)]
        sex: u8,
        /// `skinColor`, the CharSections variation the base skin and head sections key on.
        #[arg(long, default_value_t = 0)]
        skin: u8,
        /// `faceType`.
        #[arg(long, default_value_t = 0)]
        face: u8,
        /// `facialHairStyle`.
        #[arg(long, default_value_t = 0)]
        facial_hair: u8,
        /// `hairStyle`.
        #[arg(long, default_value_t = 0)]
        hair_style: u8,
        /// `hairColor`.
        #[arg(long, default_value_t = 0)]
        hair_color: u8,
        /// The eight worn `ItemDisplayInfo` ids, comma-separated in bodyslot order (shirt, chest,
        /// belt, pants, boots, wrist, gloves, tabard); a 0 or a short list leaves slots empty.
        #[arg(long, value_delimiter = ',', default_value = "0")]
        slots: Vec<u32>,
        /// The guild tabard, `emblemStyle,emblemColor,borderStyle,borderColor,backgroundColor` as
        /// `SMSG_GUILD_QUERY_RESPONSE` carries them; only a guild-emblem tabard (20621) shows it.
        #[arg(long, value_delimiter = ',')]
        emblem: Option<Vec<i32>>,
        /// Write the composited atlas here as a PNG (256²).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Dump a DBC table to CSV using our schema for it (headers included).
    Dbc {
        /// Internal path to the `.dbc` (forward or back slashes accepted).
        internal_path: String,
        /// Output `.csv` file.
        output: PathBuf,
    },
    /// Dump a spell's visual chain: its `SpellVisual` stages and each kit's contents.
    Spellvis {
        /// The `Spell.dbc` id (e.g. 133 = Fireball).
        spell_id: u32,
    },
    /// Census the beam system: `SpellChainEffects` and every `SpellVisualKit` that draws a beam.
    Chaincensus,
    /// Census the `SpellVisualKit` CharProc columns, a kit's effect on the body, by stage.
    Charprocs,
    /// Census the `SpellVisualKit` animation column (field 2), a clip on the unit's body, by stage.
    Kitanim,
    /// Sweep every `.m2` and census particle flipbook fields: backward ramps (`begin > end`, legal
    /// and shipped), tail ramps unlike the head's, indices past the atlas (the reference wraps the
    /// column, the row runs off), repeat counts, and the reference's two degenerate fallbacks.
    Cellscan,
    /// Sweep every `.m2` for materials with the multiply blends 5 (Mod) and 6 (Mod2x), the
    /// armor-reflect sheen family.
    Blendscan {
        /// Internal-path prefix filter (e.g. `item\objectcomponents\weapon`), case-insensitive;
        /// all models if omitted.
        prefix: Option<String>,
    },
    /// Sweep every `.m2` and census each particle-emitter feature the corpus authors.
    Partcensus {
        /// Internal-path prefix filter (e.g. `spells`), case-insensitive; all models if omitted.
        prefix: Option<String>,
    },
    /// Sweep every `.m2` for emitters whose file slot 0 is dead while a later slot lives
    /// (`peak0 <= 0 < peakN`): the reference samples the playing sequence's rate window, so a
    /// consumer pinned to slot 0 never emits them.
    Partslotscan {
        /// Internal-path prefix filter (e.g. `world`), case-insensitive; all models if omitted.
        prefix: Option<String>,
    },
    /// Sweep every `.m2` and list the models whose particle emitters carry any of the given
    /// file-flag bits (`M2ParticleEmitter+0x04`).
    Partscan {
        /// The flag mask to match, hex or decimal (e.g. `0x1000`).
        mask: String,
        /// Internal-path prefix filter (e.g. `spells`), case-insensitive; all models if omitted.
        prefix: Option<String>,
    },
    /// Sweep every WMO root and cross-tab its MOSB skybox model against its groups' `0x40000`
    /// flag, which never appears without a MOSB in all 815 roots; the reference tests it on the
    /// flood-visited group (`0x6b42e0`). Also which buildings replace the `Light.dbc` dome with an
    /// authored sky, and in how many groups.
    Skyboxscan,
    /// Report one v264 model: sequences, bones, attachments, events, batches, particle emitters,
    /// ribbons, lights and cameras, read with its sibling `.anim` files and `00.skin`.
    M2info {
        /// Internal path to the `.m2` (`.mdx`/`.mdl` resolve to `.m2`).
        internal_path: String,
    },
    /// Dump all 18 `LightIntBand` rows of the `Light.dbc` entry covering a position at a time of
    /// day. One params record, raw: near a sphere's falloff edge the live light is mostly the
    /// continent global, and `lightblend` shows what wins there.
    Lightbands {
        /// Map id (0 = Eastern Kingdoms, 1 = Kalimdor).
        map: u32,
        /// World-space X (raw WoW yards; may be negative).
        #[arg(allow_hyphen_values = true)]
        x: f32,
        /// World-space Y.
        #[arg(allow_hyphen_values = true)]
        y: f32,
        /// World-space Z.
        #[arg(allow_hyphen_values = true)]
        z: f32,
        /// Game minute-of-day (0..1440; 720 = noon).
        minute: u32,
    },
    /// Dump every `LightIntBand` and `LightFloatBand` row of a named `LightParams` id, for rows no
    /// position reaches: magma submersion reads the fixed global row 7 and slime row 6
    /// (`0x6d2371`), not the zone's underwater slot.
    Lightparam {
        /// `LightParams.dbc` id (1-based).
        id: u32,
        /// Game minute-of-day (0..1440; 720 = noon).
        minute: u32,
    },
    /// Dump the per-weather-slot light resolve at a position: the `LightParams` id each of the five
    /// `Light.dbc` slots names (clear, clear underwater, storm, storm underwater, death), the unset
    /// ones that fall back to clear, and each one's fog, ambient and diffuse.
    Lightslots {
        /// Map id (0 = Eastern Kingdoms, 1 = Kalimdor).
        map: u32,
        /// World-space X (raw WoW yards; may be negative).
        #[arg(allow_hyphen_values = true)]
        x: f32,
        /// World-space Y.
        #[arg(allow_hyphen_values = true)]
        y: f32,
        /// World-space Z.
        #[arg(allow_hyphen_values = true)]
        z: f32,
        /// Game minute-of-day (0..1440; 720 = noon).
        minute: u32,
    },
    /// Dump every light sphere covering a position (distance, falloff, blend alpha, clear ambient
    /// and sun), then the area-blend result against the single `pick_light` sample.
    Lightblend {
        /// Map id (0 = Eastern Kingdoms, 1 = Kalimdor).
        map: u32,
        /// World-space X (raw WoW yards; may be negative).
        #[arg(allow_hyphen_values = true)]
        x: f32,
        /// World-space Y.
        #[arg(allow_hyphen_values = true)]
        y: f32,
        /// World-space Z.
        #[arg(allow_hyphen_values = true)]
        z: f32,
        /// Game minute-of-day (0..1440; 720 = noon).
        minute: u32,
    },
    /// Dump a WMO root's placed props: each MODD doodad with its MODS sets and the groups whose
    /// MODR names it. The reference creates props only from a group's MODR refs (`0x695aa0`), so
    /// it never creates one no group names; WrathPlus still spawns those.
    Wmodoodads {
        /// Internal path to the WMO root (forward or back slashes accepted).
        internal_path: String,
        /// Case-insensitive substring of the prop's model path (e.g. `lightray`); all if omitted.
        filter: Option<String>,
    },
    /// Sweep every WMO root for placed MODD props the interior lane lights literal black: its base
    /// light is the MODD colour (ambient `cap96`, diffuse `floor112`, `0x694e90` → `0x6a77e0`), and
    /// the `112/max` floor keeps `#000000` black. An exterior referrer sky-lights a prop instead
    /// (`0x695aa0`); `RESCUED` counts those an interior group names first.
    Darkpropscan {
        /// Internal-path prefix to limit the sweep (e.g. `world\wmo\azeroth`); all if omitted.
        prefix: Option<String>,
    },
    /// List the doodad (MDDF) and WMO (MODF) placements around a position whose model path
    /// contains a substring: position, Euler rotation (deg), scale and uniqueId.
    Placescan {
        /// Map directory name (e.g. `Azeroth`).
        map: String,
        /// World-space center X (may be negative).
        #[arg(allow_hyphen_values = true)]
        center_x: f32,
        /// World-space center Y (may be negative).
        #[arg(allow_hyphen_values = true)]
        center_y: f32,
        /// ADT tile radius around the center tile to scan (0 = just the containing tile).
        tile_radius: u32,
        /// Case-insensitive substring of the model path (e.g. `instanceportal`).
        filter: String,
    },
    /// Print the terrain MCSH baked-shadow bit at a position, which an exterior M2 doodad samples
    /// once at its base (sun scale 1.0 lit, 0.5 shadowed, `0x698cb4`), and the texel
    /// neighbourhood: one ~0.52 yd texel can split adjacent pieces across a shadow edge.
    Shadeat {
        /// Map directory name (e.g. `Azeroth`).
        map: String,
        /// World-space X (raw WoW yards; may be negative).
        #[arg(allow_hyphen_values = true)]
        x: f32,
        /// World-space Y.
        #[arg(allow_hyphen_values = true)]
        y: f32,
    },
}

/// MPQ internal paths are backslash-separated; accept `/` for convenience.
fn normalize(path: &str) -> String {
    path.replace('/', "\\")
}

/// A placed model path's grouping key, lowercase with `.mdx`/`.mdl` as `.m2`; it must match
/// `wrathplus_formats`' crate-private `model_path` normalization.
fn model_key(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    match lower
        .strip_suffix(".mdx")
        .or_else(|| lower.strip_suffix(".mdl"))
    {
        Some(stem) => format!("{stem}.m2"),
        None => lower,
    }
}

/// Parse a `0x`-prefixed hex or plain-decimal u32 (`partscan`'s mask argument).
fn parse_u32_maybe_hex(s: &str) -> Result<u32> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).context("invalid hex"),
        None => s.parse().context("invalid decimal"),
    }
}

/// `--emblem` → a [`wrathplus_formats::GuildEmblem`]: the five wire indices in
/// `SMSG_GUILD_QUERY_RESPONSE` order.
fn guild_emblem(v: Vec<i32>) -> Result<wrathplus_formats::GuildEmblem> {
    let [emblem_style, emblem_color, border_style, border_color, background_color]: [i32; 5] =
        v.try_into().map_err(|v: Vec<i32>| {
            anyhow::anyhow!(
                "--emblem takes 5 comma-separated indices \
                 (emblemStyle,emblemColor,borderStyle,borderColor,backgroundColor), got {}",
                v.len()
            )
        })?;
    Ok(wrathplus_formats::GuildEmblem {
        emblem_style,
        emblem_color,
        border_style,
        border_color,
        background_color,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut chain = open_chain(&cli.path)?;

    match cli.command {
        Command::List => {
            let mut files = chain.list().context("listing chain contents")?;
            files.sort_by(|a, b| a.name.cmp(&b.name));
            for entry in &files {
                println!("{:>12}  {}", entry.size, entry.name);
            }
            eprintln!("{} files", files.len());
        }
        Command::Charatlas {
            race,
            sex,
            skin,
            face,
            facial_hair,
            hair_style,
            hair_color,
            slots,
            emblem,
            out,
        } => {
            let mut worn = [0u32; 8];
            for (dst, src) in worn.iter_mut().zip(&slots) {
                *dst = *src;
            }
            charatlas::charatlas(
                &mut chain,
                &charatlas::Look {
                    race,
                    sex,
                    skin,
                    face,
                    facial_hair,
                    hair_style,
                    hair_color,
                    slots: worn,
                    emblem: emblem.map(guild_emblem).transpose()?,
                },
                out.as_deref(),
            )?;
        }
        Command::Extract {
            internal_path,
            output,
        } => {
            let name = normalize(&internal_path);
            // The archive the file resolved from, which shows an override.
            let source = chain
                .find_file_archive(&name)
                .map(|p| p.display().to_string());
            let data = chain
                .read_file(&name)
                .with_context(|| format!("reading '{name}' from chain"))?;
            std::fs::write(&output, &data)
                .with_context(|| format!("writing {}", output.display()))?;
            match source {
                Some(src) => eprintln!(
                    "wrote {} bytes to {} (from {})",
                    data.len(),
                    output.display(),
                    src
                ),
                None => eprintln!("wrote {} bytes to {}", data.len(), output.display()),
            }
        }
        Command::Blp {
            internal_path,
            output,
            mips,
        } => {
            let name = normalize(&internal_path);
            let data = chain
                .read_file(&name)
                .with_context(|| format!("reading '{name}' from chain"))?;
            if mips {
                let stats = wrathplus_formats::blp_mips_to_png(&data, &output)
                    .with_context(|| format!("decoding BLP '{name}'"))?;
                let luma = |l: Option<(u8, f32, u8)>| match l {
                    Some((lo, mean, hi)) => format!("{lo:>3}/{mean:>6.1}/{hi:>3}"),
                    None => "      —       ".to_string(),
                };
                println!(
                    "level  size      outside(a=0)  luma lo/mean/hi   inside(a>0)  luma lo/mean/hi   below128"
                );
                for s in &stats {
                    println!(
                        "{:>5}  {:>4}x{:<4}  {:>12}  {:>14}  {:>11}  {:>14}  {:>8}",
                        s.level,
                        s.width,
                        s.height,
                        s.outside,
                        luma(s.outside_luma),
                        s.inside,
                        luma(s.inside_luma),
                        s.below_128,
                    );
                }
                eprintln!("wrote {} level(s) beside {}", stats.len(), output.display());
            } else {
                let (w, h) = wrathplus_formats::blp_to_png(&data, &output)
                    .with_context(|| format!("decoding BLP '{name}'"))?;
                eprintln!("decoded {w}x{h} -> {}", output.display());
            }
        }
        Command::Dbc {
            internal_path,
            output,
        } => {
            let name = normalize(&internal_path);
            let data = chain
                .read_file(&name)
                .with_context(|| format!("reading '{name}' from chain"))?;
            let (records, fields) = wrathplus_formats::dbc_to_csv(&data, &name, &output)?;
            eprintln!(
                "wrote {records} records ({fields} fields) -> {}",
                output.display()
            );
        }
        Command::Cellscan => scan::cellscan(&mut chain)?,
        Command::Blendscan { prefix } => scan::blendscan(&mut chain, prefix.as_deref())?,
        Command::Partcensus { prefix } => scan::partcensus(&mut chain, prefix.as_deref())?,
        Command::Partslotscan { prefix } => scan::partslotscan(&mut chain, prefix.as_deref())?,
        Command::Partscan { mask, prefix } => {
            let mask = parse_u32_maybe_hex(&mask)
                .with_context(|| format!("parsing flag mask '{mask}'"))?;
            scan::partscan(&mut chain, mask, prefix.as_deref())?;
        }
        Command::Skyboxscan => scan::skyboxscan(&mut chain)?,
        Command::M2info { internal_path } => m2info::m2info(&mut chain, &internal_path)?,
        Command::Lightbands {
            map,
            x,
            y,
            z,
            minute,
        } => {
            let catalog = wrathplus_formats::LightCatalog::load(&mut chain)?;
            // `debug_bands` takes the client's half-minute clock (0..2880).
            catalog.debug_bands(map, [x, y, z], minute * 2);
        }
        Command::Lightparam { id, minute } => {
            let catalog = wrathplus_formats::LightCatalog::load(&mut chain)?;
            catalog.debug_param(id, minute * 2);
        }
        Command::Lightslots {
            map,
            x,
            y,
            z,
            minute,
        } => {
            let catalog = wrathplus_formats::LightCatalog::load(&mut chain)?;
            catalog.debug_slots(map, [x, y, z], minute * 2);
        }
        Command::Lightblend {
            map,
            x,
            y,
            z,
            minute,
        } => {
            let catalog = wrathplus_formats::LightCatalog::load(&mut chain)?;
            catalog.debug_blend(map, [x, y, z], minute * 2);
        }
        Command::Wmodoodads {
            internal_path,
            filter,
        } => scan::wmodoodads(&mut chain, &internal_path, filter.as_deref())?,
        Command::Darkpropscan { prefix } => scan::darkpropscan(&mut chain, prefix.as_deref())?,
        Command::Placescan {
            map,
            center_x,
            center_y,
            tile_radius,
            filter,
        } => scan::placescan(&mut chain, &map, center_x, center_y, tile_radius, &filter)?,
        Command::Shadeat { map, x, y } => scan::shadeat(&mut chain, &map, x, y)?,
        Command::Spellvis { spell_id } => spellvis::run(&mut chain, spell_id)?,
        Command::Chaincensus => chaincensus::run(&mut chain)?,
        Command::Charprocs => charprocs::run(&mut chain)?,
        Command::Kitanim => kitanim::run(&mut chain)?,
    }

    Ok(())
}
