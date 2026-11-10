//! One layer of per-boxel data in a chunk. Chunks hold several layers side by side
//! (blocks now; fluids, light and mod fields later), each stored independently.

use api::{CHUNK_VOLUME, LocalPos};

/// Per-boxel values for one chunk. Starts uniform and only allocates once it differs.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer<T> {
    Uniform(T),
    Dense(Box<[T]>),
}

impl<T: Copy + PartialEq> Layer<T> {
    pub fn get(&self, pos: LocalPos) -> T {
        self.at(pos.index())
    }

    pub fn set(&mut self, pos: LocalPos, value: T) {
        self.set_at(pos.index(), value);
    }

    /// The value at `index` (see [`LocalPos::index`]).
    #[inline]
    pub fn at(&self, index: usize) -> T {
        match self {
            Layer::Uniform(value) => *value,
            Layer::Dense(values) => values[index],
        }
    }

    /// Sets the value at `index` (see [`LocalPos::index`]).
    #[inline]
    pub fn set_at(&mut self, index: usize, value: T) {
        match self {
            Layer::Uniform(current) if *current == value => {}
            Layer::Uniform(current) => {
                let mut values = vec![*current; CHUNK_VOLUME].into_boxed_slice();
                values[index] = value;
                *self = Layer::Dense(values);
            }
            Layer::Dense(values) => values[index] = value,
        }
    }

    /// Stores it as one value again if every boxel has come to hold the same.
    pub fn compact(&mut self) {
        if let Layer::Dense(values) = self
            && values.iter().all(|v| *v == values[0])
        {
            *self = Layer::Uniform(values[0]);
        }
    }

    /// The single value filling the whole layer, if it is stored as uniform.
    pub fn uniform(&self) -> Option<T> {
        match self {
            Layer::Uniform(value) => Some(*value),
            Layer::Dense(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_uniform_until_a_value_differs() {
        let mut layer = Layer::Uniform(0u16);
        layer.set(LocalPos::new(3, 4, 5), 0);
        assert_eq!(layer.uniform(), Some(0));

        layer.set(LocalPos::new(3, 4, 5), 7);
        assert_eq!(layer.uniform(), None);
        assert_eq!(layer.get(LocalPos::new(3, 4, 5)), 7);
        assert_eq!(layer.get(LocalPos::new(4, 4, 5)), 0);
        assert_eq!(layer.get(LocalPos::new(31, 31, 31)), 0);
    }

    #[test]
    fn dense_keeps_every_value() {
        let mut layer = Layer::Uniform(0u16);
        for i in 0..CHUNK_VOLUME {
            layer.set(LocalPos::from_index(i), (i % 1000) as u16);
        }
        for i in 0..CHUNK_VOLUME {
            assert_eq!(layer.get(LocalPos::from_index(i)), (i % 1000) as u16);
        }
    }
}
