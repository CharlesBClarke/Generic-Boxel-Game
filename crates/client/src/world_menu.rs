//! The world menu: Escape opens it while playing, and Escape again goes back. It saves the world,
//! puts it back as last saved, saves it under another name, opens another saved world or starts
//! a new one, and quits. Opening another world saves this one first, as quitting does.

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use protocol::Command;

use crate::world::{ClientWorld, Link};

#[derive(Resource, Default)]
pub(crate) struct WorldMenu {
    pub(crate) open: bool,
    /// The server has been asked for the saved worlds since the menu opened.
    listed: bool,
    save_as: String,
    new_world: String,
}

impl WorldMenu {
    pub(crate) fn show(&mut self) {
        self.open = true;
        self.listed = false;
    }
}

/// A name the server takes for a world: letters, digits, `-` and `_`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What a click in the menu asks for.
enum Choice {
    Send(Command),
    /// Sends the command, then goes back to playing.
    SendAndPlay(Command),
    Play,
    Quit,
}

/// Draws the menu while it's open, and does what's clicked.
pub(crate) fn world_menu(
    mut contexts: EguiContexts,
    mut menu: ResMut<WorldMenu>,
    mut world: ResMut<ClientWorld>,
    mut link: ResMut<Link>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    if !menu.open {
        return Ok(());
    }
    if !menu.listed {
        menu.listed = true;
        link.command(Command::ListWorlds, &mut world);
    }
    let current = world.world_name().map(str::to_owned);
    let saved = world.saved_worlds().to_vec();
    let mut choice = None;
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("World")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(320.0);
            let Some(current) = &current else {
                ui.label("This game doesn't save worlds.");
                return;
            };
            ui.heading(format!("World: {current}"));
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(Choice::Send(Command::SaveWorld));
                }
                let can_reload = saved.contains(current);
                if ui.add_enabled(can_reload, egui::Button::new("Reload last save")).clicked() {
                    choice = Some(Choice::SendAndPlay(Command::LoadWorld));
                }
            });

            ui.separator();
            ui.label("Save as another world, and play on in that one:");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut menu.save_as);
                if ui.add_enabled(valid_name(&menu.save_as), egui::Button::new("Save as")).clicked() {
                    choice = Some(Choice::Send(Command::SaveWorldAs(std::mem::take(&mut menu.save_as))));
                }
            });

            ui.separator();
            ui.label("Saved worlds (opening one saves this one first):");
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                if saved.is_empty() {
                    ui.weak("none yet");
                }
                for name in &saved {
                    ui.horizontal(|ui| {
                        let here = name == current;
                        if ui.add_enabled(!here, egui::Button::new("Open")).clicked() {
                            choice = Some(Choice::SendAndPlay(Command::OpenWorld(name.clone())));
                        }
                        match here {
                            true => ui.strong(format!("{name} (playing)")),
                            false => ui.label(name),
                        };
                    });
                }
            });

            ui.separator();
            ui.label("New world, as the game starts (saves this one first):");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut menu.new_world);
                let name = menu.new_world.clone();
                let ok = valid_name(&name) && !saved.contains(&name) && name != *current;
                if ui.add_enabled(ok, egui::Button::new("Start")).clicked() {
                    menu.new_world.clear();
                    choice = Some(Choice::SendAndPlay(Command::OpenWorld(name)));
                }
            });
            if !menu.new_world.is_empty() && !valid_name(&menu.new_world) {
                ui.weak("names use letters, digits, - and _");
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Back to playing").clicked() {
                    choice = Some(Choice::Play);
                }
                if ui.button("Save and quit").clicked() {
                    choice = Some(Choice::Quit);
                }
            });
            ui.weak("Escape goes back to playing.");
        });

    match choice {
        None => {}
        Some(Choice::Send(command)) => {
            link.command(command, &mut world);
            link.command(Command::ListWorlds, &mut world);
        }
        Some(Choice::SendAndPlay(command)) => {
            link.command(command, &mut world);
            menu.open = false;
            crate::set_grab(&mut cursor, true);
        }
        Some(Choice::Play) => {
            menu.open = false;
            crate::set_grab(&mut cursor, true);
        }
        // Quitting saves the world (see `crate::save_on_exit`).
        Some(Choice::Quit) => {
            exit.write(AppExit::Success);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_what_the_server_takes() {
        for good in ["default", "door-test", "a_2"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "..", "a/b", "with space", "é"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
