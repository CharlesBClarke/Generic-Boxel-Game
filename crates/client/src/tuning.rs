//! The live tuning panel: sliders for every value the player's movement controller, the
//! interaction, push-out and item drop definitions, tools, targets, the camera feel and the combat
//! feel expose, whichever mod they come from.

use api::{Tunable, Tunables};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::Sim;
use crate::combat::CombatFeel;
use crate::drill::{Drill, DrillTuning};
use crate::feel::CameraFeel;

#[derive(Resource, Default)]
pub struct TuningPanel {
    pub open: bool,
    /// The controller's values when the game started, for reset.
    movement_defaults: Option<Vec<Tunable>>,
    /// The interaction definition when the game started, for reset.
    interaction_defaults: Option<Vec<Tunable>>,
    /// The push-out definition when the game started, for reset.
    push_out_defaults: Option<Vec<Tunable>>,
    /// The item drop definition when the game started, for reset.
    item_drop_defaults: Option<Vec<Tunable>>,
    /// Each tool's and each target's values when the game started, for reset.
    tool_defaults: Option<Vec<Vec<Tunable>>>,
    target_defaults: Option<Vec<Vec<Tunable>>>,
    /// The last printout, shown in the panel so it can be copied.
    printout: String,
}

pub fn tuning_panel(
    mut contexts: EguiContexts,
    mut panel: ResMut<TuningPanel>,
    mut sim: ResMut<Sim>,
    mut feel: ResMut<CameraFeel>,
    mut combat: ResMut<CombatFeel>,
    mut drill: ResMut<Drill>,
) -> Result {
    // A copy, since the controller below borrows the simulation; written back at the end.
    let mut interaction = sim.0.interaction_def().cloned();
    if panel.interaction_defaults.is_none() {
        panel.interaction_defaults = interaction.as_ref().map(|i| i.tunables());
    }
    let mut push_out = sim.0.push_out_def().cloned();
    if panel.push_out_defaults.is_none() {
        panel.push_out_defaults = push_out.as_ref().map(|p| p.tunables());
    }
    let mut item_drops = sim.0.item_drop_def().cloned();
    if panel.item_drop_defaults.is_none() {
        panel.item_drop_defaults = item_drops.as_ref().map(|d| d.tunables());
    }
    let mut tools = sim.0.tools().to_vec();
    if panel.tool_defaults.is_none() {
        panel.tool_defaults = Some(tools.iter().map(|t| t.tunables()).collect());
    }
    let mut targets = sim.0.target_defs().to_vec();
    if panel.target_defaults.is_none() {
        panel.target_defaults = Some(targets.iter().map(|t| t.tunables()).collect());
    }
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
            if let Some(interaction) = &mut interaction {
                ui.separator();
                ui.heading("Interaction");
                sliders(ui, "interaction", interaction);
            }
            if let Some(push_out) = &mut push_out {
                ui.separator();
                ui.heading("Push-out");
                sliders(ui, "push_out", push_out);
            }
            if let Some(item_drops) = &mut item_drops {
                ui.separator();
                ui.heading("Item drops");
                sliders(ui, "item_drops", item_drops);
            }
            for tool in &mut tools {
                ui.separator();
                ui.heading(format!("Tool: {}", tool.name));
                let id = format!("tool {}", tool.name);
                sliders(ui, &id, tool);
            }
            for target in &mut targets {
                ui.separator();
                ui.heading(format!("Target: {}", target.name));
                let id = format!("target {}", target.name);
                sliders(ui, &id, target);
            }
            ui.separator();
            ui.heading("Combat feel");
            let mut juice = combat.clone();
            sliders(ui, "combat", &mut juice);
            if juice != *combat {
                *combat = juice;
            }
            ui.separator();
            ui.heading("Camera");
            // Read through a copy so the resource is only marked changed when a value moves.
            let mut camera = feel.clone();
            sliders(ui, "camera", &mut camera);
            if camera != *feel {
                *feel = camera;
            }
            ui.separator();
            ui.heading("Chase drill");
            sliders(ui, "drill", &mut drill.tuning);
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Reset all").clicked() {
                for t in panel.movement_defaults.iter().flatten() {
                    controller.set_tunable(&t.name, t.value);
                }
                if let Some(interaction) = &mut interaction {
                    for t in panel.interaction_defaults.iter().flatten() {
                        interaction.set_tunable(&t.name, t.value);
                    }
                }
                if let Some(push_out) = &mut push_out {
                    for t in panel.push_out_defaults.iter().flatten() {
                        push_out.set_tunable(&t.name, t.value);
                    }
                }
                if let Some(item_drops) = &mut item_drops {
                    for t in panel.item_drop_defaults.iter().flatten() {
                        item_drops.set_tunable(&t.name, t.value);
                    }
                }
                for (tool, defaults) in tools.iter_mut().zip(panel.tool_defaults.iter().flatten()) {
                    for t in defaults {
                        tool.set_tunable(&t.name, t.value);
                    }
                }
                for (target, defaults) in targets.iter_mut().zip(panel.target_defaults.iter().flatten()) {
                    for t in defaults {
                        target.set_tunable(&t.name, t.value);
                    }
                }
                *feel = CameraFeel::default();
                *combat = CombatFeel::default();
                drill.tuning = DrillTuning::default();
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
        if let Some(interaction) = &interaction {
            out += "// interaction\n";
            out += &printout(&interaction.tunables());
        }
        if let Some(push_out) = &push_out {
            out += "// push-out\n";
            out += &printout(&push_out.tunables());
        }
        if let Some(item_drops) = &item_drops {
            out += "// item drops\n";
            out += &printout(&item_drops.tunables());
        }
        for tool in &tools {
            out += &format!("// tool {}\n", tool.name);
            out += &printout(&tool.tunables());
        }
        for target in &targets {
            out += &format!("// target {}\n", target.name);
            out += &printout(&target.tunables());
        }
        out += "// combat feel\n";
        out += &printout(&combat.tunables());
        out += "// camera\n";
        out += &printout(&feel.tunables());
        out += "// chase drill\n";
        out += &printout(&drill.tuning.tunables());
        info!("tuned values:\n{out}");
        panel.printout = out;
    }
    if let (Some(edited), Some(def)) = (interaction, sim.0.interaction_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if let (Some(edited), Some(def)) = (push_out, sim.0.push_out_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if let (Some(edited), Some(def)) = (item_drops, sim.0.item_drop_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if tools != sim.0.tools() {
        sim.0.tools_mut().clone_from_slice(&tools);
    }
    if targets != sim.0.target_defs() {
        sim.0.target_defs_mut().clone_from_slice(&targets);
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
