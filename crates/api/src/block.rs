//! Block definitions.

/// A kind of block, registered by name. Names are the identity; any numeric IDs the
/// engine assigns at runtime are private to it.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    /// Namespaced name, such as `"base:stone"`.
    pub name: String,
    /// Whether players and other bodies collide with it.
    pub solid: bool,
    /// Flat color, sRGB. Used where the block has no textures.
    pub color: [u8; 3],
    /// Textures by face, if any. Each names a registered texture.
    pub textures: Option<FaceTextures>,
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
    /// Whether its textures turn with it, like a log's bark following its axis. Otherwise they
    /// stay lined up with the world, so bricks run level however a slab or stair is turned.
    pub turn_textures: bool,
    /// What breaking it gives: its volume of this material, as a loose item. Namespaced, like
    /// `"base:stone"`; blocks of different shapes share one, so a slab gives half a stone.
    /// `None` gives nothing.
    pub material: Option<String>,
}

impl BlockDef {
    /// A placeable block with normal friction.
    pub fn new(name: impl Into<String>, solid: bool, color: [u8; 3]) -> Self {
        Self {
            name: name.into(),
            solid,
            color,
            textures: None,
            friction: 1.0,
            placeable: true,
            shape: vec![BlockBox::FULL],
            turning: Turning::None,
            turn_textures: false,
            material: None,
        }
    }

    pub fn with_textures(mut self, textures: FaceTextures) -> Self {
        self.textures = Some(textures);
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

    pub fn with_turn_textures(mut self, turn_textures: bool) -> Self {
        self.turn_textures = turn_textures;
        self
    }

    pub fn with_material(mut self, material: impl Into<String>) -> Self {
        self.material = Some(material.into());
        self
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

/// Which texture goes on which faces of a block, by texture name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceTextures {
    pub top: String,
    pub bottom: String,
    /// The four sides, drawn upright.
    pub side: String,
}

impl FaceTextures {
    /// The same texture on every face.
    pub fn all(name: impl Into<String>) -> Self {
        let name = name.into();
        Self { top: name.clone(), bottom: name.clone(), side: name }
    }

    /// Every texture named, top, bottom then side.
    pub fn names(&self) -> [&str; 3] {
        [&self.top, &self.bottom, &self.side]
    }
}

/// An image blocks can name, registered by name like everything else. Plain pixels, so it
/// crosses any boundary as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureDef {
    /// Namespaced name, such as `"base:stone"`.
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Row by row from the top left, four bytes a pixel, sRGB with straight alpha.
    pub rgba: Vec<u8>,
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
