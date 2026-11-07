//! Block definitions.

use crate::BlockSounds;

/// A kind of block, registered by name. Names are the identity; any numeric IDs the
/// engine assigns at runtime are private to it.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    /// Namespaced name, such as `"base:stone"`.
    pub name: String,
    /// Whether players and other bodies collide with it.
    pub solid: bool,
    /// Flat color, sRGB. Used where the block has no look, and wherever it's drawn as one color.
    pub color: [u8; 3],
    /// The registered look it's drawn with, if any: a solid volume, so any cut through the
    /// block shows what's inside.
    pub look: Option<String>,
    /// How much grip the surface gives a body standing on it. 1 is normal, near 0 is ice.
    /// Movement controllers decide what it means exactly.
    pub friction: f32,
    /// Whether players can place it. Blocks that only appear through world building or game
    /// rules set this to false.
    pub placeable: bool,
    /// The boxes it fills, unturned. Bodies collide with them if the block is solid, and aiming
    /// hits them. One full box for an ordinary block.
    pub shape: Vec<BlockBox>,
    /// Which turns it takes when placed. Every block is placed by the same rule: its bottom
    /// against the face aimed at, its front (+Y) the way the player looks.
    pub turning: Turning,
    /// What breaking it gives: its volume of this material, as a loose item. Namespaced, like
    /// `"base:stone"`; blocks of different shapes share one, so a slab gives half a stone.
    /// `None` gives nothing.
    pub material: Option<String>,
    /// What it sounds like underfoot, breaking and being placed.
    pub sounds: BlockSounds,
    /// What it does, brought by its material into every shape made of it (see [`Property`]).
    pub properties: Vec<Property>,
}

/// Something a material does wherever a piece of it is, written by hand and given to materials
/// by mods. A boxel's occupants act together as one block: a signal reaches the boxel, and
/// every piece in it answers. A fixed set for now; mods can't add their own yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    /// Pressing interact aimed at a piece of it signals its boxel, and every touching boxel
    /// with a listener in it. A knob, a lever.
    Trigger,
    /// A signaled boxel holding a piece of it swings a quarter turn about the piece's long
    /// axis, with every boxel along the same hinge line, and back again on the next signal.
    Hinge,
    /// Takes signals from triggers in the boxels it touches. Passes nothing on.
    Listener,
}

impl Property {
    /// Its name in data files.
    pub fn name(self) -> &'static str {
        match self {
            Property::Trigger => "trigger",
            Property::Hinge => "hinge",
            Property::Listener => "listener",
        }
    }

    /// The property named `name` in data files.
    pub fn from_name(name: &str) -> Option<Property> {
        [Property::Trigger, Property::Hinge, Property::Listener].into_iter().find(|p| p.name() == name)
    }
}

impl BlockDef {
    /// A placeable block with normal friction.
    pub fn new(name: impl Into<String>, solid: bool, color: [u8; 3]) -> Self {
        Self {
            name: name.into(),
            solid,
            color,
            look: None,
            friction: 1.0,
            placeable: true,
            shape: vec![BlockBox::FULL],
            turning: Turning::None,
            material: None,
            sounds: BlockSounds::default(),
            properties: Vec::new(),
        }
    }

    pub fn with_look(mut self, look: impl Into<String>) -> Self {
        self.look = Some(look.into());
        self
    }

    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    pub fn with_placeable(mut self, placeable: bool) -> Self {
        self.placeable = placeable;
        self
    }

    pub fn with_shape(mut self, shape: Vec<BlockBox>) -> Self {
        self.shape = shape;
        self
    }

    pub fn with_turning(mut self, turning: Turning) -> Self {
        self.turning = turning;
        self
    }

    pub fn with_material(mut self, material: impl Into<String>) -> Self {
        self.material = Some(material.into());
        self
    }

    pub fn with_sounds(mut self, sounds: BlockSounds) -> Self {
        self.sounds = sounds;
        self
    }

    pub fn with_properties(mut self, properties: Vec<Property>) -> Self {
        self.properties = properties;
        self
    }

    pub fn has(&self, property: Property) -> bool {
        self.properties.contains(&property)
    }

    /// Whether it fills its whole boxel. A full block shares its boxel with nothing.
    pub fn is_full(&self) -> bool {
        self.shape == [BlockBox::FULL]
    }

    /// Checks the shape (at least one box, each inside the boxel and not flat) and the
    /// material's name.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(material) = &self.material
            && !is_valid_name(material)
        {
            return Err(format!("block {:?} has material {material:?}, expected namespace:path", self.name));
        }
        if self.shape.is_empty() {
            return Err(format!("block {:?} has no shape boxes", self.name));
        }
        match self.shape.iter().find(|b| !b.is_valid()) {
            Some(b) => Err(format!("block {:?} has a box outside its boxel or with no volume: {b:?}", self.name)),
            None => Ok(()),
        }
    }
}

/// A box inside a boxel, in sixteenths of a boxel: from 0 to 16 on each axis, `min` below `max`.
/// Faces of an unturned box: +Z is the top, -Z the bottom and the rest are sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockBox {
    pub min: [u8; 3],
    pub max: [u8; 3],
}

impl BlockBox {
    /// Sixteenths across a boxel.
    pub const SIZE: u8 = 16;
    /// The whole boxel.
    pub const FULL: BlockBox = BlockBox { min: [0; 3], max: [16; 3] };

    pub const fn new(min: [u8; 3], max: [u8; 3]) -> Self {
        Self { min, max }
    }

    /// Inside the boxel, with some size on every axis.
    pub fn is_valid(&self) -> bool {
        (0..3).all(|i| self.min[i] < self.max[i] && self.max[i] <= Self::SIZE)
    }
}

/// Which turns a block takes when placed. The placement rule is the same for every block (its
/// bottom against the face aimed at, its front the way the player looks); this only says how
/// much of that turn the block keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Turning {
    /// Never turned.
    #[default]
    None,
    /// Only which way its Z axis points, like a log.
    Axis,
    /// The whole turn, like stairs or a slab.
    Any,
}

/// How a block looks inside and out, registered by name like everything else: a cube of
/// [`LookDef::SIZE`] texels a side that spans one boxel and repeats every block. Faces show the
/// texels just inside them, so a cut shows what's inside. Plain pixels, so it crosses any
/// boundary as data.
#[derive(Clone, Debug, PartialEq)]
pub struct LookDef {
    /// Namespaced name, such as `"base:stone"`.
    pub name: String,
    /// Whether it turns with the blocks drawn from it, like a log's rings following its axis.
    /// Otherwise it stays lined up with the world, so bricks run level however a slab or stair
    /// is turned. A rule of the material, so every shape of it agrees.
    pub turns: bool,
    /// Four bytes a texel, sRGB, opaque. X runs fastest, then Y, then Z (up, as in the world):
    /// texel `(x, y, z)` starts at byte `4 * LookDef::index(x, y, z)`.
    pub rgba: Vec<u8>,
    /// A shine where the sun reflects off it toward the eye, like ice; `None` for matte looks.
    pub sheen: Option<Sheen>,
}

/// How a look shines (Blinn-Phong): how bright the highlight gets, and how tight it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheen {
    /// The highlight's brightness at its center, as a share of the sun's light.
    pub strength: f32,
    /// How tight the highlight is: 1 spreads it over the whole face, hundreds make a glint.
    pub sharpness: f32,
}

impl LookDef {
    /// Texels along each side.
    pub const SIZE: usize = 16;
    /// Texels in the whole cube.
    pub const VOLUME: usize = Self::SIZE * Self::SIZE * Self::SIZE;

    /// The texel number of `(x, y, z)`, each below `SIZE`.
    pub const fn index(x: usize, y: usize, z: usize) -> usize {
        x + Self::SIZE * (y + Self::SIZE * z)
    }

    /// The color at `(x, y, z)`.
    pub fn texel(&self, x: usize, y: usize, z: usize) -> [u8; 4] {
        let at = 4 * Self::index(x, y, z);
        self.rgba[at..at + 4].try_into().unwrap()
    }

    /// The average color, for wherever the block is drawn as one color.
    pub fn average(&self) -> [u8; 3] {
        let mut sum = [0u64; 3];
        for px in self.rgba.as_chunks::<4>().0 {
            for (s, &c) in sum.iter_mut().zip(px) {
                *s += c as u64;
            }
        }
        let count = (self.rgba.len() / 4).max(1) as u64;
        sum.map(|s| (s / count) as u8)
    }
}

/// Whether `name` has the `namespace:path` form, each part non-empty and made of
/// lowercase ASCII letters, digits and underscores.
pub fn is_valid_name(name: &str) -> bool {
    let valid_part = |s: &str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    match name.split_once(':') {
        Some((namespace, path)) => valid_part(namespace) && valid_part(path),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_need_a_namespace() {
        assert!(is_valid_name("base:stone"));
        assert!(is_valid_name("my_mod:stone_2"));
        assert!(!is_valid_name("stone"));
        assert!(!is_valid_name(":stone"));
        assert!(!is_valid_name("base:"));
        assert!(!is_valid_name("base:Stone"));
        assert!(!is_valid_name("base:stone:extra"));
    }
}
