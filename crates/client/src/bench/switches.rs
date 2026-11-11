//! The bench's switches: settings that are checkboxes in the tuning panel rather than values in
//! the tuning file, by `section.name`. The one part of the bench that changes with the commit:
//! `tools/bench/backport.sh` gives older commits a stub that knows none.

use bevy::prelude::*;

use crate::shading::ShadingTuning;
use crate::shadows::ShadowTuning;
use crate::view::ViewSwitches;

/// Sets the switch named `key` to `on`; false if there is no such switch.
pub fn set(world: &mut World, key: &str, on: bool) -> bool {
    let mut shading = *world.resource::<ShadingTuning>();
    let mut shadows = *world.resource::<ShadowTuning>();
    let mut view = *world.resource::<ViewSwitches>();
    let switch = match key {
        "shading.light" => &mut shading.parts.light,
        "shading.edges" => &mut shading.parts.edges,
        "shading.sheen" => &mut shading.parts.sheen,
        "shading.fog" => &mut shading.parts.fog,
        "shading.sky" => &mut shading.parts.sky,
        "shading.stored" => &mut shading.parts.stored,
        "shadows.maps" => &mut shadows.switches.maps,
        "shadows.far" => &mut shadows.switches.far,
        "shadows.sun_moves" => &mut shadows.switches.sun_moves,
        "shadows.show_cascades" => &mut shadows.switches.show_cascades,
        "view.msaa" => &mut view.msaa,
        "view.depth_prepass" => &mut view.depth_prepass,
        _ => return false,
    };
    *switch = on;
    world.resource_mut::<ShadingTuning>().set_if_neq(shading);
    world.resource_mut::<ShadowTuning>().set_if_neq(shadows);
    world.resource_mut::<ViewSwitches>().set_if_neq(view);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_are_set_by_name() {
        let mut world = World::new();
        world.insert_resource(ShadingTuning::default());
        world.insert_resource(ShadowTuning::default());
        world.insert_resource(ViewSwitches::default());
        assert!(set(&mut world, "shadows.maps", false));
        assert!(set(&mut world, "shading.fog", false));
        assert!(!set(&mut world, "shadows.map_size", false), "a tuning value, not a switch");
        assert!(!world.resource::<ShadowTuning>().switches.maps);
        assert!(world.resource::<ShadowTuning>().switches.far);
        assert!(!world.resource::<ShadingTuning>().parts.fog);
        assert!(set(&mut world, "view.msaa", false) && set(&mut world, "view.depth_prepass", true));
        assert_eq!(*world.resource::<ViewSwitches>(), ViewSwitches { msaa: false, depth_prepass: true });
    }
}
