//! The live tuning panel: sliders for every value the player's movement controller, the
//! interaction, push-out, crowd, health, item drop and inventory definitions, tools, targets, brains, the camera
//! feel and the combat feel and the animation expose, whichever mod they come from.

use api::{Tunable, Tunables};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::Sim;
use crate::animation::{AnimationPreview, AnimationTuning};
use crate::combat::CombatFeel;
use crate::drill::{Drill, DrillTuning};
use crate::feel::CameraFeel;
use crate::mesher::AoTuning;

#[derive(Resource, Default)]
pub struct TuningPanel {
    pub open: bool,
    /// The controller's values when the game started, for reset.
    movement_defaults: Option<Vec<Tunable>>,
    /// The interaction definition when the game started, for reset.
    interaction_defaults: Option<Vec<Tunable>>,
    /// The push-out definition when the game started, for reset.
    push_out_defaults: Option<Vec<Tunable>>,
    /// The crowd definition when the game started, for reset.
    crowd_defaults: Option<Vec<Tunable>>,
    /// The health definition when the game started, for reset.
    health_defaults: Option<Vec<Tunable>>,
    /// The item drop definition when the game started, for reset.
    item_drop_defaults: Option<Vec<Tunable>>,
    /// The inventory definition when the game started, for reset.
    inventory_defaults: Option<Vec<Tunable>>,
    /// Each tool's and each target's values when the game started, for reset.
    tool_defaults: Option<Vec<Vec<Tunable>>>,
    target_defaults: Option<Vec<Vec<Tunable>>>,
    /// Each brain's values when the game started, for reset.
    brain_defaults: Option<Vec<Vec<Tunable>>>,
    /// The last printout, shown in the panel so it can be copied.
    printout: String,
}

#[allow(clippy::too_many_arguments)]
pub fn tuning_panel(
    mut contexts: EguiContexts,
    mut panel: ResMut<TuningPanel>,
    mut sim: ResMut<Sim>,
    mut feel: ResMut<CameraFeel>,
    mut ao: ResMut<AoTuning>,
    mut combat: ResMut<CombatFeel>,
    mut drill: ResMut<Drill>,
    mut animation: ResMut<AnimationTuning>,
    mut preview: ResMut<AnimationPreview>,
) -> Result {
    // Every clip any model has, for the preview.
    let mut clips: Vec<String> = Vec::new();
    for skin in sim.0.models().iter().filter_map(|m| m.skin.as_ref()) {
        for clip in &skin.clips {
            if !clips.contains(&clip.name) {
                clips.push(clip.name.clone());
            }
        }
    }
    // A copy, since the controller below borrows the simulation; written back at the end.
    let mut interaction = sim.0.interaction_def().cloned();
    if panel.interaction_defaults.is_none() {
        panel.interaction_defaults = interaction.as_ref().map(|i| i.tunables());
    }
    let mut push_out = sim.0.push_out_def().cloned();
    if panel.push_out_defaults.is_none() {
        panel.push_out_defaults = push_out.as_ref().map(|p| p.tunables());
    }
    let mut crowd = sim.0.crowd_def().cloned();
    if panel.crowd_defaults.is_none() {
        panel.crowd_defaults = crowd.as_ref().map(|c| c.tunables());
    }
    let mut health = sim.0.health_def().cloned();
    if panel.health_defaults.is_none() {
        panel.health_defaults = health.as_ref().map(|h| h.tunables());
    }
    let mut unkillable = sim.0.unkillable();
    let mut refill = false;
    let mut item_drops = sim.0.item_drop_def().cloned();
    if panel.item_drop_defaults.is_none() {
        panel.item_drop_defaults = item_drops.as_ref().map(|d| d.tunables());
    }
    let mut inventory = sim.0.inventory_def().cloned();
    if panel.inventory_defaults.is_none() {
        panel.inventory_defaults = inventory.as_ref().map(|d| d.tunables());
    }
    let mut tools = sim.0.tools().to_vec();
    if panel.tool_defaults.is_none() {
        panel.tool_defaults = Some(tools.iter().map(|t| t.tunables()).collect());
    }
    let mut targets = sim.0.target_defs().to_vec();
    if panel.target_defaults.is_none() {
        panel.target_defaults = Some(targets.iter().map(|t| t.tunables()).collect());
    }
    // Copies of each brain's values too, written back at the end.
    let brain_names: Vec<String> = sim.0.brain_names().map(String::from).collect();
    let mut brains: Vec<Values> =
        (0..brain_names.len()).map(|i| Values(sim.0.brain_mut(i).map_or_else(Vec::new, |b| b.tunables()))).collect();
    let brains_before = brains.clone();
    if panel.brain_defaults.is_none() {
        panel.brain_defaults = Some(brains.iter().map(|b| b.0.clone()).collect());
    }
    let mut brains_on = sim.0.brains_on();
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
            if let Some(crowd) = &mut crowd {
                ui.separator();
                ui.heading("Crowd");
                sliders(ui, "crowd", crowd);
            }
            if let Some(health) = &mut health {
                ui.separator();
                ui.heading("Player health");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut unkillable, "Can't be knocked out");
                    refill = ui.button("Heal").clicked();
                });
                sliders(ui, "health", health);
            }
            if let Some(item_drops) = &mut item_drops {
                ui.separator();
                ui.heading("Item drops");
                sliders(ui, "item_drops", item_drops);
            }
            if let Some(inventory) = &mut inventory {
                ui.separator();
                ui.heading("Inventory");
                sliders(ui, "inventory", inventory);
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
            if !brain_names.is_empty() {
                ui.separator();
                ui.heading("Brains");
                ui.checkbox(&mut brains_on, "think (off: targets are dummies)");
            }
            for (name, brain) in brain_names.iter().zip(&mut brains) {
                ui.label(format!("Brain: {name}"));
                sliders(ui, &format!("brain {name}"), brain);
            }
            ui.separator();
            ui.heading("Combat feel");
            let mut juice = combat.clone();
            sliders(ui, "combat", &mut juice);
            if juice != *combat {
                *combat = juice;
            }
            ui.separator();
            ui.heading("Animation");
            let mut shown = preview.clip.clone();
            egui::ComboBox::from_label("every creature plays")
                .selected_text(shown.as_deref().unwrap_or("its own choice"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut shown, None, "its own choice");
                    for clip in &clips {
                        ui.selectable_value(&mut shown, Some(clip.clone()), clip);
                    }
                });
            if shown != preview.clip {
                preview.clip = shown;
            }
            let mut playing = animation.clone();
            sliders(ui, "animation", &mut playing);
            if playing != *animation {
                *animation = playing;
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
            ui.heading("Ambient occlusion");
            // Through a copy too: a change remeshes every chunk.
            let mut shading = *ao;
            sliders(ui, "ao", &mut shading);
            if shading != *ao {
                *ao = shading;
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
                if let Some(crowd) = &mut crowd {
                    for t in panel.crowd_defaults.iter().flatten() {
                        crowd.set_tunable(&t.name, t.value);
                    }
                }
                if let Some(health) = &mut health {
                    for t in panel.health_defaults.iter().flatten() {
                        health.set_tunable(&t.name, t.value);
                    }
                }
                unkillable = false;
                if let Some(item_drops) = &mut item_drops {
                    for t in panel.item_drop_defaults.iter().flatten() {
                        item_drops.set_tunable(&t.name, t.value);
                    }
                }
                if let Some(inventory) = &mut inventory {
                    for t in panel.inventory_defaults.iter().flatten() {
                        inventory.set_tunable(&t.name, t.value);
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
                for (brain, defaults) in brains.iter_mut().zip(panel.brain_defaults.iter().flatten()) {
                    brain.0.clone_from(defaults);
                }
                brains_on = true;
                *feel = CameraFeel::default();
                ao.set_if_neq(AoTuning::default());
                *combat = CombatFeel::default();
                *animation = AnimationTuning::default();
                preview.clip = None;
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
        if let Some(crowd) = &crowd {
            out += "// crowd\n";
            out += &printout(&crowd.tunables());
        }
        if let Some(health) = &health {
            out += "// health\n";
            out += &printout(&health.tunables());
        }
        if let Some(item_drops) = &item_drops {
            out += "// item drops\n";
            out += &printout(&item_drops.tunables());
        }
        if let Some(inventory) = &inventory {
            out += "// inventory\n";
            out += &printout(&inventory.tunables());
        }
        for tool in &tools {
            out += &format!("// tool {}\n", tool.name);
            out += &printout(&tool.tunables());
        }
        for target in &targets {
            out += &format!("// target {}\n", target.name);
            out += &printout(&target.tunables());
        }
        for (name, brain) in brain_names.iter().zip(&brains) {
            out += &format!("// brain {name}\n");
            out += &printout(&brain.0);
        }
        out += "// combat feel\n";
        out += &printout(&combat.tunables());
        out += "// animation\n";
        out += &printout(&animation.tunables());
        out += "// camera\n";
        out += &printout(&feel.tunables());
        out += "// ambient occlusion\n";
        out += &printout(&ao.tunables());
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
    if let (Some(edited), Some(def)) = (crowd, sim.0.crowd_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if let (Some(edited), Some(def)) = (health, sim.0.health_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if unkillable != sim.0.unkillable() {
        sim.0.set_unkillable(unkillable);
    }
    if refill {
        sim.0.set_health(f32::MAX);
    }
    if let (Some(edited), Some(def)) = (item_drops, sim.0.item_drop_def_mut())
        && edited != *def
    {
        *def = edited;
    }
    if let (Some(edited), Some(def)) = (inventory, sim.0.inventory_def_mut())
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
    for (i, (edited, before)) in brains.iter().zip(&brains_before).enumerate() {
        if let Some(brain) = sim.0.brain_mut(i) {
            for (t, b) in edited.0.iter().zip(&before.0) {
                if t.value != b.value {
                    brain.set_tunable(&t.name, t.value);
                }
            }
        }
    }
    if brains_on != sim.0.brains_on() {
        sim.0.set_brains_on(brains_on);
    }
    Ok(())
}

/// A copy of something's values, for editing while the original is out of reach.
#[derive(Clone)]
struct Values(Vec<Tunable>);

impl Tunables for Values {
    fn tunables(&self) -> Vec<Tunable> {
        self.0.clone()
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        self.0.iter_mut().find(|t| t.name == name).map(|t| t.value = value).is_some()
    }
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
