//! The guest side of the WebAssembly boundary. A mod compiled to WebAssembly writes the plain
//! traits ([`MovementController`](crate::MovementController), [`Brain`](crate::Brain)) and
//! exports one with [`export_movement_controller!`](crate::export_movement_controller) or
//! [`export_brain!`](crate::export_brain); these modules turn the component calls into trait
//! calls and convert the records both ways.
//!
//! Neither keeps state between calls: the engine keeps each body's and creature's memory and
//! passes it in, so a component holds only its settings and can be restarted at any time.

pub mod brain;
pub mod movement;
