//! The live tuning panel: sliders for every value the player's movement controller and the
//! camera feel expose, whichever mod they come from.

use api::{Tunable, Tunables};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::Sim;
use crate::feel::CameraFeel;

#[derive(Resource, Default)]
pub struct TuningPanel {
    pub open: bool,
    /// The controller's values when the game started, for reset.
    movement_defaults: Option<Vec<Tunable>>,
    /// The last printout, shown in the panel so it can be copied.
    printout: String,
}

pub fn tuning_panel(
    mut contexts: EguiContexts,
    mut panel: ResMut<TuningPanel>,
    mut sim: ResMut<Sim>,
    mut feel: ResMut<CameraFeel>,
) -> Result {
    let Some(player) = sim.0.player_mut() else {
        return Ok(());
    };
    let controller = player.controller_mut();
    if panel.movement_defaults.is_none() {
        panel.movement_defaults = Some(controller.tunables());
    }
    if !panel.open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut print = false;
    egui::Window::new("Tuning").default_pos([10.0, 120.0]).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(600.0).show(ui, |ui| {
            ui.heading("Movement");
            sliders(ui, "movement", controller);
            ui.separator();
            ui.heading("Camera");
            // Read through a copy so the resource is only marked changed when a value moves.
            let mut camera = feel.clone();
            sliders(ui, "camera", &mut camera);
            if camera != *feel {
                *feel = camera;
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Reset all").clicked() {
                for t in panel.movement_defaults.iter().flatten() {
                    controller.set_tunable(&t.name, t.value);
                }
                *feel = CameraFeel::default();
            }
            print = ui.button("Print values").clicked();
        });
        if !panel.printout.is_empty() {
            ui.label("Printed to the log too:");
            let mut text = panel.printout.clone();
            ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(6));
        }
    });

    if print {
        let mut out = String::from("// movement\n");
        out += &printout(&controller.tunables());
        out += "// camera\n";
        out += &printout(&feel.tunables());
        info!("tuned values:\n{out}");
        panel.printout = out;
    }
    Ok(())
}

fn sliders<T: Tunables + ?Sized>(ui: &mut egui::Ui, id: &str, target: &mut T) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        for t in target.tunables() {
            let mut value = t.value;
            ui.label(&t.name);
            if ui.add(egui::Slider::new(&mut value, t.min..=t.max)).changed() {
                target.set_tunable(&t.name, value);
            }
            ui.end_row();
        }
    });
}

/// Values as `name: value,` lines, ready to paste into a struct literal.
fn printout(tunables: &[Tunable]) -> String {
    tunables.iter().map(|t| format!("{}: {:?},\n", t.name, t.value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printout_pastes_as_struct_fields() {
        let out = printout(&[Tunable::new("walk_speed", 5.0, 0.0, 10.0), Tunable::new("gravity", 25.5, 0.0, 80.0)]);
        assert_eq!(out, "walk_speed: 5.0,\ngravity: 25.5,\n");
    }
}
