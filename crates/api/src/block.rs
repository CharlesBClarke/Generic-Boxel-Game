//! Block definitions.

/// A kind of block, registered by name. Names are the identity; any numeric IDs the
/// engine assigns at runtime are private to it.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    /// Namespaced name, such as `"base:stone"`.
    pub name: String,
    /// Whether players and other bodies collide with it.
    pub solid: bool,
    /// Flat color, sRGB.
    pub color: [u8; 3],
}

impl BlockDef {
    pub fn new(name: impl Into<String>, solid: bool, color: [u8; 3]) -> Self {
        Self { name: name.into(), solid, color }
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
