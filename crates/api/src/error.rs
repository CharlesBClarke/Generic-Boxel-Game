//! Errors mods can get back from the engine.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    /// A name isn't in `namespace:path` form.
    InvalidName(String),
    /// Something with this name is already registered.
    DuplicateName(String),
    /// No block is registered under this name.
    UnknownBlock(String),
    /// No texture is registered under this name.
    UnknownTexture(String),
    /// No movement controller is registered under this name.
    UnknownController(String),
    /// A player definition is already registered.
    DuplicatePlayerDef,
    /// An interaction definition is already registered.
    DuplicateInteractionDef,
    /// A push-out definition is already registered.
    DuplicatePushOutDef,
    /// An item drop definition is already registered.
    DuplicateItemDropDef,
    /// A definition has values that make no sense.
    InvalidDef(String),
    /// A block can't share the boxel with what's already there.
    CannotShare(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::InvalidName(name) => write!(f, "invalid name {name:?}, expected namespace:path"),
            ApiError::DuplicateName(name) => write!(f, "{name:?} is already registered"),
            ApiError::UnknownBlock(name) => write!(f, "no block named {name:?}"),
            ApiError::UnknownTexture(name) => write!(f, "no texture named {name:?}"),
            ApiError::UnknownController(name) => write!(f, "no movement controller named {name:?}"),
            ApiError::DuplicatePlayerDef => write!(f, "a player definition is already registered"),
            ApiError::DuplicateInteractionDef => write!(f, "an interaction definition is already registered"),
            ApiError::DuplicatePushOutDef => write!(f, "a push-out definition is already registered"),
            ApiError::DuplicateItemDropDef => write!(f, "an item drop definition is already registered"),
            ApiError::InvalidDef(why) => write!(f, "invalid definition: {why}"),
            ApiError::CannotShare(why) => write!(f, "can't add the block: {why}"),
        }
    }
}

impl std::error::Error for ApiError {}
