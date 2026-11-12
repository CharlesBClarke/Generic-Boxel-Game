//! Storage and menders: materials kept in the world, and pieces that put back what's gone
//! around them (see [`api::Property::Storage`] and [`api::Property::Mender`]).
//!
//! **Records.** A boxel can keep a [`Record`] beside its occupants, stored with its chunk (so it's
//! saved, sent to clients and stored when its column unloads): what its storage holds, and what
//! menders remember of its texels.
//!
//! **Storage.** A loose item touching a storage piece goes into that piece's boxel, as the
//! materials it's made of (see [`materials_of`]). When a boxel's last storage piece goes, what
//! it held drops as loose items (see [`orphaned`]).
//!
//! **Memory.** Every texel within the mend reach of a mender piece, edges and corners
//! counting, remembers what was last in it: the piece it was part of (the occupant it was cut
//! from, if it was cut). Texels filled now are remembered as they are; empty ones keep what
//! they remembered; texels no mender reaches any more are forgotten, so when a mender piece
//! goes, what only it reached is forgotten (see [`remember`]). A mender's own texels are within
//! its reach, so pieces of a mender remember each other.
//!
//! **Mending.** A signal to a boxel with a mender piece mends the piece's group (every mender
//! piece of its material touching it face to face): every texel the group remembers that's
//! gone comes back, as part of the piece it was part of, joining what's left of that piece. It's
//! paid for from the storage the same signal reached, a texel of the material a texel: every
//! storage group (storage pieces of one material touching face to face) with a piece in a
//! boxel the signal reached, so a bin of several blocks pays as one. All or nothing (see [`plan`]): if the storage is short, or something solid or a body is
//! in a texel's place, nothing comes back.

use std::collections::{HashMap, HashSet};

use api::{BoxelPos, I64Vec3, Property, ShapeGrid};

use crate::signal::{Piece, Texels, Undo, group_with, has_property, join, part, texels};
use crate::{BlockRegistry, Drop, Grid, MAX_OCCUPANTS, MaterialId, Occupant};

/// What a boxel keeps besides its occupants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    /// What its storage holds: amounts of materials, in texels, by material, each listed once.
    pub stored: Vec<(MaterialId, u32)>,
    /// What menders remember of its texels: each piece remembered (the occupant it was cut
    /// from, never a cut part) and the texels it last filled here, each piece listed once.
    pub memory: Vec<(Occupant, ShapeGrid)>,
}

impl Record {
    pub fn is_empty(&self) -> bool {
        self.stored.is_empty() && self.memory.is_empty()
    }

    /// How much of `material` it holds.
    pub fn amount(&self, material: MaterialId) -> u32 {
        self.stored.iter().find(|(m, _)| *m == material).map_or(0, |(_, a)| *a)
    }

    /// Puts `amount` of `material` in.
    pub fn add(&mut self, material: MaterialId, amount: u32) {
        if amount == 0 {
            return;
        }
        match self.stored.iter_mut().find(|(m, _)| *m == material) {
            Some((_, a)) => *a = a.saturating_add(amount),
            None => self.stored.push((material, amount)),
        }
    }

    /// Takes up to `amount` of `material` out, returning how much it took.
    pub fn take(&mut self, material: MaterialId, amount: u32) -> u32 {
        let Some(i) = self.stored.iter().position(|(m, _)| *m == material) else {
            return 0;
        };
        let took = self.stored[i].1.min(amount);
        self.stored[i].1 -= took;
        if self.stored[i].1 == 0 {
            self.stored.remove(i);
        }
        took
    }

    /// How many texels it remembers, every piece together.
    pub fn remembered(&self) -> u32 {
        self.memory.iter().map(|(_, s)| s.volume()).sum()
    }
}

/// The materials `drop` is made of, in texels: a part's material, each piece's material for a
/// bundle, or the material itself.
pub fn materials_of(blocks: &BlockRegistry, drop: Drop) -> Vec<(MaterialId, u32)> {
    let pieces = blocks.pieces(drop.material);
    if !pieces.is_empty() {
        let count = drop.amount / blocks.unit(drop.material).max(1);
        return pieces
            .iter()
            .filter_map(|&b| Some((blocks.made_of(b)?, blocks.shape(b).volume().saturating_mul(count))))
            .collect();
    }
    vec![(blocks.part_of(drop.material).unwrap_or(drop.material), drop.amount)]
}

/// Whether a piece in the boxel at `pos` has `property`, shown or hidden.
fn holds(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos, property: Property) -> bool {
    grid.cell(pos).iter().any(|&o| has_property(blocks, o, property))
}

/// Every boxel within `r` boxels of `pos`, edges and corners counting.
fn around(pos: BoxelPos, r: i64) -> impl Iterator<Item = BoxelPos> {
    (-r..=r).flat_map(move |x| (-r..=r).flat_map(move |y| (-r..=r).map(move |z| pos.offset(I64Vec3::new(x, y, z)))))
}

/// Boxels in a fixed order, each once.
fn sorted(set: HashSet<BoxelPos>) -> Vec<BoxelPos> {
    let mut out: Vec<BoxelPos> = set.into_iter().collect();
    out.sort_by_key(|p| p.0.to_array());
    out
}

/// Brings what menders remember up to date after the boxels in `changed` changed: in each boxel
/// a mender can reach from them, filled texels within `reach` of a mender piece are remembered
/// as they are now, and texels out of every mender's reach are forgotten (see the module).
/// `reach` is in texels, at most a boxel. Returns the boxels whose memory changed.
pub fn remember(grid: &mut Grid, blocks: &BlockRegistry, changed: &[BoxelPos], reach: i32) -> Vec<BoxelPos> {
    let reach = reach.clamp(0, 16);
    // A change reaches a boxel beside it (a new mender remembers its neighbors); the menders that
    // reach those are a boxel further.
    let affected: HashSet<BoxelPos> = changed.iter().flat_map(|&p| around(p, 1)).collect();
    let near: HashSet<BoxelPos> = affected.iter().flat_map(|&p| around(p, 1)).collect();
    let mut menders = Texels::default();
    for &p in &near {
        for &o in grid.cell(p) {
            if has_property(blocks, o, Property::Mender) {
                menders.add(p, &texels(blocks, o));
            }
        }
    }
    let reached = menders.grown(reach);
    let mut out = Vec::new();
    for p in sorted(affected) {
        let mask = reached.get(p);
        let old = grid.record(p).map(|r| r.memory.clone()).unwrap_or_default();
        if mask.is_empty() && old.is_empty() {
            continue;
        }
        let memory = remembered(blocks, grid.cell(p), &old, &mask);
        if memory != old {
            let mut record = grid.record(p).cloned().unwrap_or_default();
            record.memory = memory;
            grid.set_record(p, record);
            out.push(p);
        }
    }
    out
}

/// What a boxel holding `cell` remembers within `mask`, having remembered `old`: what's there
/// now where texels are filled, what it remembered where they're empty.
fn remembered(
    blocks: &BlockRegistry,
    cell: &[Occupant],
    old: &[(Occupant, ShapeGrid)],
    mask: &ShapeGrid,
) -> Vec<(Occupant, ShapeGrid)> {
    let mut now: Vec<(Occupant, ShapeGrid)> = Vec::new();
    let mut filled = ShapeGrid::EMPTY;
    for &o in cell {
        let (from, shape) = blocks.origin(o);
        let inside = shape.intersection(mask);
        if inside.is_empty() {
            continue;
        }
        filled = filled.union(&inside);
        add_to(&mut now, from, &inside);
    }
    let mut out: Vec<(Occupant, ShapeGrid)> = Vec::new();
    for (from, shape) in old {
        let kept = shape.intersection(mask).minus(&filled);
        if !kept.is_empty() {
            add_to(&mut out, *from, &kept);
        }
    }
    for (from, shape) in &now {
        add_to(&mut out, *from, shape);
    }
    out
}

fn add_to(list: &mut Vec<(Occupant, ShapeGrid)>, from: Occupant, shape: &ShapeGrid) {
    match list.iter_mut().find(|(f, _)| *f == from) {
        Some((_, s)) => *s = s.union(shape),
        None => list.push((from, shape.clone())),
    }
}

/// The boxels among `changed` that hold something in storage but no storage piece any more,
/// with what each held, emptied out of their records. The caller drops it.
pub fn orphaned(grid: &mut Grid, blocks: &BlockRegistry, changed: &[BoxelPos]) -> Vec<(BoxelPos, Vec<(MaterialId, u32)>)> {
    let mut out = Vec::new();
    for p in sorted(changed.iter().copied().collect()) {
        let Some(record) = grid.record(p).filter(|r| !r.stored.is_empty()) else {
            continue;
        };
        if holds(grid, blocks, p, Property::Storage) {
            continue;
        }
        let mut record = record.clone();
        out.push((p, std::mem::take(&mut record.stored)));
        grid.set_record(p, record);
    }
    out
}

/// A mend worked out: what comes back and what it costs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mend {
    /// Each boxel, the piece remembered there, and its texels that come back.
    pub put: Vec<(BoxelPos, Occupant, ShapeGrid)>,
    /// What it costs, by material, in texels.
    pub cost: Vec<(MaterialId, u32)>,
    /// How many texels come back.
    pub texels: u32,
    /// How many texels the menders remember in all, gone or not.
    pub remembered: u32,
    /// The boxels of the storage that pays, in a fixed order.
    pub storage: Vec<BoxelPos>,
}

/// Why a mend puts nothing back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MendRefusal {
    /// More texels would come back than one signal may put back.
    TooMuch { texels: u32, limit: u32 },
    /// The storage the signal reached holds too little: by how much of each material.
    Short(Vec<(MaterialId, u32)>),
    /// Something solid or a body is in the place of a texel that would come back, or a boxel has
    /// no room for another occupant.
    Blocked,
}

/// Works out the mend a signal to the boxels in `reached` makes (see the module): `reach` in
/// texels, at most `limit` texels; `clear` says whether bodies leave room for solid texels
/// `shape` in the boxel at a position. An empty mend if nothing's gone. Changes nothing.
pub fn plan(
    grid: &Grid,
    blocks: &BlockRegistry,
    reached: &[BoxelPos],
    reach: i32,
    limit: u32,
    clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool,
) -> Result<Mend, MendRefusal> {
    let (mend, blocked) = survey(grid, blocks, reached, reach, clear);
    if mend.texels == 0 {
        return Ok(mend);
    }
    if mend.texels > limit {
        return Err(MendRefusal::TooMuch { texels: mend.texels, limit });
    }
    let short: Vec<(MaterialId, u32)> = mend
        .cost
        .iter()
        .filter_map(|&(m, need)| {
            let have: u32 = mend.storage.iter().filter_map(|&p| grid.record(p)).map(|r| r.amount(m)).sum();
            (have < need).then(|| (m, need - have))
        })
        .collect();
    if !short.is_empty() {
        return Err(MendRefusal::Short(short));
    }
    if blocked {
        return Err(MendRefusal::Blocked);
    }
    Ok(mend)
}

/// What a signal to the boxels in `reached` would put back and what it would cost, whether or
/// not it can be paid for, and whether anything is in the way (see [`plan`]).
pub fn survey(
    grid: &Grid,
    blocks: &BlockRegistry,
    reached: &[BoxelPos],
    reach: i32,
    clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool,
) -> (Mend, bool) {
    let mut taken: HashSet<Piece> = HashSet::new();
    let mut area = Texels::default();
    let mut storage: HashSet<BoxelPos> = HashSet::new();
    for &pos in reached {
        for &o in grid.cell(pos) {
            let piece = Piece { pos, occupant: o };
            if taken.contains(&piece) || blocks.is_hidden(o.block) {
                continue;
            }
            if has_property(blocks, o, Property::Mender) {
                for p in group_with(grid, blocks, piece, Property::Mender) {
                    area.add(p.pos, &texels(blocks, p.occupant));
                    taken.insert(p);
                }
            }
            if has_property(blocks, o, Property::Storage) {
                for p in group_with(grid, blocks, piece, Property::Storage) {
                    if !blocks.is_hidden(p.occupant.block) {
                        storage.insert(p.pos);
                    }
                    taken.insert(p);
                }
            }
        }
    }
    let mut within: Vec<(BoxelPos, ShapeGrid)> = area.grown(reach.clamp(0, 16)).iter().map(|(p, s)| (p, s.clone())).collect();
    within.sort_by_key(|(p, _)| p.0.to_array());

    let mut mend = Mend { storage: sorted(storage), ..Mend::default() };
    let mut cost: HashMap<MaterialId, u32> = HashMap::new();
    let mut blocked = false;
    for (pos, mask) in within {
        let Some(record) = grid.record(pos) else {
            continue;
        };
        let cell = grid.cell(pos);
        let mut added = 0;
        for (from, shape) in &record.memory {
            let want = shape.intersection(&mask);
            mend.remembered += want.volume();
            let mut have = ShapeGrid::EMPTY;
            for &o in cell {
                let (f, s) = blocks.origin(o);
                if f == *from {
                    have = have.union(&s);
                }
            }
            let missing = want.minus(&have);
            // A block that drops nothing can't be paid for, so it isn't put back.
            let Some(material) = blocks.made_of(from.block) else {
                continue;
            };
            if missing.is_empty() {
                continue;
            }
            if blocks.is_solid(from.block) {
                let in_place = cell
                    .iter()
                    .any(|&o| blocks.origin(o).0 != *from && blocks.is_solid(o.block) && texels(blocks, o).overlaps(&missing));
                if in_place || !clear(pos, &missing) {
                    blocked = true;
                }
            }
            // What comes back joins a shown part of its piece; otherwise it's one more occupant.
            if !cell.iter().any(|&o| !blocks.is_hidden(o.block) && blocks.origin(o).0 == *from) {
                added += 1;
            }
            let n = missing.volume();
            *cost.entry(material).or_default() += n;
            mend.texels += n;
            mend.put.push((pos, *from, missing));
        }
        if cell.len() + added > MAX_OCCUPANTS {
            blocked = true;
        }
    }
    mend.cost = cost.into_iter().collect();
    mend.cost.sort_by_key(|(m, _)| m.index());
    (mend, blocked)
}

/// Puts back what `mend` brings back, keeping what it changes in `undo`. Doesn't pay for it
/// (see [`pay`]). Fails, changing nothing it hasn't kept in `undo`, if a part can't be made.
pub fn apply(grid: &mut Grid, blocks: &mut BlockRegistry, mend: &Mend, undo: &mut Undo) -> Result<(), crate::signal::Blocked> {
    for (pos, from, shape) in &mend.put {
        let mut cell = grid.cell(*pos).to_vec();
        cell.push(part(blocks, *from, shape, false).ok_or(crate::signal::Blocked)?);
        join(&mut cell, blocks);
        undo.save(grid, *pos);
        grid.set_cell(*pos, &cell);
    }
    Ok(())
}

/// Takes what `mend` costs from its storage, boxel by boxel in its order. Returns the boxels
/// whose storage changed.
pub fn pay(grid: &mut Grid, mend: &Mend) -> Vec<BoxelPos> {
    let mut changed = Vec::new();
    for &(material, need) in &mend.cost {
        let mut need = need;
        for &p in &mend.storage {
            if need == 0 {
                break;
            }
            let Some(mut record) = grid.record(p).cloned() else {
                continue;
            };
            let took = record.take(material, need);
            if took > 0 {
                need -= took;
                grid.set_record(p, record);
                if !changed.contains(&p) {
                    changed.push(p);
                }
            }
        }
    }
    changed
}

/// Puts `materials` in the storage of the boxel at `pos`.
pub fn store(grid: &mut Grid, pos: BoxelPos, materials: &[(MaterialId, u32)]) {
    let mut record = grid.record(pos).cloned().unwrap_or_default();
    for &(m, amount) in materials {
        record.add(m, amount);
    }
    grid.set_record(pos, record);
}

/// Whether the boxel at `pos` holds a shown storage piece.
pub fn is_storage(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos) -> bool {
    grid.cell(pos).iter().any(|&o| !blocks.is_hidden(o.block) && has_property(blocks, o, Property::Storage))
}

/// Every boxel holding a piece with `property`, shown or hidden, in a fixed order.
pub fn holding(grid: &Grid, blocks: &BlockRegistry, property: Property) -> Vec<BoxelPos> {
    let mut known: HashMap<crate::CellId, bool> = HashMap::new();
    let mut has = |id: crate::CellId| *known.entry(id).or_insert_with(|| grid.occupants(id).iter().any(|&o| has_property(blocks, o, property)));
    let mut out = Vec::new();
    for (pos, chunk) in grid.chunks() {
        match &chunk.cells {
            crate::Layer::Uniform(id) => {
                if has(*id) {
                    out.extend((0..api::CHUNK_VOLUME).map(|i| pos.boxel(api::LocalPos::from_index(i))));
                }
            }
            crate::Layer::Dense(ids) => {
                for (i, &id) in ids.iter().enumerate() {
                    if has(id) {
                        out.push(pos.boxel(api::LocalPos::from_index(i)));
                    }
                }
            }
        }
    }
    out.sort_by_key(|p| p.0.to_array());
    out
}

/// How close a loose item's box has to come to a storage piece to count as touching it, in
/// meters.
const TOUCH: f32 = 0.02;

/// The boxel of a storage piece a loose item with its box's center at `center`, `extent`
/// across, touches, if any: the one whose middle is nearest the item's center, the lowest
/// first if two are as near.
pub fn storage_touching(grid: &Grid, blocks: &BlockRegistry, center: api::WorldPos, extent: api::Vec3) -> Option<BoxelPos> {
    let half = extent * 0.5 + api::Vec3::splat(TOUCH);
    let lo = center.translate(-half).boxel();
    let hi = center.translate(half).boxel();
    let mut best: Option<(f32, BoxelPos)> = None;
    for x in lo.0.x..=hi.0.x {
        for y in lo.0.y..=hi.0.y {
            for z in lo.0.z..=hi.0.z {
                let pos = BoxelPos::new(x, y, z);
                if !is_storage(grid, blocks, pos) {
                    continue;
                }
                let rel = api::WorldPos::at_boxel(pos).delta_to(center);
                let (min, max) = (rel - half, rel + half);
                let touching = grid
                    .cell(pos)
                    .iter()
                    .filter(|&&o| !blocks.is_hidden(o.block) && has_property(blocks, o, Property::Storage))
                    .flat_map(|&o| crate::occupant_boxes(o, blocks))
                    .any(|b| min.cmplt(b.max).all() && max.cmpgt(b.min).all());
                if !touching {
                    continue;
                }
                let d = (rel - api::Vec3::splat(0.5)).length_squared();
                if best.is_none_or(|(e, _)| d < e) {
                    best = Some((d, pos));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockId;
    use api::{BlockBox, BlockDef};

    /// Stone, dirt, mendite and cachite.
    struct Kit {
        blocks: BlockRegistry,
        grid: Grid,
        stone: BlockId,
        dirt: BlockId,
        mendite: BlockId,
        cachite: BlockId,
    }

    fn kit() -> Kit {
        let mut blocks = BlockRegistry::default();
        let mut def = |n: &str, p: Vec<Property>| {
            blocks.register(BlockDef::new(format!("t:{n}"), true, [0; 3]).with_material(format!("t:{n}")).with_properties(p)).unwrap()
        };
        let stone = def("stone", vec![]);
        let dirt = def("dirt", vec![]);
        let mendite = def("mendite", vec![Property::Mender]);
        let cachite = def("cachite", vec![Property::Storage]);
        Kit { blocks, grid: Grid::default(), stone, dirt, mendite, cachite }
    }

    fn at(x: i64, y: i64, z: i64) -> BoxelPos {
        BoxelPos::new(x, y, z)
    }

    fn always(_: BoxelPos, _: &ShapeGrid) -> bool {
        true
    }

    fn grid(min: [u8; 3], max: [u8; 3]) -> ShapeGrid {
        ShapeGrid::from_boxes(&[BlockBox::new(min, max)])
    }

    impl Kit {
        fn put(&mut self, pos: BoxelPos, block: BlockId, reach: i32) {
            let mut cell = self.grid.cell(pos).to_vec();
            cell.push(Occupant::new(block));
            self.grid.set_cell(pos, &cell);
            remember(&mut self.grid, &self.blocks, &[pos], reach);
        }

        fn clear(&mut self, pos: BoxelPos, reach: i32) {
            self.grid.set_cell(pos, &[]);
            remember(&mut self.grid, &self.blocks, &[pos], reach);
        }

        fn material(&self, block: BlockId) -> MaterialId {
            self.blocks.made_of(block).unwrap()
        }

        /// A mender at the origin, a stone east of it, and a bin (with `stone` texels of stone)
        /// north of the mender, both in `reached`.
        fn bench(stone: u32, reach: i32) -> Kit {
            let mut k = kit();
            k.put(at(0, 0, 0), k.mendite, reach);
            k.put(at(1, 0, 0), k.stone, reach);
            k.put(at(0, 1, 0), k.cachite, reach);
            let m = k.material(k.stone);
            store(&mut k.grid, at(0, 1, 0), &[(m, stone)]);
            k
        }

        fn mend(&mut self, reach: i32, clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool) -> Result<Mend, MendRefusal> {
            let reached = [at(0, 0, 0), at(0, 1, 0)];
            let mend = plan(&self.grid, &self.blocks, &reached, reach, 1 << 20, clear)?;
            apply(&mut self.grid, &mut self.blocks, &mend, &mut Undo::default()).unwrap();
            pay(&mut self.grid, &mend);
            remember(&mut self.grid, &self.blocks, &mend.put.iter().map(|p| p.0).collect::<Vec<_>>(), reach);
            Ok(mend)
        }
    }

    /// The texel layer of the boxel east of the mender that touches it.
    fn skin() -> ShapeGrid {
        grid([0, 0, 0], [1, 16, 16])
    }

    #[test]
    fn a_mender_remembers_what_was_there_before_it_and_what_comes_after() {
        let mut k = kit();
        k.put(at(1, 0, 0), k.stone, 1);
        k.put(at(0, 0, 0), k.mendite, 1);
        let stone = Occupant::new(k.stone);
        assert_eq!(k.grid.record(at(1, 0, 0)).unwrap().memory, [(stone, skin())], "a texel deep, edges and corners too");
        // Corner to corner, a single texel.
        k.put(at(1, 1, 1), k.stone, 1);
        assert_eq!(k.grid.record(at(1, 1, 1)).unwrap().remembered(), 1);
        // Far off: nothing.
        k.put(at(3, 0, 0), k.stone, 1);
        assert!(k.grid.record(at(3, 0, 0)).is_none());
        // The mender remembers itself.
        assert_eq!(k.grid.record(at(0, 0, 0)).unwrap().remembered(), 4096);
    }

    #[test]
    fn a_texel_remembers_what_was_last_in_it() {
        let mut k = Kit::bench(0, 1);
        k.clear(at(1, 0, 0), 1);
        let stone = Occupant::new(k.stone);
        assert_eq!(k.grid.record(at(1, 0, 0)).unwrap().memory, [(stone, skin())], "gone, still remembered");
        k.put(at(1, 0, 0), k.dirt, 1);
        assert_eq!(k.grid.record(at(1, 0, 0)).unwrap().memory, [(Occupant::new(k.dirt), skin())], "dirt now");
        assert_eq!(k.mend(1, &always).unwrap().texels, 0, "nothing gone");
    }

    #[test]
    fn mending_puts_back_exactly_what_touched_and_pays_for_it() {
        let mut k = Kit::bench(1000, 1);
        k.clear(at(1, 0, 0), 1);
        let mend = k.mend(1, &always).unwrap();
        assert_eq!(mend.texels, 256);
        let back = k.grid.cell(at(1, 0, 0)).to_vec();
        assert_eq!(back.len(), 1);
        let d = k.blocks.derived(back[0].block).expect("a cut part of the stone");
        assert_eq!((d.from, &d.shape, d.hidden), (Occupant::new(k.stone), &skin(), false));
        assert!(k.blocks.is_solid(back[0].block));
        let m = k.material(k.stone);
        assert_eq!(k.grid.record(at(0, 1, 0)).unwrap().amount(m), 1000 - 256);
        // Again: nothing more is gone.
        assert_eq!(k.mend(1, &always).unwrap().texels, 0);
    }

    #[test]
    fn a_whole_piece_comes_back_whole_when_the_reach_covers_it() {
        let mut k = Kit::bench(4096, 16);
        k.clear(at(1, 0, 0), 16);
        k.mend(16, &always).unwrap();
        assert_eq!(k.grid.cell(at(1, 0, 0)), [Occupant::new(k.stone)], "the stone itself, not a part");
    }

    #[test]
    fn what_comes_back_joins_what_is_left_of_its_piece() {
        let mut k = Kit::bench(4096, 1);
        // The stone's west half is gone; what's left is a cut part of it.
        let stone = Occupant::new(k.stone);
        let east = k.blocks.derive(stone, &grid([8, 0, 0], [16, 16, 16]), false).unwrap();
        k.grid.set_cell(at(1, 0, 0), &[Occupant::new(east)]);
        remember(&mut k.grid, &k.blocks, &[at(1, 0, 0)], 1);
        k.mend(1, &always).unwrap();
        let cell = k.grid.cell(at(1, 0, 0)).to_vec();
        assert_eq!(cell.len(), 1, "one piece");
        assert_eq!(k.blocks.origin(cell[0]), (stone, grid([8, 0, 0], [16, 16, 16]).union(&skin())));
    }

    #[test]
    fn a_mend_is_all_or_nothing() {
        let mut k = Kit::bench(255, 1);
        k.clear(at(1, 0, 0), 1);
        k.put(at(0, -1, 0), k.stone, 1);
        k.clear(at(0, -1, 0), 1);
        let m = k.material(k.stone);
        assert_eq!(k.mend(1, &always), Err(MendRefusal::Short(vec![(m, 256 * 2 - 255)])));
        assert!(k.grid.cell(at(1, 0, 0)).is_empty() && k.grid.cell(at(0, -1, 0)).is_empty(), "neither came back");
        assert_eq!(k.grid.record(at(0, 1, 0)).unwrap().amount(m), 255, "nothing paid");

        let mut k = Kit::bench(4096, 1);
        k.clear(at(1, 0, 0), 1);
        assert_eq!(k.mend(1, &|_, _| false), Err(MendRefusal::Blocked), "a body in the way");
        let reached = [at(0, 0, 0), at(0, 1, 0)];
        assert_eq!(plan(&k.grid, &k.blocks, &reached, 1, 100, &always), Err(MendRefusal::TooMuch { texels: 256, limit: 100 }));
    }

    #[test]
    fn storage_the_signal_does_not_reach_pays_nothing() {
        let mut k = Kit::bench(4096, 1);
        k.clear(at(1, 0, 0), 1);
        let m = k.material(k.stone);
        let only_mender = plan(&k.grid, &k.blocks, &[at(0, 0, 0)], 1, 1 << 20, &always);
        assert_eq!(only_mender, Err(MendRefusal::Short(vec![(m, 256)])));
        // A bin of two touching blocks pays as one, reached through either.
        k.put(at(0, 2, 0), k.cachite, 1);
        store(&mut k.grid, at(0, 2, 0), &[(m, 50)]);
        let mut bin = k.grid.record(at(0, 1, 0)).unwrap().clone();
        bin.take(m, 4096);
        bin.add(m, 210);
        k.grid.set_record(at(0, 1, 0), bin);
        let mend = plan(&k.grid, &k.blocks, &[at(0, 0, 0), at(0, 2, 0)], 1, 1 << 20, &always).unwrap();
        assert_eq!(mend.storage, [at(0, 1, 0), at(0, 2, 0)]);
        pay(&mut k.grid, &mend);
        assert_eq!(k.grid.record(at(0, 1, 0)).unwrap().amount(m), 0);
        assert_eq!(k.grid.record(at(0, 2, 0)).unwrap().amount(m), 4);
    }

    #[test]
    fn breaking_the_mender_forgets_what_only_it_reached() {
        let mut k = Kit::bench(0, 1);
        k.put(at(-1, 0, 0), k.mendite, 1);
        k.clear(at(0, 0, 0), 1);
        assert!(k.grid.record(at(1, 0, 0)).is_none(), "the stone two boxels from the other mender");
        let left = k.grid.record(at(0, 0, 0)).unwrap();
        assert_eq!(left.memory, [(Occupant::new(k.mendite), skin())], "the other mender remembers its neighbor's skin");
    }

    #[test]
    fn storage_with_no_storage_piece_left_gives_up_what_it_held() {
        let mut k = Kit::bench(300, 1);
        k.grid.set_cell(at(0, 1, 0), &[]);
        let m = k.material(k.stone);
        assert_eq!(orphaned(&mut k.grid, &k.blocks, &[at(0, 1, 0)]), [(at(0, 1, 0), vec![(m, 300)])]);
        assert!(k.grid.record(at(0, 1, 0)).is_none_or(|r| r.stored.is_empty()));
        assert!(orphaned(&mut k.grid, &k.blocks, &[at(0, 1, 0)]).is_empty(), "once");
    }

    #[test]
    fn an_item_goes_into_storage_as_its_materials() {
        let mut k = kit();
        let m = k.material(k.stone);
        let half = k.blocks.shaped(m, &grid([0, 0, 0], [16, 16, 8])).unwrap();
        let part = k.blocks.drop(half).unwrap();
        assert_eq!(materials_of(&k.blocks, part), [(m, 2048)]);
        let d = k.material(k.dirt);
        let bundle = k.blocks.bundle(&[(m, grid([0, 0, 0], [16, 16, 8])), (d, grid([0, 0, 8], [16, 16, 16]))]).unwrap();
        let two = Drop { material: bundle, amount: k.blocks.unit(bundle) * 2 };
        assert_eq!(materials_of(&k.blocks, two), [(m, 4096), (d, 4096)]);
        assert_eq!(materials_of(&k.blocks, Drop { material: m, amount: 7 }), [(m, 7)]);
    }

    #[test]
    fn an_item_touches_storage_resting_on_it_or_against_its_side() {
        let mut k = kit();
        k.put(at(0, 0, 0), k.cachite, 1);
        let size = api::Vec3::splat(0.25);
        let on_top = api::WorldPos::new(I64Vec3::new(0, 0, 1), api::Vec3::new(0.5, 0.5, 0.125));
        assert_eq!(storage_touching(&k.grid, &k.blocks, on_top, size), Some(at(0, 0, 0)));
        let beside = api::WorldPos::new(I64Vec3::new(1, 0, 0), api::Vec3::new(0.13, 0.5, 0.5));
        assert_eq!(storage_touching(&k.grid, &k.blocks, beside, size), Some(at(0, 0, 0)));
        let apart = api::WorldPos::new(I64Vec3::new(1, 0, 0), api::Vec3::new(0.3, 0.5, 0.5));
        assert_eq!(storage_touching(&k.grid, &k.blocks, apart, size), None);
    }
}
