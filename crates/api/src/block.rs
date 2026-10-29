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
    /// How a player's aim turns it when placed.
    pub placement: Placement,
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
            placement: Placement::Fixed,
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

    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// Whether it fills its whole boxel. A full block shares its boxel with nothing.
    pub fn is_full(&self) -> bool {
        self.shape == [BlockBox::FULL]
    }

    /// Checks the shape: at least one box, each inside the boxel and not flat.
    pub fn validate(&self) -> Result<(), String> {
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

/// How a placed block is turned, from where the player aims. The engine owns the rules; a
/// block picks one. Each rule only produces some orientations, which are the ones the block
/// can have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Never turned.
    #[default]
    Fixed,
    /// Its +Z axis lines up with the face aimed at, like a log.
    AlongFace,
    /// Its +Y side points the way the player looks, rounded to north, south, east or west, and
    /// it's flipped upside down when aimed at the upper half of the boxel, like stairs.
    FacingAway,
    /// Its bottom goes down, or it's flipped to the top of the boxel when aimed at the upper
    /// half, like a slab.
    Half,
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
