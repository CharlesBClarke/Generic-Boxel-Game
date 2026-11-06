//! Developer text on screen: readouts, coordinates, the controls list. Shown by default in
//! debug builds and hidden in release builds; the `dev_text` key flips it in either.

use bevy::prelude::*;

use crate::controls::Controls;

#[derive(Resource, Debug, PartialEq)]
pub struct DevMode {
    /// Whether dev text and dev outlines are drawn.
    pub show_text: bool,
    /// Whether creatures' paths are drawn.
    pub show_routes: bool,
}

impl Default for DevMode {
    fn default() -> Self {
        Self { show_text: cfg!(debug_assertions), show_routes: false }
    }
}

/// Marks a text entity as dev text, hidden while `DevMode::show_text` is off.
#[derive(Component)]
pub struct DevText;

pub(crate) fn dev_keys(keys: Res<ButtonInput<KeyCode>>, controls: Res<Controls>, mut dev: ResMut<DevMode>) {
    if keys.just_pressed(controls.dev_text) {
        dev.show_text = !dev.show_text;
    }
    if keys.just_pressed(controls.show_routes) {
        dev.show_routes = !dev.show_routes;
    }
}

pub(crate) fn show_dev_text(dev: Res<DevMode>, mut texts: Query<&mut Visibility, With<DevText>>) {
    let shown = if dev.show_text { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut texts {
        visibility.set_if_neq(shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_text_follows_the_build() {
        assert_eq!(DevMode::default().show_text, cfg!(debug_assertions));
    }

    #[test]
    fn the_key_flips_dev_text_and_hides_marked_entities() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Controls::default())
            .insert_resource(DevMode { show_text: true, show_routes: false })
            .add_systems(Update, (dev_keys, show_dev_text).chain());
        let dev = app.world_mut().spawn((Visibility::Inherited, DevText)).id();
        let game = app.world_mut().spawn(Visibility::Inherited).id();

        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F1);
        app.update();
        assert!(!app.world().resource::<DevMode>().show_text);
        assert_eq!(app.world().get::<Visibility>(dev), Some(&Visibility::Hidden));
        assert_eq!(app.world().get::<Visibility>(game), Some(&Visibility::Inherited), "only dev text hides");

        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(KeyCode::F1);
        keys.clear();
        keys.press(KeyCode::F1);
        app.update();
        assert!(app.world().resource::<DevMode>().show_text);
        assert_eq!(app.world().get::<Visibility>(dev), Some(&Visibility::Inherited));
    }
}
