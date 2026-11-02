//! The crafting menu: opened at a face of a crafting table with the interact key, it shows the
//! face's 4×4 grid, what the player carries, and a picture of what the pattern makes. Things
//! carried are dragged onto cells (dragging across cells fills each), right-click empties a
//! cell, and the craft button sends the craft with the next tick. The simulation decides what's
//! made; this only asks and shows.

use api::crafting::GRID;
use api::{BoxelPos, ShapeGrid, TableFace, Vec3 as ZVec3};
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use sim::{BlockRegistry, Craft, CraftRefusal, MaterialId, Simulation};

use crate::controls::{Controls, label};
use crate::editing::{Palette, part_name, short};
use crate::{Look, Sim};

/// The menu's state. Its cells stay filled between openings, so a pattern can be made again.
#[derive(Resource, Default)]
pub struct CraftingMenu {
    /// The table and face it's open at, if it's open.
    pub open: Option<(BoxelPos, TableFace)>,
    /// A material or part in each cell, by row from the top and column from the left.
    pub cells: [[Option<MaterialId>; GRID]; GRID],
    /// A craft to send with the next tick.
    pub pending: Option<Craft>,
    /// What the last craft sent did.
    pub result: Option<Result<(MaterialId, u32), CraftRefusal>>,
}

impl CraftingMenu {
    /// The craft the cells make at the open table, as many as asked for.
    fn request(&self, count: u32) -> Option<Craft> {
        let (table, face) = self.open?;
        Some(Craft { table, face, cells: self.cells, count })
    }
}

/// Opens the menu at the table face aimed at with the interact key, and closes it with the
/// interact key again, or once the table is gone or out of reach. Escape closes it too (see
/// `grab_cursor`).
pub(crate) fn crafting_keys(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    sim: Res<Sim>,
    look: Res<Look>,
    mut menu: ResMut<CraftingMenu>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if menu.open.is_some() {
        let gone = menu.request(1).map(|craft| sim.0.craft_plan(&craft));
        let unreachable =
            matches!(gone, Some(Err(CraftRefusal::TooFar | CraftRefusal::NoTable | CraftRefusal::NoCrafting)));
        if keys.just_pressed(controls.interact) || unreachable {
            menu.open = None;
            crate::set_grab(&mut cursor, true);
        }
        return;
    }
    if !keys.just_pressed(controls.interact) || !crate::editing::playing(&cursor) {
        return;
    }
    let Some((pos, face)) = aimed_face(&sim.0, &look) else {
        return;
    };
    menu.open = Some((pos, face));
    menu.result = None;
    crate::set_grab(&mut cursor, false);
}

/// The crafting table and face the player looks at, if any.
fn aimed_face(sim: &Simulation, look: &Look) -> Option<(BoxelPos, TableFace)> {
    let hit = sim.target(look.yaw, look.pitch)?;
    sim.table_face(&hit).map(|face| (hit.pos, face))
}

/// A hint by the crosshair when it's on a usable table face.
pub(crate) fn update_table_hint(
    sim: Res<Sim>,
    look: Res<Look>,
    menu: Res<CraftingMenu>,
    controls: Res<Controls>,
    mut text: Single<&mut Text, With<TableHint>>,
) {
    let line = match aimed_face(&sim.0, &look) {
        Some((_, face)) if menu.open.is_none() => {
            format!("crafting table, {}: {} to craft", face.label(), label(controls.interact))
        }
        _ => String::new(),
    };
    if text.0 != line {
        text.0 = line;
    }
}

#[derive(Component)]
pub(crate) struct TableHint;

/// Cell size in the grid, in points.
const CELL: f32 = 48.0;
/// The size of the picture of the result.
const PREVIEW: f32 = 170.0;
/// The size of a part's picture in the list.
const ICON: f32 = 28.0;

/// Draws the menu while it's open.
pub(crate) fn crafting_menu(
    mut contexts: EguiContexts,
    sim: Res<Sim>,
    mut menu: ResMut<CraftingMenu>,
    mut palette: ResMut<Palette>,
) -> Result {
    let Some((_, face)) = menu.open else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let sim = &sim.0;
    let blocks = sim.blocks();
    let shift = ctx.input(|i| i.modifiers.shift);
    egui::Window::new("Crafting")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "{} of the table: shapes run through {}. A part is placed as if you stood at the table's front.",
                face.label(),
                face.pushes()
            ));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label("Pattern (drag here, right-click empties)");
                    grid(ui, blocks, &mut menu.cells);
                    if ui.button("Clear").clicked() {
                        menu.cells = Default::default();
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label("Carried (drag onto the grid)");
                    carried(ui, sim, &mut palette);
                });
                ui.separator();
                ui.vertical(|ui| result(ui, sim, &mut menu, &mut palette, shift));
            });
        });
    Ok(())
}

/// The 4×4 grid. A cell under the pointer takes whatever is being dragged, so dragging across
/// cells fills each one.
fn grid(ui: &mut egui::Ui, blocks: &BlockRegistry, cells: &mut [[Option<MaterialId>; GRID]; GRID]) {
    let dragging = ui.ctx().input(|i| i.pointer.primary_down());
    egui::Grid::new("crafting_grid").spacing([2.0, 2.0]).show(ui, |ui| {
        for row in cells.iter_mut() {
            for cell in row.iter_mut() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click());
                if let Some(id) = response.dnd_hover_payload::<MaterialId>()
                    && (dragging || response.dnd_release_payload::<MaterialId>().is_some())
                {
                    *cell = Some(*id);
                }
                if response.secondary_clicked() {
                    *cell = None;
                }
                let painter = ui.painter_at(rect);
                let hovered = response.contains_pointer();
                let stroke = egui::Stroke::new(if hovered { 2.0_f32 } else { 1.0 }, egui::Color32::from_gray(150));
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                if let Some(id) = *cell {
                    paint_thing(&painter, rect.shrink(3.0), blocks, id);
                }
                painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
                if let Some(id) = *cell {
                    response.on_hover_text(thing_name(blocks, id));
                }
            }
            ui.end_row();
        }
    });
}

/// Everything carried, each a drag source; a part also has a button to hold it.
fn carried(ui: &mut egui::Ui, sim: &Simulation, palette: &mut Palette) {
    let blocks = sim.blocks();
    let things: Vec<(MaterialId, u32)> = sim.inventory().iter().collect();
    if things.is_empty() {
        ui.label("nothing");
    }
    egui::ScrollArea::vertical().max_height(4.0 * CELL + 30.0).show(ui, |ui| {
        for (id, amount) in things {
            ui.horizontal(|ui| {
                let source = egui::Id::new(("crafting_source", id.index()));
                ui.dnd_drag_source(source, id, |ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
                    paint_thing(ui.painter(), rect, blocks, id);
                    let amount = match blocks.part_of(id) {
                        Some(_) => format!("×{}", amount / blocks.unit(id)),
                        None => crate::items::amount_label(amount),
                    };
                    ui.label(format!("{} {amount}", thing_name(blocks, id)));
                });
                if blocks.part_of(id).is_some() && ui.small_button("hold").on_hover_text("put it in hand").clicked() {
                    palette.select_part(sim, id);
                }
            });
        }
    });
}

/// The picture of what the pattern makes, what it costs, and the craft buttons.
fn result(ui: &mut egui::Ui, sim: &Simulation, menu: &mut CraftingMenu, palette: &mut Palette, shift: bool) {
    let blocks = sim.blocks();
    let plan = menu.request(u32::MAX).map(|craft| sim.craft_plan(&craft));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(25));
    match &plan {
        Some(Ok(plan)) => {
            let color = blocks.get(blocks.material_block(plan.material)).map_or([200; 3], |d| d.color);
            paint_shape(ui.painter(), rect.shrink(12.0), &plan.shape, color);
            let material = short(blocks.material_name(plan.material));
            ui.label(format!(
                "{} of a block of {material} each; you can make {}",
                crate::editing::fraction(plan.cost),
                plan.count
            ));
            ui.horizontal(|ui| {
                let one = ui.button("Craft").on_hover_text("shift-click crafts as many as you can");
                let all = ui.button(format!("Craft {}", plan.count));
                let count = if all.clicked() || (one.clicked() && shift) {
                    Some(plan.count)
                } else {
                    one.clicked().then_some(1)
                };
                if let Some(count) = count {
                    menu.pending = menu.request(count);
                }
            });
        }
        Some(Err(why)) => {
            ui.label(why.to_string());
        }
        None => {}
    }
    match menu.result.take() {
        Some(Ok((part, count))) => {
            palette.select_part(sim, part);
            menu.result = Some(Ok((part, count)));
        }
        other => menu.result = other,
    }
    match &menu.result {
        Some(Ok((part, count))) => {
            ui.label(format!("made {count}: {} (in hand)", part_name(blocks, *part)));
        }
        Some(Err(why)) => {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 100), why.to_string());
        }
        None => {}
    }
}

/// What a material or part is called in the menu.
fn thing_name(blocks: &BlockRegistry, id: MaterialId) -> String {
    match blocks.part_of(id) {
        Some(_) => part_name(blocks, id),
        None => short(blocks.material_name(id)).to_owned(),
    }
}

/// A material as a solid square of its color, or a part as a picture of its shape.
fn paint_thing(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, id: MaterialId) {
    let block = blocks.material_block(id);
    let color = blocks.get(block).map_or([200; 3], |d| d.color);
    match blocks.part_of(id) {
        Some(_) => paint_shape(painter, rect, &blocks.shape(block), color),
        None => {
            painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(color[0], color[1], color[2]));
        }
    }
}

/// Where the pictures look from: in front of the shape (-Y, where someone at the table's front
/// stands), a little to the left and above, so the front, the left side and the top show.
const VIEW: [f32; 3] = [0.6, 1.0, -0.8];

/// One face of one sixteenth, as the picture draws it.
struct Quad {
    depth: f32,
    corners: [ZVec3; 4],
    shade: f32,
}

/// The visible faces of `shape` from [`VIEW`], farthest first, in sixteenths.
fn quads(shape: &ShapeGrid) -> Vec<Quad> {
    let forward = ZVec3::from_array(VIEW).normalize();
    // The faces that turn toward the viewer, with the corners of a unit cube on them.
    let faces: [([i32; 3], [[f32; 3]; 4], f32); 3] = [
        ([-1, 0, 0], [[0., 0., 0.], [0., 1., 0.], [0., 1., 1.], [0., 0., 1.]], 0.62),
        ([0, -1, 0], [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]], 0.8),
        ([0, 0, 1], [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]], 1.0),
    ];
    let filled = |x: i32, y: i32, z: i32| {
        (0..16).contains(&x) && (0..16).contains(&y) && (0..16).contains(&z) && shape.get(x as usize, y as usize, z as usize)
    };
    let mut out = Vec::new();
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                if !filled(x, y, z) {
                    continue;
                }
                for (n, corners, shade) in &faces {
                    if filled(x + n[0], y + n[1], z + n[2]) {
                        continue;
                    }
                    let at = ZVec3::new(x as f32, y as f32, z as f32);
                    let corners = corners.map(|c| at + ZVec3::from_array(c));
                    let middle = (corners[0] + corners[2]) * 0.5;
                    out.push(Quad { depth: middle.dot(forward), corners, shade: *shade });
                }
            }
        }
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// Draws `shape` in `rect` as a little 3D picture in `color`, seen from the front.
pub(crate) fn paint_shape(painter: &egui::Painter, rect: egui::Rect, shape: &ShapeGrid, color: [u8; 3]) {
    let forward = ZVec3::from_array(VIEW).normalize();
    let right = forward.cross(ZVec3::Z).normalize();
    let up = right.cross(forward);
    let flat = |p: ZVec3| {
        let c = p - ZVec3::splat(8.0);
        egui::vec2(c.dot(right), -c.dot(up))
    };
    // Scale so the whole boxel's outline fits.
    let corners = (0..8).map(|i| flat(ZVec3::new((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32) * 16.0));
    let extent = corners.fold(egui::Vec2::ZERO, |m, c| m.max(c.abs()));
    let scale = (rect.width() / (2.0 * extent.x)).min(rect.height() / (2.0 * extent.y));
    let center = rect.center();
    for quad in quads(shape) {
        let shade = |c: u8| (c as f32 * quad.shade) as u8;
        let fill = egui::Color32::from_rgb(shade(color[0]), shade(color[1]), shade(color[2]));
        let points = quad.corners.iter().map(|&p| center + flat(p) * scale).collect();
        // A hairline of the same color closes the seams between neighboring faces.
        painter.add(egui::Shape::convex_polygon(points, fill, egui::Stroke::new(0.6_f32, fill)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::BlockBox;

    #[test]
    fn pictures_draw_only_the_faces_that_show() {
        // A full block shows one big face on each of three sides, in sixteenths.
        assert_eq!(quads(&ShapeGrid::FULL).len(), 3 * 16 * 16);
        assert!(quads(&ShapeGrid::EMPTY).is_empty());
        // A slab's top is lower, and its back half isn't drawn twice.
        let slab = ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [16, 16, 8])]);
        assert_eq!(quads(&slab).len(), 16 * 16 + 2 * 16 * 8);
        let q = quads(&slab);
        assert!(q.windows(2).all(|w| w[0].depth >= w[1].depth), "farthest first");
    }

    #[test]
    fn the_request_needs_an_open_table() {
        let mut menu = CraftingMenu::default();
        assert_eq!(menu.request(1), None);
        menu.open = Some((BoxelPos::new(1, 2, 3), TableFace::Top));
        let craft = menu.request(5).unwrap();
        assert_eq!((craft.table, craft.face, craft.count), (BoxelPos::new(1, 2, 3), TableFace::Top, 5));
    }
}
