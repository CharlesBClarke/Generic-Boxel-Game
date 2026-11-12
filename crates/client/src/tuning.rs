//! The live tuning panel: sliders for every value the server lets a client tune (the player's
//! movement controller, the interaction, push-out, crowd, health, item drop and inventory
//! definitions, tools, targets and brains, whichever mod they come from) and the client's own:
//! sound, the camera feel, the combat feel, the animation, ambient occlusion, shading and
//! shadows, the view, the UI scale and the chase drill. Terrain values take effect when the
//! terrain is generated again. Save keeps them in the tuning file (see `tuning_file`), which is
//! applied at startup.
//!
//! Sections sit on tabs in a sidebar that folds away (see `tuning_layout`), their values in
//! the groups each section gives them; a search box lists every match instead. Hovering a
//! slider's name shows its help; right-click resets it or stars it as a favourite.
//!
//! The server's values are edited as copies and each change is sent back as a command; the
//! server answers with its definitions as changed.

use api::{Tunable, Tunables};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use protocol::{Command, TuningSection};

use crate::animation::{AnimationPreview, AnimationTuning};
use crate::combat::CombatFeel;
use crate::drill::{Drill, DrillTuning};
use crate::feel::CameraFeel;
use crate::mesher::AoTuning;
use crate::sound::{Kind, Pick, Picks, SoundPicks, SoundPreview, SoundTuning};
use crate::tuning_file;
use crate::tuning_layout::{self, Favourites, Tab};
use crate::ui_scale::UiScaleTuning;
use crate::shading::ShadingTuning;
use crate::shadows::{Filter, ShadowTuning};
use crate::view::{ToneMapper, ViewSwitches, ViewTuning};
use crate::world::{ClientWorld, Link};

#[derive(Resource, Default)]
pub struct TuningPanel {
    pub open: bool,
    /// The server's values when the game started, for reset.
    server_defaults: Option<Vec<TuningSection>>,
    /// The last printout, shown in the panel so it can be copied.
    printout: String,
    /// Whether the saved tuning file has been applied yet.
    loaded: bool,
    /// Where the last save went, or why it failed.
    note: String,
    /// Ambient occlusion values moved but not applied yet: applying remeshes every chunk,
    /// too slow to do on every slider step.
    ao_draft: Option<AoTuning>,
    /// The sidebar's page.
    tab: Tab,
    /// Whether the sidebar's list of tabs is folded away.
    hide_tabs: bool,
    /// What's typed in the search box; while it has words, the page is every match.
    search: String,
    /// Fold or unfold every header this frame (the Fold all and Unfold all buttons).
    fold: Option<bool>,
    /// The starred sliders, loaded from their file when the panel first opens.
    favourites: Option<Favourites>,
    /// Every section's values as the game started, by saved name, for the changed marks and
    /// resetting one value.
    defaults: Vec<(String, Vec<Tunable>)>,
}

#[allow(clippy::too_many_arguments)]
pub fn tuning_panel(
    mut contexts: EguiContexts,
    mut panel: ResMut<TuningPanel>,
    mut world: ResMut<ClientWorld>,
    mut link: ResMut<Link>,
    mut feel: ResMut<CameraFeel>,
    mut ao: ResMut<AoTuning>,
    mut combat: ResMut<CombatFeel>,
    mut drill: ResMut<Drill>,
    mut animation: ResMut<AnimationTuning>,
    mut preview: ResMut<AnimationPreview>,
    mut sound: ResMut<SoundTuning>,
    mut preview_sound: ResMut<SoundPreview>,
    mut picks: ResMut<SoundPicks>,
    (mut view, mut view_switches): (ResMut<ViewTuning>, ResMut<ViewSwitches>),
    (mut light, mut shadow): (ResMut<ShadingTuning>, ResMut<ShadowTuning>),
    mut ui_scale: ResMut<UiScaleTuning>,
) -> Result {
    // The saved file is applied once, on the first frame there is a player, open or not.
    if world.player().is_none() {
        return Ok(());
    }
    let load = !panel.loaded;
    if !panel.open && !load {
        return Ok(());
    }
    panel.loaded = true;
    // Every clip any model has, for the preview.
    let mut clips: Vec<String> = Vec::new();
    for skin in world.models().iter().filter_map(|m| m.skin.as_ref()) {
        for clip in &skin.clips {
            if !clips.contains(&clip.name) {
                clips.push(clip.name.clone());
            }
        }
    }
    // Copies of the server's values, compared with these at the end to send what changed.
    let before = world.defs().tuning.clone();
    if panel.server_defaults.is_none() {
        panel.server_defaults = Some(before.clone());
        let mut defaults: Vec<(String, Vec<Tunable>)> = before.iter().map(|s| (s.name.clone(), s.values.clone())).collect();
        let client = [
            ("combat", CombatFeel::default().tunables()),
            ("animation", AnimationTuning::default().tunables()),
            ("camera", CameraFeel::default().tunables()),
            ("ao", AoTuning::default().tunables()),
            ("view", ViewTuning::default().tunables()),
            ("ui", UiScaleTuning::default().tunables()),
            ("shading", ShadingTuning::default().tunables()),
            ("shadows", ShadowTuning::default().tunables()),
            ("drill", DrillTuning::default().tunables()),
            ("sound", SoundTuning::default().tunables()),
        ];
        defaults.extend(client.map(|(name, values)| (name.to_owned(), values)));
        panel.defaults = defaults;
    }
    let mut server: Vec<(String, Values)> = before.iter().map(|s| (s.name.clone(), Values(s.values.clone()))).collect();
    let mut unkillable = world.defs().unkillable;
    let mut brains_on = world.defs().brains_on;
    let clock = *world.clock();
    let mut hours = clock.hours();
    let mut paused = clock.paused;
    let mut refill = false;
    // The client's own values, through copies so a resource is only marked changed when a
    // value moves (an ambient occlusion change remeshes every chunk).
    let mut juice = combat.clone();
    let mut playing = animation.clone();
    let mut camera = feel.clone();
    let mut shading = panel.ao_draft.unwrap_or(*ao);
    let mut apply_ao = false;
    let mut seeing = *view;
    let mut drawing = *view_switches;
    let mut sizing = *ui_scale;
    let mut lighting = *light;
    let mut shadows = *shadow;
    let mut regenerate = false;
    let mut drill_tuning = drill.tuning.clone();
    let mut hearing = sound.clone();
    let mut trying = picks.clone();
    // Every sound whose name (after the namespace) starts with a prefix, for the pick lists.
    let named = |prefix: &str| -> Vec<String> {
        world.sounds().iter().map(|s| s.name.clone()).filter(|n| short(n).starts_with(prefix)).collect()
    };
    let step_sounds = named("step_");
    // Any sound can be picked anywhere: a row's own kind is listed first, then the rest. Loops
    // (the servo) are left out.
    let all_sounds: Vec<String> = world.sounds().iter().filter(|s| !s.looping).map(|s| s.name.clone()).collect();
    // Every surface with steps of its own, for the Footsteps list.
    let surfaces: Vec<(String, String)> = world
        .blocks()
        .iter()
        .filter(|(_, d)| d.is_full())
        .filter_map(|(_, d)| Some((d.name.clone(), d.sounds.step.clone()?)))
        .collect();
    // Each kind of combat sound: its own, and the sounds offered for it.
    let kinds: Vec<(Kind, String, Vec<String>)> =
        Kind::ALL.iter().map(|&k| (k, k.own(world.defs()).unwrap_or_default(), named(k.prefix()))).collect();

    // Every section of sliders by its saved name, for loading, saving and printing.
    macro_rules! sections {
        () => {{
            let mut s: Vec<(String, &mut dyn Tunables)> =
                server.iter_mut().map(|(name, values)| (name.clone(), values as &mut dyn Tunables)).collect();
            s.push(("combat".to_owned(), &mut juice));
            s.push(("animation".to_owned(), &mut playing));
            s.push(("camera".to_owned(), &mut camera));
            s.push(("ao".to_owned(), &mut shading));
            s.push(("view".to_owned(), &mut seeing));
            s.push(("ui".to_owned(), &mut sizing));
            s.push(("shading".to_owned(), &mut lighting));
            s.push(("shadows".to_owned(), &mut shadows));
            s.push(("drill".to_owned(), &mut drill_tuning));
            s.push(("sound".to_owned(), &mut hearing));
            s
        }};
    }

    if load
        && let Some(path) = tuning_file::path()
        && let Ok(text) = std::fs::read_to_string(&path)
    {
        let (saved, mut problems) = tuning_file::SavedTuning::parse(&text);
        let mut sections = sections!();
        for (name, target) in &mut sections {
            saved.apply(name, &mut **target, &mut problems);
        }
        let known: Vec<String> = sections.into_iter().map(|(name, _)| name).collect();
        let surface_names: Vec<String> = surfaces.iter().map(|(name, _)| name.clone()).collect();
        let lines = saved.text(tuning_file::FOOTSTEPS);
        trying.steps = Picks::from_lines(tuning_file::FOOTSTEPS, &lines, &surface_names, &all_sounds, &mut problems);
        let kind_keys: Vec<String> = Kind::ALL.iter().map(|k| k.key().to_owned()).collect();
        let every_sound: Vec<String> = world.sounds().iter().map(|s| s.name.clone()).collect();
        let lines = saved.text(tuning_file::COMBAT_SOUNDS);
        trying.combat = Picks::from_lines(tuning_file::COMBAT_SOUNDS, &lines, &kind_keys, &every_sound, &mut problems);
        let known: Vec<String> = known.into_iter().chain(tuning_file::TEXT_SECTIONS.map(str::to_owned)).collect();
        for section in saved.unknown_sections(&known) {
            problems.push(format!("no section named [{section}]"));
        }
        for problem in problems {
            warn!("{}: {problem}", path.display());
        }
        info!("loaded tuning from {}", path.display());
    }

    let mut print = false;
    let mut save = false;
    if panel.open {
        let ctx = contexts.ctx_mut()?;
        let TuningPanel { tab, hide_tabs, search, fold, favourites, defaults, server_defaults, printout, note, .. } =
            &mut *panel;
        let favourites = favourites.get_or_insert_with(Favourites::load);
        // A star clicked this frame, applied and saved after drawing.
        let mut toggled: Option<(String, String)> = None;
        // Every section's saved name, in panel order.
        let keys: Vec<String> = sections!().into_iter().map(|(name, _)| name).collect();
        let tabs: Vec<Tab> =
            Tab::ALL.into_iter().filter(|&t| t == Tab::Favourites || keys.iter().any(|k| Tab::of(k) == t)).collect();
        if !tabs.contains(tab) {
            *tab = Tab::default();
        }
        let height = ctx.content_rect().height() * 0.7;

        // Sliders for every value of `section` that `keep` keeps, under a heading per
        // section that has any: search results and favourites.
        macro_rules! flat_list {
            ($ui:expr, $keep:expr) => {{
                let ui: &mut egui::Ui = $ui;
                let mut any = false;
                for (key, target) in sections!() {
                    let hits: Vec<Tunable> =
                        target.tunables().into_iter().filter(|t| !skipped(&key, &t.name) && $keep(&key, t)).collect();
                    if hits.is_empty() {
                        continue;
                    }
                    any = true;
                    let title = format!("{} › {}", Tab::of(&key).label(), tuning_layout::heading(&key));
                    let go = ui.add(egui::Label::new(egui::RichText::new(title).strong()).sense(egui::Sense::click()));
                    if go.on_hover_text("Go to this section's tab").clicked() {
                        *tab = Tab::of(&key);
                        search.clear();
                    }
                    rows(ui, &key, "flat", hits, target, defaults, favourites, &mut toggled);
                    ui.add_space(4.0);
                }
                any
            }};
        }

        // A section's own controls that aren't sliders, above its sliders (`before`) or
        // below them.
        macro_rules! extras {
            ($ui:expr, $key:expr, before) => {{
                let ui: &mut egui::Ui = $ui;
                match $key {
                    "health" => {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut unkillable, "Can't be knocked out");
                            refill = ui.button("Heal").clicked();
                        });
                    }
                    "time" => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut hours, 0.0..=23.99).text(clock_text(clock.hours())));
                            ui.checkbox(&mut paused, "paused");
                        });
                    }
                    "view" => {
                        egui::ComboBox::from_label("tone mapper")
                            .selected_text(seeing.tone_mapper.label())
                            .show_ui(ui, |ui| {
                                for t in ToneMapper::ALL {
                                    ui.selectable_value(&mut seeing.tone_mapper, t, t.label());
                                }
                            });
                    }
                    "ui" => {
                        ui.label("menus and text grow with the window's height (natural size at reference_height), times size");
                    }
                    "shading" => {
                        ui.label("sun and sky light, the sky, fog, and the lines along creases and ridges");
                        ui.horizontal_wrapped(|ui| {
                            ui.label("draw:");
                            let parts = &mut lighting.parts;
                            ui.checkbox(&mut parts.light, "sun and sky light");
                            ui.checkbox(&mut parts.edges, "edges");
                            ui.checkbox(&mut parts.sheen, "sheen");
                            ui.checkbox(&mut parts.fog, "fog");
                            ui.checkbox(&mut parts.sky, "sky");
                            ui.checkbox(&mut parts.stored, "stored light");
                            ui.checkbox(&mut parts.ao, "ambient occlusion");
                            ui.checkbox(&mut parts.detail, "generated detail");
                            ui.checkbox(&mut parts.mips, "mipmaps");
                        });
                    }
                    "shadows" => {
                        ui.label("sun shadows: sharp maps near the camera, fading into a coarse height-map shadow farther out");
                        ui.horizontal(|ui| {
                            let switches = &mut shadows.switches;
                            ui.checkbox(&mut switches.maps, "shadow maps");
                            ui.checkbox(&mut switches.far, "far shadow");
                            ui.checkbox(&mut switches.show_cascades, "tint each map");
                        });
                        egui::ComboBox::from_label("edges")
                            .selected_text(shadows.filter.label())
                            .show_ui(ui, |ui| {
                                for f in Filter::ALL {
                                    ui.selectable_value(&mut shadows.filter, f, f.label());
                                }
                            });
                    }
                    "animation" => {
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
                    }
                    key if key.starts_with("brain ") => {
                        ui.checkbox(&mut brains_on, "think (off: every brain's targets are dummies)");
                    }
                    _ => {}
                }
            }};
            ($ui:expr, $key:expr, after) => {{
                let ui: &mut egui::Ui = $ui;
                match $key {
                    key if key == "world" || key.starts_with("terrain ") => {
                        ui.horizontal(|ui| {
                            regenerate |= ui.button("Regenerate terrain").clicked();
                            ui.label("terrain values take effect here (edits in it are lost)");
                        });
                    }
                    "view" => {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut drawing.msaa, "anti-aliasing (4× MSAA)");
                            ui.checkbox(&mut drawing.depth_prepass, "depth prepass");
                            ui.checkbox(&mut drawing.merge_faces, "merge faces");
                        });
                    }
                    "ao" => {
                        ui.horizontal(|ui| {
                            let pending = shading.meshing() != ao.meshing();
                            apply_ao = ui.add_enabled(pending, egui::Button::new("Apply")).clicked();
                            ui.label(if pending { "reach (or turning it on from 0) changed: Apply remeshes every chunk" } else { "live" });
                        });
                    }
                    "sound" => {
                        egui::CollapsingHeader::new("Footsteps").open(*fold).show(ui, |ui| {
                            ui.label("Try any step sound on any surface. Save and Print values keep them too.");
                            for (surface, own) in &surfaces {
                                pick_row(ui, surface, short(surface), own, &step_sounds, &all_sounds, &mut trying.steps, &mut preview_sound.0);
                            }
                            if ui.button("Reset footsteps").clicked() {
                                trying.steps = Picks::default();
                            }
                        });
                        egui::CollapsingHeader::new("Body sounds").open(*fold).show(ui, |ui| {
                            ui.label("A landing is the surface's footstep, louder (land_loudness); a hard one adds this. A pick-up plays the material too (pickup_material).");
                            for (kind, own, choices) in kinds.iter().filter(|(k, _, _)| Kind::BODY.contains(k)) {
                                pick_row(ui, kind.key(), kind.label(), own, choices, &all_sounds, &mut trying.combat, &mut preview_sound.0);
                            }
                        });
                        egui::CollapsingHeader::new("Combat sounds").open(*fold).show(ui, |ui| {
                            ui.label("Try any sound for each kind. A swing that hits a block plays the block's footstep, louder (block_hit_volume).");
                            for (kind, own, choices) in kinds.iter().filter(|(k, _, _)| Kind::COMBAT.contains(k)) {
                                pick_row(ui, kind.key(), kind.label(), own, choices, &all_sounds, &mut trying.combat, &mut preview_sound.0);
                            }
                            if ui.button("Reset combat and body sounds").clicked() {
                                trying.combat = Picks::default();
                            }
                        });
                        egui::CollapsingHeader::new("Hear a sound").open(*fold).show(ui, |ui| {
                            for name in world.sounds().iter().filter(|s| !s.looping).map(|s| s.name.clone()) {
                                if ui.button(format!("Play {name}")).clicked() {
                                    preview_sound.0 = Some(Pick::plain(name));
                                }
                            }
                        });
                    }
                    _ => {}
                }
            }};
        }

        // One section on its tab's page: its own controls and its sliders in their groups.
        macro_rules! section_body {
            ($ui:expr, $key:expr) => {{
                let ui: &mut egui::Ui = $ui;
                let key: &str = $key;
                extras!(ui, key, before);
                {
                    let mut all = sections!();
                    if let Some((_, target)) = all.iter_mut().find(|(name, _)| name == key) {
                        grouped(ui, key, &mut **target, defaults, favourites, &mut toggled, *fold);
                    }
                }
                extras!(ui, key, after);
            }};
        }

        egui::Window::new("Tuning").default_pos([10.0, 120.0]).default_size([640.0, 560.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!*hide_tabs, "☰").on_hover_text("Show or hide the list of tabs").clicked() {
                    *hide_tabs = !*hide_tabs;
                }
                ui.add(egui::TextEdit::singleline(search).hint_text("Search every slider and its help").desired_width(240.0));
                if !search.is_empty() && ui.small_button("✕").on_hover_text("Clear the search").clicked() {
                    search.clear();
                }
                if ui.small_button("Fold all").clicked() {
                    *fold = Some(false);
                }
                if ui.small_button("Unfold all").clicked() {
                    *fold = Some(true);
                }
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                if !*hide_tabs {
                    ui.vertical(|ui| {
                        ui.set_width(130.0);
                        for &t in &tabs {
                            if ui.selectable_label(*tab == t && search.trim().is_empty(), t.label()).clicked() {
                                *tab = t;
                                search.clear();
                            }
                        }
                    });
                    ui.separator();
                }
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                        if !search.trim().is_empty() {
                            let query = search.clone();
                            if !flat_list!(ui, |key: &str, t: &Tunable| tuning_layout::matches(&query, key, t)) {
                                ui.label("Nothing matches. Search looks at names, sections, groups and help.");
                            }
                        } else if *tab == Tab::Favourites {
                            let marked = favourites.clone();
                            if !flat_list!(ui, |key: &str, t: &Tunable| marked.has(key, &t.name)) {
                                ui.label("No favourites yet: click a slider's ☆, or right-click it.");
                            }
                        } else {
                            let here: Vec<&String> = keys.iter().filter(|k| Tab::of(k) == *tab).collect();
                            let single = here.len() == 1;
                            for key in here {
                                let title = tuning_layout::heading(key);
                                if single {
                                    ui.heading(title);
                                    section_body!(ui, key);
                                } else {
                                    egui::CollapsingHeader::new(egui::RichText::new(title).heading())
                                        .id_salt(("section", key))
                                        .default_open(true)
                                        .open(*fold)
                                        .show(ui, |ui| section_body!(ui, key));
                                }
                            }
                        }
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset all").clicked() {
                    for (name, values) in &mut server {
                        let defaults = server_defaults.iter().flatten().find(|d| d.name == *name);
                        for t in defaults.map(|d| d.values.as_slice()).unwrap_or_default() {
                            values.set_tunable(&t.name, t.value);
                        }
                    }
                    unkillable = false;
                    brains_on = true;
                    camera = CameraFeel::default();
                    shading = AoTuning::default();
                    apply_ao = true;
                    seeing = ViewTuning::default();
                    drawing = ViewSwitches::default();
                    sizing = UiScaleTuning::default();
                    lighting = ShadingTuning::default();
                    shadows = ShadowTuning::default();
                    juice = CombatFeel::default();
                    playing = AnimationTuning::default();
                    preview.clip = None;
                    drill_tuning = DrillTuning::default();
                    hearing = SoundTuning::default();
                    trying = SoundPicks::default();
                }
                save = ui.button("Save").on_hover_text("Keep these values for next time").clicked();
                print = ui.button("Print values").clicked();
            });
            if !note.is_empty() {
                ui.label(&*note);
            }
            if !printout.is_empty() {
                ui.label("Printed to the log too:");
                let mut text = printout.clone();
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(6));
            }
        });
        *fold = None;
        if let Some((section, name)) = toggled {
            favourites.toggle(&section, &name);
            if let Err(e) = favourites.save() {
                *note = format!("Couldn't save favourites: {e}");
                warn!("{note}");
            }
        }
    }

    if print || save {
        let values: Vec<(String, Vec<Tunable>)> =
            sections!().into_iter().map(|(name, target)| (name, target.tunables())).collect();
        let texts = [
            (tuning_file::FOOTSTEPS.to_owned(), trying.steps.to_lines()),
            (tuning_file::COMBAT_SOUNDS.to_owned(), trying.combat.to_lines()),
        ];
        if print {
            let mut out: String = values.iter().map(|(name, t)| format!("// {name}\n{}", printout(t))).collect();
            for (section, lines) in texts.iter().filter(|(_, lines)| !lines.is_empty()) {
                out += &format!("// {section}\n");
                out += &lines.iter().map(|(name, pick)| format!("{name} = {pick}\n")).collect::<String>();
            }
            info!("tuned values:\n{out}");
            panel.printout = out;
        }
        if save {
            panel.note = match save_file(&values, &texts) {
                Ok(path) => format!("Saved to {}", path.display()),
                Err(e) => format!("Couldn't save: {e}"),
            };
            info!("{}", panel.note);
        }
    }
    let mut sent = changes(&before, &server);
    if load && world.from_save() {
        // A loaded world keeps the terrain it was saved with; generating it again would lose
        // the edits in it.
        sent.retain(|c| !shapes_terrain(c));
    }
    // Saved terrain values only show once the terrain is made again with them.
    regenerate |= load && sent.iter().any(shapes_terrain);
    for command in sent {
        link.command(command, &mut world);
    }
    if regenerate {
        link.command(Command::RegenerateTerrain, &mut world);
    }
    if unkillable != world.defs().unkillable {
        link.command(Command::Unkillable(unkillable), &mut world);
    }
    if brains_on != world.defs().brains_on {
        link.command(Command::BrainsOn(brains_on), &mut world);
    }
    if hours != clock.hours() {
        link.command(Command::SetTime(hours), &mut world);
    }
    if paused != clock.paused {
        link.command(Command::PauseTime(paused), &mut world);
    }
    if refill {
        link.command(Command::Heal, &mut world);
    }
    if juice != *combat {
        *combat = juice;
    }
    if playing != *animation {
        *animation = playing;
    }
    if camera != *feel {
        *feel = camera;
    }
    // Saved values apply as they load; moved sliders apply at once unless they need a remesh,
    // which waits for Apply.
    if load || apply_ao || shading.meshing() == ao.meshing() {
        ao.set_if_neq(shading);
    }
    panel.ao_draft = (shading != *ao).then_some(shading);
    view.set_if_neq(seeing);
    view_switches.set_if_neq(drawing);
    ui_scale.set_if_neq(sizing);
    light.set_if_neq(lighting);
    shadow.set_if_neq(shadows);
    if trying != *picks {
        *picks = trying;
    }
    if hearing != *sound {
        *sound = hearing;
    }
    if drill_tuning != drill.tuning {
        drill.tuning = drill_tuning;
    }
    Ok(())
}

/// One row of a pick list: what `key` plays (its own sound, `own`, unless picked from
/// `choices`), with volume, pitch and a Play button.
/// The list offers `choices` first, then every other sound in `all`.
#[allow(clippy::too_many_arguments)]
fn pick_row(
    ui: &mut egui::Ui,
    key: &str,
    label: &str,
    own: &str,
    choices: &[String],
    all: &[String],
    picks: &mut Picks,
    play: &mut Option<Pick>,
) {
    let unpicked = Pick { sound: own.to_owned(), volume: 1.0, pitch: 1.0 };
    let mut pick = picks.0.get(key).cloned().unwrap_or_else(|| unpicked.clone());
    ui.horizontal(|ui| {
        ui.label(label);
        let shown = if pick.sound.is_empty() { "none" } else { short(&pick.sound) };
        egui::ComboBox::from_id_salt(key).selected_text(shown).height(400.0).show_ui(ui, |ui| {
            for s in choices {
                ui.selectable_value(&mut pick.sound, s.clone(), short(s));
            }
            ui.separator();
            for s in all.iter().filter(|s| !choices.contains(s)) {
                ui.selectable_value(&mut pick.sound, s.clone(), short(s));
            }
        });
        ui.add(egui::Slider::new(&mut pick.volume, 0.0..=2.0).text("volume"));
        ui.add(egui::Slider::new(&mut pick.pitch, 0.5..=2.0).text("pitch"));
        if ui.button("Play").clicked() && !pick.sound.is_empty() {
            *play = Some(pick.clone());
        }
    });
    if pick == unpicked || pick.sound.is_empty() {
        picks.0.remove(key);
    } else {
        picks.0.insert(key.to_owned(), pick);
    }
}

/// A name without its mod's namespace, such as `step_cloth` for `base:step_cloth`.
fn short(name: &str) -> &str {
    name.split_once(':').map_or(name, |(_, path)| path)
}

/// A command for every value in `edited` that differs from `before`.
fn changes(before: &[TuningSection], edited: &[(String, Values)]) -> Vec<Command> {
    let mut out = Vec::new();
    for (section, (name, values)) in before.iter().zip(edited) {
        debug_assert_eq!(section.name, *name);
        for (old, new) in section.values.iter().zip(&values.0) {
            if old.value != new.value {
                out.push(Command::Tune { section: name.clone(), name: new.name.clone(), value: new.value });
            }
        }
    }
    out
}

/// Whether a command changes what generating the terrain makes: a generator's value, or the
/// terrain's size. The simulation distance doesn't.
/// The time of day as a clock reads it, such as `"17:05"`.
fn clock_text(hours: f64) -> String {
    let minutes = (hours.rem_euclid(24.0) * 60.0).floor() as u32;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn shapes_terrain(command: &Command) -> bool {
    match command {
        Command::Tune { section, name, .. } => section.starts_with("terrain ") || (section == "world" && name.starts_with("terrain_")),
        _ => false,
    }
}

/// Writes the values to the tuning file, making its folder if needed.
fn save_file(values: &[(String, Vec<Tunable>)], texts: &[(String, Vec<(String, String)>)]) -> std::io::Result<std::path::PathBuf> {
    let path = tuning_file::path().ok_or_else(|| std::io::Error::other("no config folder (HOME isn't set)"))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(&path, tuning_file::format(values, texts))?;
    Ok(path)
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

/// Values with a control of their own instead of a slider.
fn skipped(section: &str, name: &str) -> bool {
    matches!((section, name), ("view", "tone_mapper") | ("shadows", "filter"))
}

/// The value `name` of `section` had as the game started.
fn default_of(defaults: &[(String, Vec<Tunable>)], section: &str, name: &str) -> Option<f32> {
    defaults.iter().find(|(s, _)| s == section)?.1.iter().find(|t| t.name == name).map(|t| t.value)
}

/// A section's sliders: those in no group first, then each group under a header of its own,
/// folded unless `fold` says otherwise this frame.
#[allow(clippy::too_many_arguments)]
fn grouped(
    ui: &mut egui::Ui,
    section: &str,
    target: &mut dyn Tunables,
    defaults: &[(String, Vec<Tunable>)],
    favourites: &Favourites,
    toggled: &mut Option<(String, String)>,
    fold: Option<bool>,
) {
    let all: Vec<Tunable> = target.tunables().into_iter().filter(|t| !skipped(section, &t.name)).collect();
    let loose: Vec<Tunable> = all.iter().filter(|t| t.group.is_empty()).cloned().collect();
    rows(ui, section, "", loose, target, defaults, favourites, toggled);
    let mut groups: Vec<&str> = Vec::new();
    for t in &all {
        if !t.group.is_empty() && !groups.contains(&t.group.as_str()) {
            groups.push(&t.group);
        }
    }
    for group in groups {
        let values: Vec<Tunable> = all.iter().filter(|t| t.group == group).cloned().collect();
        let moved = values.iter().any(|t| default_of(defaults, section, &t.name).is_some_and(|d| tuning_layout::changed(t, d)));
        let title = format!("{group}  ({}){}", values.len(), if moved { " •" } else { "" });
        egui::CollapsingHeader::new(title).id_salt((section, group)).open(fold).show(ui, |ui| {
            rows(ui, section, group, values, target, defaults, favourites, toggled);
        });
    }
}

/// A grid of slider rows for `values` of `section`: a star, the name (its help on hover, a dot
/// when off its default) and the slider. Right-click resets a value or stars it. A star
/// clicked lands in `toggled`.
#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut egui::Ui,
    section: &str,
    salt: &str,
    values: Vec<Tunable>,
    target: &mut dyn Tunables,
    defaults: &[(String, Vec<Tunable>)],
    favourites: &Favourites,
    toggled: &mut Option<(String, String)>,
) {
    if values.is_empty() {
        return;
    }
    egui::Grid::new((section, salt, "rows")).num_columns(3).show(ui, |ui| {
        for t in values {
            let default = default_of(defaults, section, &t.name);
            let starred = favourites.has(section, &t.name);
            let star = ui.add(egui::Button::new(if starred { "★" } else { "☆" }).frame(false).small());
            if star.on_hover_text(if starred { "Unfavourite" } else { "Favourite" }).clicked() {
                *toggled = Some((section.to_owned(), t.name.clone()));
            }
            let moved = default.is_some_and(|d| tuning_layout::changed(&t, d));
            let text = if moved {
                egui::RichText::new(format!("{} •", t.name)).color(ui.visuals().warn_fg_color)
            } else {
                egui::RichText::new(&t.name)
            };
            let label = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            let label = label.on_hover_text(tuning_layout::tooltip(section, &t, default));
            let mut value = t.value;
            let slider = ui.add(egui::Slider::new(&mut value, t.min..=t.max));
            if slider.changed() {
                target.set_tunable(&t.name, value);
            }
            label.union(slider).context_menu(|ui| {
                if let Some(d) = default
                    && ui.add_enabled(moved, egui::Button::new(format!("Reset to {}", tuning_layout::number(d)))).clicked()
                {
                    target.set_tunable(&t.name, d);
                    ui.close();
                }
                if ui.button(if starred { "Unfavourite" } else { "Favourite" }).clicked() {
                    *toggled = Some((section.to_owned(), t.name.clone()));
                    ui.close();
                }
            });
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
    fn the_clock_reads_hours_and_minutes() {
        assert_eq!(clock_text(0.0), "00:00");
        assert_eq!(clock_text(17.0 + 5.5 / 60.0), "17:05");
        assert_eq!(clock_text(23.999), "23:59");
    }

    #[test]
    fn only_changed_values_are_sent() {
        let before = vec![TuningSection {
            name: "health".into(),
            values: vec![Tunable::new("regen", 1.0, 0.0, 5.0), Tunable::new("health", 10.0, 1.0, 50.0)],
        }];
        let mut edited: Vec<(String, Values)> = before.iter().map(|s| (s.name.clone(), Values(s.values.clone()))).collect();
        assert!(changes(&before, &edited).is_empty());
        edited[0].1.set_tunable("health", 20.0);
        assert_eq!(
            changes(&before, &edited),
            [Command::Tune { section: "health".into(), name: "health".into(), value: 20.0 }]
        );
    }

    #[test]
    fn only_terrain_values_and_its_size_need_generating_again() {
        let tune = |section: &str, name: &str| Command::Tune { section: section.into(), name: name.into(), value: 1.0 };
        assert!(shapes_terrain(&tune("terrain base:hills", "seed")));
        assert!(shapes_terrain(&tune("world", "terrain_width")));
        assert!(!shapes_terrain(&tune("world", "sim_distance")));
        assert!(!shapes_terrain(&tune("health", "regen")));
    }

    #[test]
    fn printout_pastes_as_struct_fields() {
        let out = printout(&[Tunable::new("walk_speed", 5.0, 0.0, 10.0), Tunable::new("gravity", 25.5, 0.0, 80.0)]);
        assert_eq!(out, "walk_speed: 5.0,\ngravity: 25.5,\n");
    }
}
