//! World storage: a universe of grids, each made of 3D chunks, each holding layers.
//! Everything is addressed by (grid, chunk, layer), even while there is only one of each.

use std::collections::HashMap;

use api::{BoxelPos, ChunkPos};

use crate::cell::{CellId, CellTable, Occupant};
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
    /// What fills each boxel, as an ID into the grid's [`CellTable`].
    pub cells: Layer<CellId>,
}

impl Chunk {
    pub fn empty() -> Self {
        Self { cells: Layer::Uniform(CellId::EMPTY) }
    }
}

/// A volume of boxels made of chunks. Missing chunks read as empty.
#[derive(Default)]
pub struct Grid {
    chunks: HashMap<ChunkPos, Chunk>,
    cells: CellTable,
}

impl Grid {
    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn chunks(&self) -> impl Iterator<Item = (ChunkPos, &Chunk)> {
        self.chunks.iter().map(|(pos, chunk)| (*pos, chunk))
    }

    /// The occupant list a chunk's cell ID points at.
    pub fn occupants(&self, id: CellId) -> &[Occupant] {
        self.cells.get(id)
    }

    /// Everything in the boxel at `pos`, in the order it was placed, oldest first.
    pub fn cell(&self, pos: BoxelPos) -> &[Occupant] {
        let (chunk, local) = pos.split();
        let id = self.chunks.get(&chunk).map_or(CellId::EMPTY, |c| c.cells.get(local));
        self.cells.get(id)
    }

    /// Replaces everything in the boxel at `pos`. Doesn't check whether the occupants may share.
    pub fn set_cell(&mut self, pos: BoxelPos, occupants: &[Occupant]) {
        let id = self.cells.intern(occupants);
        let (chunk, local) = pos.split();
        match self.chunks.get_mut(&chunk) {
            Some(c) => c.cells.set(local, id),
            None if id.is_empty() => {}
            None => self.chunks.entry(chunk).or_insert_with(Chunk::empty).cells.set(local, id),
        }
    }

    /// The block in the boxel at `pos`: its first occupant's, or empty. Most boxels hold one.
    pub fn block(&self, pos: BoxelPos) -> BlockId {
        self.cell(pos).first().map_or(BlockId::EMPTY, |o| o.block)
    }

    /// Fills the boxel at `pos` with just `id`, unturned, or empties it.
    pub fn set_block(&mut self, pos: BoxelPos, id: BlockId) {
        self.set_cell(pos, &[Occupant::new(id)]);
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
    fn a_boxel_holds_several_occupants() {
        let stone = some_block();
        let mut grid = Grid::default();
        let p = BoxelPos::new(40, -3, 7);
        let turned = Occupant::turned(stone, api::Orientation::quarter_turns(2, 1));
        grid.set_cell(p, &[turned, Occupant::new(stone)]);
        assert_eq!(grid.cell(p).len(), 2);
        assert!(grid.cell(p).contains(&turned));
        assert!(grid.cell(BoxelPos::new(41, -3, 7)).is_empty());
        // Neighbors with the same contents share one entry in the table.
        grid.set_cell(BoxelPos::new(41, -3, 7), &[turned, Occupant::new(stone)]);
        let (chunk, local) = p.split();
        let (_, next) = BoxelPos::new(41, -3, 7).split();
        let c = grid.chunk(chunk).unwrap();
        assert_eq!(c.cells.get(local), c.cells.get(next));
        grid.set_cell(p, &[]);
        assert!(grid.cell(p).is_empty());
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
