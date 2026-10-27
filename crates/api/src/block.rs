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
}

impl BlockDef {
    /// A placeable block with normal friction.
    pub fn new(name: impl Into<String>, solid: bool, color: [u8; 3]) -> Self {
        Self { name: name.into(), solid, color, textures: None, friction: 1.0, placeable: true }
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
