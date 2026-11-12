//! Generated terrain on the engine side: how tall it is, filling chunks on every core, and
//! turning what a generator wrote into chunks. Which chunks are filled when is streaming's
//! business (see [`crate::stream`]): the terrain goes on forever sideways.

use api::{ApiError, BoxelPos, ChunkFill, ChunkPos, Generator, I64Vec3, TerrainDef, Tunable, Tunables};

use crate::{BlockRegistry, ChunkData, Layer, Occupant};

/// The engine's own world settings: how tall terrain is, and how far from the player things are
/// simulated.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSettings {
    /// How many chunks each terrain spans bottom to top.
    pub terrain_height: u32,
    /// Creatures and loose items farther than this from the player, in meters, hold still.
    pub sim_distance: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self { terrain_height: 16, sim_distance: 256.0 }
    }
}

/// The tuning panel's groups for [`WorldSettings`]'s values.
const WORLD_GROUPS: &[(&str, &[&str])] = &[
];
/// What each of [`WorldSettings`]'s values does, for the tuning panel.
const WORLD_HELP: &[(&str, &str)] = &[
    ("terrain_height", "How many 32-boxel chunks each terrain spans, bottom to top. Takes effect on Regenerate terrain."),
    ("sim_distance", "Creatures and loose items farther than this from you hold still, meters."),
];

impl Tunables for WorldSettings {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![
            Tunable::new("terrain_height", self.terrain_height as f32, 1.0, 32.0),
            Tunable::new("sim_distance", self.sim_distance, 16.0, 4096.0),
        ], WORLD_GROUPS, WORLD_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            // The terrain had a width before it went on forever (save version 3 and older). Its
            // value is taken and ignored, so older saves and tuning files load quietly.
            "terrain_width" => {}
            "terrain_height" => self.terrain_height = value.round().clamp(1.0, 32.0) as u32,
            "sim_distance" => self.sim_distance = value.max(0.0),
            _ => return false,
        }
        true
    }
}

/// Every chunk of a terrain placed at `center`, `width` chunks across and `height` high,
/// centered on it.
pub fn terrain_chunks(center: BoxelPos, width: u32, height: u32) -> Vec<ChunkPos> {
    let center = center.chunk().0;
    let (w, h) = (width as i64, height as i64);
    let mut out = Vec::with_capacity((w * w * h) as usize);
    for z in -h / 2..h - h / 2 {
        for y in -w / 2..w - w / 2 {
            for x in -w / 2..w - w / 2 {
                out.push(ChunkPos(center + I64Vec3::new(x, y, z)));
            }
        }
    }
    out
}

/// Fills `chunks` of the terrain `def` with `generator`, spread over every core. Comes back in
/// the order asked for.
pub fn fill_chunks(
    generator: &dyn Generator,
    def: &TerrainDef,
    chunks: &[ChunkPos],
    blocks: &BlockRegistry,
) -> Result<Vec<ChunkData>, ApiError> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(chunks.len().max(1));
    let per = chunks.len().div_ceil(threads.max(1)).max(1);
    let center = def.center.0;
    let parts: Vec<Result<Vec<ChunkData>, ApiError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .chunks(per)
            .map(|part| {
                scope.spawn(move || {
                    let mut fill = ChunkFill::default();
                    part.iter()
                        .map(|&pos| {
                            fill.clear();
                            generator.fill(pos.origin().0 - center, &mut fill);
                            to_chunk(pos, &fill, blocks)
                        })
                        .collect()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("a terrain thread panicked")).collect()
    });
    let mut out = Vec::with_capacity(chunks.len());
    for part in parts {
        out.extend(part?);
    }
    Ok(out)
}

/// A generator's chunk as engine chunk data. A chunk of one block all through is stored as that
/// one value.
pub fn to_chunk(pos: ChunkPos, fill: &ChunkFill, blocks: &BlockRegistry) -> Result<ChunkData, ApiError> {
    let mut lists: Vec<Vec<Occupant>> = vec![Vec::new()];
    for name in &fill.palette {
        let id = blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.clone()))?;
        lists.push(vec![Occupant::new(id)]);
    }
    if let Some(&bad) = fill.cells.iter().find(|&&e| e as usize >= lists.len()) {
        return Err(ApiError::InvalidDef(format!("terrain chunk {:?} uses palette entry {bad}, past its end", pos.0.to_array())));
    }
    let first = fill.cells[0];
    let cells = if fill.cells.iter().all(|&e| e == first) {
        Layer::Uniform(first)
    } else {
        Layer::Dense(fill.cells.clone().into_boxed_slice())
    };
    // Light is worked out once the whole terrain is in (see `crate::light`).
    Ok(ChunkData { pos, lists, cells, light: Layer::Uniform(crate::Light::SKY), records: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, CHUNK_SIZE};

    /// Stone below z = 0 (relative to the center), plus a marker wherever x + y + z is 0 mod 7.
    struct Layered;

    impl Tunables for Layered {
        fn tunables(&self) -> Vec<Tunable> {
            Vec::new()
        }
        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl Generator for Layered {
        fn fill(&self, origin: I64Vec3, out: &mut ChunkFill) {
            let stone = out.entry("t:stone");
            let marker = out.entry("t:marker");
            let s = CHUNK_SIZE as usize;
            for z in 0..s {
                for y in 0..s {
                    for x in 0..s {
                        let p = origin + I64Vec3::new(x as i64, y as i64, z as i64);
                        if p.z < 0 {
                            out.set(x, y, z, stone);
                        } else if (p.x + p.y + p.z).rem_euclid(7) == 0 && p.z < 40 {
                            out.set(x, y, z, marker);
                        }
                    }
                }
            }
        }
    }

    fn blocks() -> BlockRegistry {
        let mut blocks = BlockRegistry::default();
        blocks.register(BlockDef::new("t:stone", true, [128, 128, 128])).unwrap();
        blocks.register(BlockDef::new("t:marker", true, [255, 0, 0])).unwrap();
        blocks
    }

    #[test]
    fn a_terrain_box_is_centered_on_its_chunk() {
        let chunks = terrain_chunks(BoxelPos::new(64, 0, -32), 4, 2);
        assert_eq!(chunks.len(), 32);
        let xs: Vec<i64> = chunks.iter().map(|c| c.0.x).collect();
        assert_eq!((xs.iter().min(), xs.iter().max()), (Some(&0), Some(&3)));
        let zs: Vec<i64> = chunks.iter().map(|c| c.0.z).collect();
        assert_eq!((zs.iter().min(), zs.iter().max()), (Some(&-2), Some(&-1)));
    }

    #[test]
    fn a_chunk_is_the_same_filled_alone_or_among_others() {
        let blocks = blocks();
        let def = TerrainDef { generator: "t:layered".into(), center: BoxelPos::new(320, -64, 0) };
        let all = terrain_chunks(def.center, 4, 4);
        let together = fill_chunks(&Layered, &def, &all, &blocks).unwrap();
        for (i, pos) in all.iter().enumerate().step_by(5) {
            let alone = fill_chunks(&Layered, &def, &[*pos], &blocks).unwrap();
            assert_eq!(alone[0], together[i], "chunk {:?}", pos.0);
        }
    }

    #[test]
    fn solid_and_empty_chunks_are_stored_as_one_value() {
        let blocks = blocks();
        let def = TerrainDef { generator: "t:layered".into(), center: BoxelPos::new(0, 0, 0) };
        let data = fill_chunks(&Layered, &def, &terrain_chunks(def.center, 2, 6), &blocks).unwrap();
        let at = |z: i64| data.iter().find(|c| c.pos.0.z == z).unwrap();
        // Below the center: all stone. Above 64: all empty. Between: markers up to z 40.
        assert_eq!(at(-2).cells, Layer::Uniform(1));
        assert_eq!(at(2).cells, Layer::Uniform(0));
        assert!(matches!(at(0).cells, Layer::Dense(_)));
        assert!(matches!(at(1).cells, Layer::Dense(_)));
        // Rows -3 to -1 and 2 are uniform, 4 chunks each.
        let uniform = data.iter().filter(|c| matches!(c.cells, Layer::Uniform(_))).count();
        assert_eq!((data.len(), uniform), (24, 16));
    }

    #[test]
    fn unknown_block_names_are_refused() {
        struct Bad;
        impl Tunables for Bad {
            fn tunables(&self) -> Vec<Tunable> {
                Vec::new()
            }
            fn set_tunable(&mut self, _: &str, _: f32) -> bool {
                false
            }
        }
        impl Generator for Bad {
            fn fill(&self, _: I64Vec3, out: &mut ChunkFill) {
                let e = out.entry("t:nope");
                out.set(0, 0, 0, e);
            }
        }
        let def = TerrainDef { generator: "t:bad".into(), center: BoxelPos::new(0, 0, 0) };
        let got = fill_chunks(&Bad, &def, &[ChunkPos(I64Vec3::ZERO)], &blocks());
        assert_eq!(got, Err(ApiError::UnknownBlock("t:nope".into())));
    }

    #[test]
    fn world_settings_round_chunk_counts() {
        let mut s = WorldSettings::default();
        assert!(s.set_tunable("terrain_height", 7.6));
        assert_eq!(s.terrain_height, 8);
        assert!(s.set_tunable("terrain_width", 11.0), "the old width is taken and ignored");
        assert!(!s.tunables().iter().any(|t| t.name == "terrain_width"));
        assert!(!s.set_tunable("nope", 1.0));
    }
}
