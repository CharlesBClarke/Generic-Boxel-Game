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
    /// No look is registered under this name.
    UnknownLook(String),
    /// No movement controller is registered under this name.
    UnknownController(String),
    /// No model is registered under this name.
    UnknownModel(String),
    /// No sound is registered under this name.
    UnknownSound(String),
    /// No target is registered under this name.
    UnknownTarget(String),
    /// No block names this material.
    UnknownMaterial(String),
    /// A player definition is already registered.
    DuplicatePlayerDef,
    /// An interaction definition is already registered.
    DuplicateInteractionDef,
    /// A push-out definition is already registered.
    DuplicatePushOutDef,
    /// An item drop definition is already registered.
    DuplicateItemDropDef,
    /// An inventory definition is already registered.
    DuplicateInventoryDef,
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
            ApiError::UnknownLook(name) => write!(f, "no look named {name:?}"),
            ApiError::UnknownController(name) => write!(f, "no movement controller named {name:?}"),
            ApiError::UnknownModel(name) => write!(f, "no model named {name:?}"),
            ApiError::UnknownSound(name) => write!(f, "no sound named {name:?}"),
            ApiError::UnknownTarget(name) => write!(f, "no target named {name:?}"),
            ApiError::UnknownMaterial(name) => write!(f, "no block is made of a material named {name:?}"),
            ApiError::DuplicatePlayerDef => write!(f, "a player definition is already registered"),
            ApiError::DuplicateInteractionDef => write!(f, "an interaction definition is already registered"),
            ApiError::DuplicatePushOutDef => write!(f, "a push-out definition is already registered"),
            ApiError::DuplicateItemDropDef => write!(f, "an item drop definition is already registered"),
            ApiError::DuplicateInventoryDef => write!(f, "an inventory definition is already registered"),
            ApiError::InvalidDef(why) => write!(f, "invalid definition: {why}"),
            ApiError::CannotShare(why) => write!(f, "can't add the block: {why}"),
        }
    }
}

impl std::error::Error for ApiError {}
