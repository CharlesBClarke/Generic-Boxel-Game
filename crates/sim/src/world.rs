//! World storage: a universe of grids, each made of 3D chunks, each holding layers.
//! Everything is addressed by (grid, chunk, layer), even while there is only one of each.

use std::collections::HashMap;

use api::{BoxelPos, ChunkPos};

use crate::{BlockId, Layer};

/// Identifies a grid. The main world is one grid among many; sub-grids and pocket
/// dimensions will be others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridId(pub u32);

impl GridId {
    pub const MAIN: GridId = GridId(0);
}

/// A 32×32×32 cube of boxels. Each kind of data is its own layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub blocks: Layer<BlockId>,
}

impl Chunk {
    pub fn empty() -> Self {
        Self { blocks: Layer::Uniform(BlockId::EMPTY) }
    }
}

/// A volume of boxels made of chunks. Missing chunks read as empty.
#[derive(Default)]
pub struct Grid {
    chunks: HashMap<ChunkPos, Chunk>,
}

impl Grid {
    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn chunks(&self) -> impl Iterator<Item = (ChunkPos, &Chunk)> {
        self.chunks.iter().map(|(pos, chunk)| (*pos, chunk))
    }

    pub fn block(&self, pos: BoxelPos) -> BlockId {
        let (chunk, local) = pos.split();
        self.chunks.get(&chunk).map_or(BlockId::EMPTY, |c| c.blocks.get(local))
    }

    pub fn set_block(&mut self, pos: BoxelPos, id: BlockId) {
        let (chunk, local) = pos.split();
        match self.chunks.get_mut(&chunk) {
            Some(c) => c.blocks.set(local, id),
            None if id.is_empty() => {}
            None => self.chunks.entry(chunk).or_insert_with(Chunk::empty).blocks.set(local, id),
        }
    }
}

/// Every grid in the world.
pub struct Universe {
    grids: HashMap<GridId, Grid>,
}

impl Default for Universe {
    fn default() -> Self {
        Self { grids: HashMap::from([(GridId::MAIN, Grid::default())]) }
    }
}

impl Universe {
    pub fn grid(&self, id: GridId) -> Option<&Grid> {
        self.grids.get(&id)
    }

    pub fn grid_mut(&mut self, id: GridId) -> Option<&mut Grid> {
        self.grids.get_mut(&id)
    }

    pub fn main(&self) -> &Grid {
        &self.grids[&GridId::MAIN]
    }

    pub fn main_mut(&mut self) -> &mut Grid {
        self.grids.get_mut(&GridId::MAIN).expect("the main grid always exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some_block() -> BlockId {
        let mut reg = crate::BlockRegistry::default();
        reg.register(api::BlockDef::new("test:stone", true, [0, 0, 0])).unwrap()
    }

    #[test]
    fn reads_back_across_chunk_boundaries() {
        let stone = some_block();
        let mut grid = Grid::default();
        let spots = [
            BoxelPos::new(0, 0, 0),
            BoxelPos::new(31, 31, 31),
            BoxelPos::new(32, 0, 0),
            BoxelPos::new(-1, -1, -1),
            BoxelPos::new(-33, 64, -65),
            BoxelPos::new(3_000_000, -2_000_000, 1_000),
        ];
        for p in spots {
            grid.set_block(p, stone);
        }
        for p in spots {
            assert_eq!(grid.block(p), stone, "{p:?}");
        }
        assert_eq!(grid.block(BoxelPos::new(1, 0, 0)), BlockId::EMPTY);
        assert_eq!(grid.block(BoxelPos::new(-2, -1, -1)), BlockId::EMPTY);
    }

    #[test]
    fn chunks_are_created_only_when_needed() {
        let stone = some_block();
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(5, 5, 5), BlockId::EMPTY);
        assert_eq!(grid.chunks().count(), 0);
        grid.set_block(BoxelPos::new(5, 5, 5), stone);
        grid.set_block(BoxelPos::new(-5, 5, 5), stone);
        assert_eq!(grid.chunks().count(), 2);
        assert!(grid.chunk(ChunkPos::new(-1, 0, 0)).is_some());
    }

    #[test]
    fn universe_starts_with_a_main_grid() {
        let mut universe = Universe::default();
        assert!(universe.grid(GridId::MAIN).is_some());
        assert!(universe.grid(GridId(1)).is_none());
        let stone = some_block();
        universe.main_mut().set_block(BoxelPos::new(1, 2, 3), stone);
        assert_eq!(universe.main().block(BoxelPos::new(1, 2, 3)), stone);
    }
}
