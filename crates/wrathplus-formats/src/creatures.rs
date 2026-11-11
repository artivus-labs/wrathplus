//! Creature display resolution: a `displayId` to its M2, skin textures and scale through
//! `CreatureDisplayInfo` and `CreatureModelData`. A humanoid NPC's look is not on the wire: it
//! comes from `CreatureDisplayInfoExtra` and a baked atlas under `Textures\BakedNpcTextures\`.

use std::collections::HashMap;

use crate::{Chain, DataProfile};
use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{f32_at, parse, str_at, u32_at};

const CREATURE_MODEL_DATA: &str = "DBFilesClient\\CreatureModelData.dbc";
const CREATURE_DISPLAY_INFO: &str = "DBFilesClient\\CreatureDisplayInfo.dbc";
const CREATURE_DISPLAY_INFO_EXTRA: &str = "DBFilesClient\\CreatureDisplayInfoExtra.dbc";

mod scale;

/// A resolved creature display: model, scale, skins and, for a humanoid NPC, its appearance.
#[derive(Debug, Clone)]
pub struct CreatureModel {
    /// `.mdx` path (the M2 loader maps it to `.m2`), e.g. `Creature\Basilisk\Basilisk.mdx`.
    pub model_path: String,
    /// `CreatureModelData.modelScale * CreatureDisplayInfo.creatureModelScale`.
    pub scale: f32,
    /// `textureVariation[0..2]`, bare names found beside the model, for its `Monster1/2/3` slots;
    /// a humanoid NPC's body build (`0x5fb200`) never reads them.
    pub textures: [Option<String>; 3],
    /// A character-model NPC's appearance; `None` for a plain creature (`ExtendedDisplayInfoID` 0).
    pub npc_appearance: Option<NpcAppearance>,
    /// `CreatureDisplayInfo.BloodLevel` (+0x28), tier 1 of the reference's `UnitBloodLevels` row
    /// resolve, which [`crate::BloodCatalog::level_key`] performs.
    pub blood_display: i32,
    /// `CreatureModelData.BloodID` (+0x14), tier 2 of the same resolve: `−1` is a miss that falls
    /// through to tier 3, not bloodlessness.
    pub blood_model: i32,
    /// `CreatureModelData.collisionHeight`, in raw model units.
    pub collision_height: f32,
}

/// A character-model NPC's appearance, from `CreatureDisplayInfoExtra.dbc`. `skin` and `face` are
/// already in the baked atlas; they serve the live composite when a row has no bake name.
#[derive(Debug, Clone)]
pub struct NpcAppearance {
    pub race: u8,
    pub sex: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    /// Worn `ItemDisplayInfo` ids (fields 8..17), 0 = empty, by body slot: head, shoulder, shirt,
    /// chest, belt, pants, boots, wrist, gloves, tabard (no cloak column).
    pub equipment: [u32; 10],
    /// The eleventh NPC item display, the cloak slot.
    pub cloak: u32,
    /// The baked body atlas under `Textures\BakedNpcTextures\`; `None` composites the body live.
    pub bake_name: Option<String>,
}

/// One CreatureDisplayInfo row (the parts we use).
#[derive(Debug, Clone)]
struct DisplayRow {
    model_id: u32,
    /// `ExtendedDisplayInfoID`: the `CreatureDisplayInfoExtra` key; 0 = none.
    extended_id: u32,
    scale: f32,
    textures: [Option<String>; 3],
    /// `BloodLevel` (field 10), a per-display `UnitBloodLevels` override; 0 names no row.
    blood_level: u32,
    /// `SizeClass` (field 9, +0x24), the display's override, read signed: `−1` defers to the model.
    size_class: i32,
    /// `CreatureModelAlpha` (field 5, +0x14), 0..=255: the `baseAlpha` of the reference's unit
    /// alpha product (`0x60d2d0`, × 1/255), which is a flat 1.0 for players.
    model_alpha: u32,
}

/// One CreatureModelData row (the parts we use).
#[derive(Debug, Clone)]
struct ModelRow {
    path: String,
    scale: f32,
    /// `Flags` (field 1); bit 0x2 is the no-breath flag.
    flags: u32,
    /// `SizeClass` (field 3, +0x0c), 0 Small to 4 Colossal, read signed like the display's.
    size_class: i32,
    /// `BloodID`, read signed: `−1` is a tier-2 miss, not bloodlessness.
    blood: i32,
    /// `FootprintTextureID` (field 6), read signed: `−1` leaves no prints.
    footprint_texture: i32,
    /// `FootprintTextureLength`/`Width` (fields 7/8), in inches (× 1/36 to yards, `0x607a00`).
    footprint_length: f32,
    footprint_width: f32,
    /// `collisionHeight` (field 15), raw model units.
    collision_height: f32,
    /// `FoleyMaterialID` (field 10): the reference reads `[unit+0xb3c]+0x28`, field 10 of 16.
    foley_material: u32,
    /// `FootstepShakeSize`/`DeathThudShakeSize` (fields 11/12), `CameraShakes.dbc` ids; 0 = none.
    footstep_shake: u32,
    death_thud_shake: u32,
    /// `WorldEffectScale`/`AttachedEffectScale` (fields 23/24, `+0x5c`/`+0x60`); 1 before Wrath.
    effect_scale: [f32; 2],
}

/// A footprint decal: the `FootprintTextures.dbc` key and the print size in yards (length along
/// the facing, width across), before the unit's scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootprintParams {
    pub texture_id: u32,
    pub length: f32,
    pub width: f32,
}

/// The creature display tables; `Default` is empty, which consumers treat as a failed load.
#[derive(Default)]
pub struct CreatureCatalog {
    display: HashMap<u32, DisplayRow>,
    models: HashMap<u32, ModelRow>,
    extra: HashMap<u32, NpcAppearance>,
    /// `ChrRaces` `(race, sex)` → the race's base display (`+0x10` male, `+0x14` female).
    race_displays: HashMap<(u8, u8), u32>,
}

impl CreatureCatalog {
    /// A display's footstep camera shake, a `CameraShakes.dbc` id (`FootstepShakeSize`).
    pub fn footstep_shake(&self, display_id: u32) -> Option<u32> {
        let row = self.display.get(&display_id)?;
        let id = self.models.get(&row.model_id)?.footstep_shake;
        (id != 0).then_some(id)
    }

    /// A display's camera shake as the body lands (`DeathThudShakeSize`).
    pub fn death_thud_shake(&self, display_id: u32) -> Option<u32> {
        let row = self.display.get(&display_id)?;
        let id = self.models.get(&row.model_id)?.death_thud_shake;
        (id != 0).then_some(id)
    }

    /// A display's audible size class, 0 Small to 4 Colossal, the axis of
    /// [`crate::DeathThudCatalog`]. The display's `SizeClass` wins and only its `−1` defers to the
    /// model's (`0x625500`); `None` when both are `−1`, which the reference's unsigned `>= 5` gate
    /// (`0x623744`) also silences.
    pub fn size_class(&self, display_id: u32) -> Option<u32> {
        let row = self.display.get(&display_id)?;
        let class = match row.size_class {
            -1 => self.models.get(&row.model_id)?.size_class,
            c => c,
        };
        u32::try_from(class).ok()
    }

    /// Every model row naming a shake: `(model id, path, footstep id, death-thud id)`.
    pub fn shaking_models(&self) -> impl Iterator<Item = (u32, &str, u32, u32)> + '_ {
        self.models
            .iter()
            .filter(|(_, m)| m.footstep_shake != 0 || m.death_thud_shake != 0)
            .map(|(id, m)| (*id, m.path.as_str(), m.footstep_shake, m.death_thud_shake))
    }

    /// Every model path: being a creature model decides whether an M2's animation events reach
    /// `CGUnit_C::HandleAnimEvent`.
    pub fn model_paths(&self) -> impl Iterator<Item = &str> + '_ {
        self.models.values().map(|m| m.path.as_str())
    }

    /// Every model row as `(model id, path, SizeClass)`; most displays override the column.
    pub fn sized_models(&self) -> impl Iterator<Item = (u32, &str, i32)> + '_ {
        self.models
            .iter()
            .map(|(id, m)| (*id, m.path.as_str(), m.size_class))
    }

    /// Every display as `(display id, model id)`.
    pub fn display_models(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.display.iter().map(|(id, r)| (*id, r.model_id))
    }

    /// The display's model's `[WorldEffectScale, AttachedEffectScale]`: the size factor a spell
    /// effect planted at the unit (`0x6f7950`) or riding one of its attach points (`0x6f8c50`)
    /// takes, through the same display → model resolve (`0x717a20`).
    pub fn effect_scales(&self, display_id: u32) -> Option<[f32; 2]> {
        let row = self.display.get(&display_id)?;
        Some(self.models.get(&row.model_id)?.effect_scale)
    }

    /// `CreatureModelAlpha / 255`, the first factor of the unit alpha product.
    pub fn display_base_alpha(&self, display_id: u32) -> Option<f32> {
        self.display
            .get(&display_id)
            .map(|r| f32::from(r.model_alpha.min(255) as u8) / 255.0)
    }

    /// `CreatureModelData.collisionHeight`, the `h` of every depth line (swim `0.75·h`, splash
    /// `0.4·h`, foam gate `2·h`, read against `CMovement+0xb4`). It is the M2's collision-box Z
    /// extent, pre-scale: the prism is this × `max(OBJECT_FIELD_SCALE_X, creatureModelScale)`
    /// (`0x60b312` → `0x617501`). A miss takes the constructor default, `2.0277777` (`0x616fd8`).
    pub fn collision_height(&self, display_id: u32) -> Option<f32> {
        let row = self.display.get(&display_id)?;
        Some(self.models.get(&row.model_id)?.collision_height)
    }

    /// A display's foley material (`Material.dbc`), the creature half of the footfall rustle:
    /// `0x623610` hands `[[unit+0xb3c]+0x28]` to `0x4584e0`, while a player reads its chest
    /// (`0x62fa30`). Every shipped row is 0, so an NPC's silent armor is the data, not a bug.
    pub fn foley_material(&self, display_id: u32) -> Option<u32> {
        let row = self.display.get(&display_id)?;
        Some(self.models.get(&row.model_id)?.foley_material)
    }

    /// Whether the model may wear the `$BTH` breath effects: `CreatureModelData.Flags & 0x2`
    /// suppresses them (`0x600003`, the row at `[unit+0xb3c]`); an unknown display breathes.
    pub fn breathes(&self, display_id: u32) -> bool {
        self.display
            .get(&display_id)
            .and_then(|row| self.models.get(&row.model_id))
            .is_none_or(|m| m.flags & 0x2 == 0)
    }

    /// A display's footprint decal, fields 6..=8, in yards (inches × 1/36, the client's cache at
    /// `0x607a00` for the getter `0x607920`); `None` for a `−1` texture or a 0×0 print.
    pub fn footprint(&self, display_id: u32) -> Option<FootprintParams> {
        let row = self.display.get(&display_id)?;
        let model = self.models.get(&row.model_id)?;
        let texture_id = u32::try_from(model.footprint_texture).ok()?;
        let (length, width) = (model.footprint_length / 36.0, model.footprint_width / 36.0);
        (length > 0.0 && width > 0.0).then_some(FootprintParams {
            texture_id,
            length,
            width,
        })
    }

    /// Resolve an NPC display id to its model.
    pub fn model(&self, display_id: u32) -> Option<CreatureModel> {
        let row = self.display.get(&display_id)?;
        let model = self.models.get(&row.model_id)?;
        // A non-zero `ExtendedDisplayInfoID` with an extra row marks a character-model NPC.
        let npc_appearance = (row.extended_id != 0)
            .then(|| self.extra.get(&row.extended_id).cloned())
            .flatten();
        Some(CreatureModel {
            model_path: model.path.clone(),
            scale: model.scale * row.scale,
            textures: row.textures.clone(),
            npc_appearance,
            blood_display: row.blood_level as i32,
            blood_model: model.blood,
            collision_height: model.collision_height,
        })
    }

    /// The `(race, sex)` of a display's `CreatureDisplayInfoExtra` row, the unit's display
    /// override (`[unit+0xb38]`, `0x60afb0`); `None` for a display with no extra row.
    pub fn display_race_sex(&self, display_id: u32) -> Option<(u8, u8)> {
        let extra = self.display.get(&display_id)?.extended_id;
        let row = (extra != 0).then(|| self.extra.get(&extra)).flatten()?;
        Some((row.race, row.sex))
    }

    /// Number of display entries (for logging/diagnostics).
    pub fn len(&self) -> usize {
        self.display.len()
    }

    /// Whether the catalog has no display entries.
    pub fn is_empty(&self) -> bool {
        self.display.is_empty()
    }

    /// Number of appearance rows; 0 means `CreatureDisplayInfoExtra` failed to load.
    pub fn extra_len(&self) -> usize {
        self.extra.len()
    }
}

/// CreatureModelData.dbc: 28 fields in build 12340.
pub(crate) fn creature_model_data_schema(profile: &DataProfile) -> Schema {
    let mut s = Schema::new("CreatureModelData");
    for (name, ty) in [
        ("ID", FieldType::UInt32),
        ("Flags", FieldType::UInt32),
        ("ModelName", FieldType::String),
        ("SizeClass", FieldType::UInt32),
        ("ModelScale", FieldType::Float32),
        ("BloodID", FieldType::UInt32),
        ("FootprintTextureID", FieldType::UInt32),
        ("FootprintTextureLength", FieldType::Float32),
        ("FootprintTextureWidth", FieldType::Float32),
        ("FootprintParticleScale", FieldType::Float32),
        ("FoleyMaterialID", FieldType::UInt32),
        ("FootstepShakeSize", FieldType::UInt32),
        ("DeathThudShakeSize", FieldType::UInt32),
        ("SoundID", FieldType::UInt32),
        ("CollisionWidth", FieldType::Float32),
        ("CollisionHeight", FieldType::Float32),
    ] {
        s.add_field(SchemaField::new(name, ty));
    }
    if matches!(profile, DataProfile::Wrath { .. }) {
        for name in [
            "MountHeight",
            "GeoBoxMinX",
            "GeoBoxMinY",
            "GeoBoxMinZ",
            "GeoBoxMaxX",
            "GeoBoxMaxY",
            "GeoBoxMaxZ",
            "WorldEffectScale",
            "AttachedEffectScale",
            "MissileCollisionRadius",
            "MissileCollisionPush",
            "MissileCollisionRaise",
        ] {
            s.add_field(SchemaField::new(name, FieldType::Float32));
        }
    }
    s
}

/// CreatureDisplayInfo.dbc: 16 fields in build 12340.
pub(crate) fn creature_display_info_schema(profile: &DataProfile) -> Schema {
    let mut s = Schema::new("CreatureDisplayInfo");
    for (name, ty) in [
        ("ID", FieldType::UInt32),
        ("ModelID", FieldType::UInt32),
        ("SoundID", FieldType::UInt32),
        ("ExtendedDisplayInfoID", FieldType::UInt32),
        ("CreatureModelScale", FieldType::Float32),
        ("CreatureModelAlpha", FieldType::UInt32),
        ("TextureVariation0", FieldType::String),
        ("TextureVariation1", FieldType::String),
        ("TextureVariation2", FieldType::String),
        ("PortraitTexture", FieldType::UInt32),
        ("SizeClass", FieldType::UInt32),
        ("BloodID", FieldType::UInt32),
    ] {
        s.add_field(SchemaField::new(name, ty));
    }
    if matches!(profile, DataProfile::Wrath { .. }) {
        for name in [
            "NPCSoundID",
            "ParticleColorID",
            "CreatureGeosetData",
            "ObjectEffectPackageID",
        ] {
            s.add_field(SchemaField::new(name, FieldType::UInt32));
        }
    }
    s
}

/// CreatureDisplayInfoExtra.dbc: 21 fields in build 12340.
pub(crate) fn creature_display_info_extra_schema(profile: &DataProfile) -> Schema {
    let mut s = Schema::new("CreatureDisplayInfoExtra");
    for (name, ty) in [
        ("ID", FieldType::UInt32),
        ("Race", FieldType::UInt32),
        ("Sex", FieldType::UInt32),
        ("SkinColor", FieldType::UInt32),
        ("FaceType", FieldType::UInt32),
        ("HairStyle", FieldType::UInt32),
        ("HairColor", FieldType::UInt32),
        ("FacialHair", FieldType::UInt32),
    ] {
        s.add_field(SchemaField::new(name, ty));
    }
    for i in 0..10 {
        s.add_field(SchemaField::new(format!("Equipment{i}"), FieldType::UInt32));
    }
    match profile {
        DataProfile::Wrath { .. } => {
            s.add_field(SchemaField::new("CloakDisplay", FieldType::UInt32));
            s.add_field(SchemaField::new("Flags", FieldType::UInt32));
            s.add_field(SchemaField::new("BakeName", FieldType::String));
        }
    }
    s
}

/// Load the creature DBCs from the patch chain into a [`CreatureCatalog`].
pub fn load_creature_catalog(chain: &mut Chain) -> Result<CreatureCatalog> {
    let profile = chain.profile().clone();
    let models = {
        let bytes = chain
            .read_file(CREATURE_MODEL_DATA)
            .with_context(|| format!("reading {CREATURE_MODEL_DATA}"))?;
        let rs = parse(
            &bytes,
            creature_model_data_schema(&profile),
            "CreatureModelData",
        )?;
        let mut m = HashMap::with_capacity(rs.records().len());
        for r in rs.records() {
            if let (Some(id), Some(name)) = (u32_at(r, 0), str_at(&rs, r, 2)) {
                m.insert(
                    id,
                    ModelRow {
                        path: name,
                        scale: f32_at(r, 4).unwrap_or(1.0),
                        flags: u32_at(r, 1).unwrap_or(0),
                        size_class: u32_at(r, 3).map_or(-1, |v| v as i32),
                        blood: u32_at(r, 5).map_or(0, |v| v as i32),
                        footprint_texture: u32_at(r, 6).map_or(-1, |v| v as i32),
                        footprint_length: f32_at(r, 7).unwrap_or(0.0),
                        footprint_width: f32_at(r, 8).unwrap_or(0.0),
                        collision_height: f32_at(r, 15).unwrap_or(0.0),
                        foley_material: u32_at(r, 10).unwrap_or(0),
                        footstep_shake: u32_at(r, 11).unwrap_or(0),
                        death_thud_shake: u32_at(r, 12).unwrap_or(0),
                        effect_scale: [23, 24].map(|i| f32_at(r, i).unwrap_or(1.0)),
                    },
                );
            }
        }
        m
    };

    let display = {
        let bytes = chain
            .read_file(CREATURE_DISPLAY_INFO)
            .with_context(|| format!("reading {CREATURE_DISPLAY_INFO}"))?;
        let rs = parse(
            &bytes,
            creature_display_info_schema(&profile),
            "CreatureDisplayInfo",
        )?;
        let mut d = HashMap::with_capacity(rs.records().len());
        for r in rs.records() {
            if let (Some(id), Some(model_id)) = (u32_at(r, 0), u32_at(r, 1)) {
                d.insert(
                    id,
                    DisplayRow {
                        model_id,
                        extended_id: u32_at(r, 3).unwrap_or(0),
                        scale: f32_at(r, 4).unwrap_or(1.0),
                        textures: [str_at(&rs, r, 6), str_at(&rs, r, 7), str_at(&rs, r, 8)],
                        blood_level: u32_at(r, 11).unwrap_or(0),
                        size_class: u32_at(r, 10).map_or(-1, |v| v as i32),
                        model_alpha: u32_at(r, 5).unwrap_or(255),
                    },
                );
            }
        }
        d
    };

    // Best-effort: without it only humanoid NPCs lose their skins; the caller logs `extra_len()`.
    let extra = load_creature_display_info_extra(chain).unwrap_or_default();
    // Best-effort: without it a humanoid NPC's race scale reads 1.0.
    let race_displays = scale::race_displays(chain);

    Ok(CreatureCatalog {
        display,
        models,
        extra,
        race_displays,
    })
}

/// Load CreatureDisplayInfoExtra.dbc: `extendedDisplayInfoId` → [`NpcAppearance`].
fn load_creature_display_info_extra(chain: &mut Chain) -> Result<HashMap<u32, NpcAppearance>> {
    let profile = chain.profile().clone();
    let bytes = chain
        .read_file(CREATURE_DISPLAY_INFO_EXTRA)
        .with_context(|| format!("reading {CREATURE_DISPLAY_INFO_EXTRA}"))?;
    let rs = parse(
        &bytes,
        creature_display_info_extra_schema(&profile),
        "CreatureDisplayInfoExtra",
    )?;
    let mut e = HashMap::with_capacity(rs.records().len());
    for r in rs.records() {
        if let Some(id) = u32_at(r, 0) {
            e.insert(
                id,
                NpcAppearance {
                    race: u32_at(r, 1).unwrap_or(0) as u8,
                    sex: u32_at(r, 2).unwrap_or(0) as u8,
                    skin: u32_at(r, 3).unwrap_or(0) as u8,
                    face: u32_at(r, 4).unwrap_or(0) as u8,
                    hair_style: u32_at(r, 5).unwrap_or(0) as u8,
                    hair_color: u32_at(r, 6).unwrap_or(0) as u8,
                    facial_hair: u32_at(r, 7).unwrap_or(0) as u8,
                    equipment: std::array::from_fn(|i| u32_at(r, 8 + i).unwrap_or(0)),
                    cloak: u32_at(r, 18).unwrap_or(0),
                    bake_name: str_at(&rs, r, 20),
                },
            );
        }
    }
    Ok(e)
}

#[cfg(test)]
#[path = "creatures_tests.rs"]
mod tests;
