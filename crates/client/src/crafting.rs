//! The crafting menu: opened at a face of a crafting table with the interact key, it shows the
//! face's 4×4 grid, what the player carries, and a picture of what the pattern makes. Pressing
//! something carried picks it as the brush, and holding the button on the way to the grid fills
//! every cell passed over, so picking and dragging are one gesture. Clicking or sweeping cells
//! fills them with the brush; the right button empties them, and right-clicking the list drops
//! the brush. The craft button sends the craft with the next tick. The simulation decides
//! what's made; this only asks and shows.

use api::crafting::GRID;
use api::{BoxelPos, ShapeGrid, TableFace, Vec3 as ZVec3};
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use sim::{BlockRegistry, Craft, CraftRefusal, MaterialId, Simulation};

use crate::controls::{Controls, label};
use crate::editing::{Palette, PartNames, short};
use crate::{Look, Sim};

/// The menu's state. Its cells stay filled between openings, so a pattern can be made again.
#[derive(Resource, Default)]
pub struct CraftingMenu {
    /// The table and face it's open at, if it's open.
    pub open: Option<(BoxelPos, TableFace)>,
    /// A material or part in each cell, by row from the top and column from the left.
    pub cells: [[Option<MaterialId>; GRID]; GRID],
    /// What clicking a cell puts there.
    pub brush: Option<MaterialId>,
    /// The stroke under way, if a button went down on the list or the grid and is still down.
    stroke: Option<Stroke>,
    /// A craft to send with the next tick.
    pub pending: Option<Craft>,
    /// What the last craft sent did.
    pub result: Option<Result<(MaterialId, u32), CraftRefusal>>,
    /// A part to throw away with the next tick.
    pub discard: Option<MaterialId>,
    /// The part being renamed, and the name typed so far.
    renaming: Option<(MaterialId, String)>,
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
        // While a name is typed, the interact key is just a letter.
        let closing = keys.just_pressed(controls.interact) && menu.renaming.is_none();
        if closing || unreachable {
            menu.renaming = None;
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
/// How many "craft many" makes when there's no carrying limit.
const MANY: u32 = 64;

/// Draws the menu while it's open.
pub(crate) fn crafting_menu(
    mut contexts: EguiContexts,
    sim: Res<Sim>,
    mut menu: ResMut<CraftingMenu>,
    mut palette: ResMut<Palette>,
    mut names: ResMut<PartNames>,
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
                    ui.label("Pattern: click or hold to fill, right button empties");
                    let CraftingMenu { cells, brush, stroke, .. } = &mut *menu;
                    grid(ui, blocks, &names, cells, *brush, stroke);
                    if ui.button("Clear").clicked() {
                        menu.cells = Default::default();
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label("Carried: press to pick, hold to paint");
                    carried(ui, sim, &mut palette, &mut names, &mut menu);
                });
                ui.separator();
                ui.vertical(|ui| result(ui, sim, &mut menu, &mut palette, &names, shift));
            });
        });
    Ok(())
}

/// A press that paints or empties the cells it passes over, until the button comes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stroke {
    /// The left button, from the list or a cell: fills with the brush.
    Fill,
    /// The right button, from a cell: empties.
    Empty,
}

/// What a stroke does to a cell under the pointer: `Some(new contents)`, or `None` to leave it.
fn paint(stroke: Option<Stroke>, brush: Option<MaterialId>) -> Option<Option<MaterialId>> {
    match stroke? {
        Stroke::Fill => brush.map(Some),
        Stroke::Empty => Some(None),
    }
}

/// The 4×4 grid, painted by strokes.
fn grid(
    ui: &mut egui::Ui,
    blocks: &BlockRegistry,
    names: &PartNames,
    cells: &mut [[Option<MaterialId>; GRID]; GRID],
    brush: Option<MaterialId>,
    stroke: &mut Option<Stroke>,
) {
    let (primary, secondary) = ui.ctx().input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
    if !primary && !secondary {
        *stroke = None;
    }
    egui::Grid::new("crafting_grid").spacing([2.0, 2.0]).show(ui, |ui| {
        for row in cells.iter_mut() {
            for cell in row.iter_mut() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click_and_drag());
                // A press on a cell starts a stroke there.
                if response.is_pointer_button_down_on() && stroke.is_none() {
                    *stroke = if secondary { Some(Stroke::Empty) } else { Some(Stroke::Fill) };
                }
                let over = response.contains_pointer();
                if over && let Some(new) = paint(*stroke, brush) {
                    *cell = new;
                }
                let painter = ui.painter_at(rect);
                let outline = egui::Stroke::new(if over { 2.0_f32 } else { 1.0 }, egui::Color32::from_gray(150));
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                if let Some(id) = *cell {
                    paint_thing(&painter, rect.shrink(3.0), blocks, id);
                }
                painter.rect_stroke(rect, 2.0, outline, egui::StrokeKind::Inside);
                if let Some(id) = *cell {
                    response.on_hover_text(names.label(blocks, id));
                }
            }
            ui.end_row();
        }
    });
}

/// Everything carried. Pressing one picks it as the brush and starts a stroke, so holding on
/// the way to the grid paints; right-clicking drops the brush. A part also has buttons to hold
/// it, rename it and throw all of it away.
fn carried(ui: &mut egui::Ui, sim: &Simulation, palette: &mut Palette, names: &mut PartNames, menu: &mut CraftingMenu) {
    let blocks = sim.blocks();
    let things: Vec<(MaterialId, u32)> = sim.inventory().iter().collect();
    if things.is_empty() {
        ui.label("nothing");
    }
    egui::ScrollArea::vertical().max_height(4.0 * CELL + 30.0).show(ui, |ui| {
        for (id, amount) in things {
            ui.horizontal(|ui| {
                let amount = match blocks.part_of(id) {
                    Some(_) => format!("×{}", amount / blocks.unit(id)),
                    None => crate::items::amount_label(amount),
                };
                let picked = menu.brush == Some(id);
                let text = egui::WidgetText::from(format!("{} {amount}", names.label(blocks, id)));
                let galley = text.into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button);
                let size = egui::vec2(ICON + 6.0 + galley.size().x + 8.0, ICON.max(galley.size().y) + 4.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                if response.is_pointer_button_down_on() && ui.ctx().input(|i| i.pointer.primary_down()) {
                    menu.brush = Some(id);
                    menu.stroke = Some(Stroke::Fill);
                }
                if response.secondary_clicked() {
                    menu.brush = None;
                }
                let visuals = ui.style().interact_selectable(&response, picked);
                ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
                let icon = egui::Rect::from_min_size(rect.min + egui::vec2(2.0, 2.0), egui::vec2(ICON, ICON));
                paint_thing(ui.painter(), icon, blocks, id);
                let at = egui::pos2(icon.max.x + 6.0, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(at, galley, visuals.text_color());
                response.on_hover_text("press to paint with it, hold to paint on the grid; right-click to put it down");
                if blocks.part_of(id).is_none() {
                    return;
                }
                if ui.small_button("hold").on_hover_text("put it in hand").clicked() {
                    palette.select_part(sim, id);
                }
                match &mut menu.renaming {
                    Some((part, typed)) if *part == id => {
                        let field = ui.add(egui::TextEdit::singleline(typed).desired_width(110.0).hint_text("a name"));
                        field.request_focus();
                        let done = ui.small_button("ok").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if done {
                            names.set(blocks, id, typed);
                            menu.renaming = None;
                        }
                    }
                    _ => {
                        if ui.small_button("rename").clicked() {
                            menu.renaming = Some((id, names.given(blocks, id).unwrap_or_default().to_owned()));
                        }
                    }
                }
                if ui.small_button("discard").on_hover_text("throw all of them away").clicked() {
                    menu.discard = Some(id);
                    if menu.brush == Some(id) {
                        menu.brush = None;
                    }
                }
            });
        }
    });
}

/// The picture of what the pattern makes, what it costs, and the craft buttons.
fn result(
    ui: &mut egui::Ui,
    sim: &Simulation,
    menu: &mut CraftingMenu,
    palette: &mut Palette,
    names: &PartNames,
    shift: bool,
) {
    let blocks = sim.blocks();
    // With no limit, "as many as fit" would be billions: make a stack at a time instead.
    let most = if sim.capacity().is_some() { u32::MAX } else { MANY };
    let plan = menu.request(most).map(|craft| sim.craft_plan(&craft));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(25));
    match &plan {
        Some(Ok(plan)) => {
            let color = blocks.get(blocks.material_block(plan.material)).map_or([200; 3], |d| d.color);
            paint_shape(ui.painter(), rect.shrink(12.0), &plan.shape, color);
            let material = short(blocks.material_name(plan.material));
            ui.label(format!(
                "{} of a block of {material} each, free to make; {} at once",
                crate::editing::fraction(plan.volume),
                plan.count
            ));
            ui.horizontal(|ui| {
                let one = ui.button("Craft").on_hover_text("shift-click crafts as many at once");
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
            ui.label(format!("made {count}: {} (in hand)", names.label(blocks, *part)));
        }
        Some(Err(why)) => {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 100), why.to_string());
        }
        None => {}
    }
}

/// A material as a solid square of its color, or a part as a picture of its shape.
pub(crate) fn paint_thing(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, id: MaterialId) {
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
    fn strokes_fill_with_the_brush_and_the_right_button_empties() {
        let mut blocks = BlockRegistry::default();
        blocks.register(api::BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        let stone = blocks.material("test:stone").unwrap();
        // No stroke under way: nothing changes, brush or not.
        assert_eq!(paint(None, Some(stone)), None);
        // A left-button stroke, from the list or a cell, fills with the brush.
        assert_eq!(paint(Some(Stroke::Fill), Some(stone)), Some(Some(stone)));
        assert_eq!(paint(Some(Stroke::Fill), None), None, "no brush, nothing to paint");
        // A right-button stroke empties.
        assert_eq!(paint(Some(Stroke::Empty), Some(stone)), Some(None));
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
