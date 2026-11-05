//! The crafting menu: opened at a crafting table with the interact key, it shows the table's
//! top as a 4×4 grid of stacks, what the player carries, and a picture of what the stacks make.
//! Pressing something carried picks it as the brush, and holding the button on the way to the
//! grid paints, so picking and painting are one gesture. Clicking a cell adds a layer of the
//! brush on top of its stack; dragging on adds a layer to every cell as high as the first one,
//! so a sweep levels a row. The right button takes the top layer off the same way. The turn key
//! turns a part brush a quarter turn. The craft button sends the craft with the next tick. The
//! simulation decides what's made; this only asks and shows.

use api::crafting::GRID;
use api::{BoxelPos, CraftError, Orientation, ShapeGrid, Vec3 as ZVec3};
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use sim::{BlockRegistry, Cells, Craft, CraftRefusal, MaterialId, Piece};

use crate::world::ClientWorld;

use crate::controls::{Controls, label};
use crate::editing::{Palette, PartNames, short};
use crate::Look;

/// The menu's state. Its stacks stay between openings, so a shape can be made again.
#[derive(Resource, Default)]
pub struct CraftingMenu {
    /// The table it's open at, if it's open.
    pub open: Option<BoxelPos>,
    /// Each cell's stack, bottom first, by row from the far side and column from the left.
    pub cells: Cells,
    /// What clicking a cell adds: a material, or a part turned some quarter turns.
    pub brush: Option<Piece>,
    /// Turn the made shape upside down.
    pub flip: bool,
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
    /// The craft the stacks make at the open table, as many as asked for.
    fn request(&self, count: u32) -> Option<Craft> {
        let table = self.open?;
        Some(Craft { table, cells: self.cells, flip: self.flip, count })
    }
}

/// Opens the menu at the table aimed at with the interact key, and closes it with the interact
/// key again, or once the table is gone or out of reach. Escape closes it too (see
/// `grab_cursor`). While it's open, the turn key turns the brush.
pub(crate) fn crafting_keys(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    world: Res<ClientWorld>,
    look: Res<Look>,
    mut menu: ResMut<CraftingMenu>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if menu.open.is_some() {
        let gone = menu.request(1).map(|craft| world.craft_plan(&craft));
        let unreachable =
            matches!(gone, Some(Err(CraftRefusal::TooFar | CraftRefusal::NoTable | CraftRefusal::NoCrafting)));
        // While a name is typed, keys are just letters.
        let typing = menu.renaming.is_some();
        if keys.just_pressed(controls.turn_part) && !typing && let Some(brush) = &mut menu.brush {
            brush.turns = (brush.turns + 1) % 4;
        }
        if (keys.just_pressed(controls.interact) && !typing) || unreachable {
            menu.renaming = None;
            menu.open = None;
            crate::set_grab(&mut cursor, true);
        }
        return;
    }
    if !keys.just_pressed(controls.interact) || !crate::editing::playing(&cursor) {
        return;
    }
    let Some(pos) = aimed_table(&world, &look) else {
        return;
    };
    menu.open = Some(pos);
    menu.result = None;
    crate::set_grab(&mut cursor, false);
}

/// The crafting table the player looks at, if any.
fn aimed_table(world: &ClientWorld, look: &Look) -> Option<BoxelPos> {
    let hit = world.target(look.yaw, look.pitch)?;
    world.is_table(&hit).then_some(hit.pos)
}

/// A hint by the crosshair when it's on a table.
pub(crate) fn update_table_hint(
    world: Res<ClientWorld>,
    look: Res<Look>,
    menu: Res<CraftingMenu>,
    controls: Res<Controls>,
    mut text: Single<&mut Text, With<TableHint>>,
) {
    let line = match aimed_table(&world, &look) {
        Some(_) if menu.open.is_none() => format!("crafting table: {} to craft", label(controls.interact)),
        _ => String::new(),
    };
    if text.0 != line {
        text.0 = line;
    }
}

#[derive(Component)]
pub(crate) struct TableHint;

/// Cell size in the grid, in points.
const CELL: f32 = 52.0;
/// The size of the picture of the result.
const PREVIEW: f32 = 170.0;
/// The size of a part's picture in the list.
const ICON: f32 = 28.0;
/// How many "craft many" makes when there's no carrying limit.
const MANY: u32 = 64;

/// Draws the menu while it's open.
pub(crate) fn crafting_menu(
    mut contexts: EguiContexts,
    world: Res<ClientWorld>,
    controls: Res<Controls>,
    mut menu: ResMut<CraftingMenu>,
    mut palette: ResMut<Palette>,
    mut names: ResMut<PartNames>,
) -> Result {
    if menu.open.is_none() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let world = &world;
    let blocks = world.blocks();
    let shift = ctx.input(|i| i.modifiers.shift);
    let most = if world.capacity().is_some() { u32::MAX } else { MANY };
    let plan = menu.request(most).map(|craft| world.craft_plan(&craft));
    // The cell to mark, if the stacks are refused because of one.
    let bad = match &plan {
        Some(Err(CraftRefusal::Pattern(CraftError::Floating { row, col } | CraftError::TooFine { row, col, .. }))) => {
            Some((*row, *col))
        }
        _ => None,
    };
    egui::Window::new("Crafting")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0])
        .show(ctx, |ui| {
            ui.label(
                "Build on the table top, seen from its front. Stacks rise from the table, and nothing may float. \
                 Placed, the far side points the way you look.",
            );
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label("Click to add a layer, hold to level a row; right button takes one off");
                    let CraftingMenu { cells, brush, stroke, .. } = &mut *menu;
                    grid(ui, blocks, &names, cells, *brush, stroke, bad);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            menu.cells = Default::default();
                        }
                        let turn = format!("Turn brush ({})", label(controls.turn_part));
                        let can_turn = menu.brush.is_some_and(|b| blocks.part_of(b.thing).is_some());
                        if ui.add_enabled(can_turn, egui::Button::new(turn)).clicked()
                            && let Some(brush) = &mut menu.brush
                        {
                            brush.turns = (brush.turns + 1) % 4;
                        }
                    });
                    brush_line(ui, blocks, &names, menu.brush);
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label("Carried: press to pick, hold to paint");
                    carried(ui, world, &mut palette, &mut names, &mut menu);
                });
                ui.separator();
                ui.vertical(|ui| result(ui, world, &mut menu, &mut palette, &names, plan, shift));
            });
        });
    Ok(())
}

/// What the brush is, and how it's turned.
fn brush_line(ui: &mut egui::Ui, blocks: &BlockRegistry, names: &PartNames, brush: Option<Piece>) {
    let Some(brush) = brush else {
        ui.label("brush: none (press something carried)");
        return;
    };
    ui.horizontal(|ui| {
        ui.label("brush:");
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
        paint_piece(ui.painter(), rect, blocks, brush);
        let turned = match brush.turns {
            0 => String::new(),
            n => format!(", turned {} left", ["", "a quarter", "half", "three quarters"][n as usize]),
        };
        ui.label(format!("{}{turned}", names.label(blocks, brush.thing)));
    });
}

/// A press that adds or takes off layers in the cells it passes over, until the button comes up.
/// Each holds the height of the cells it changes: the first cell's, before the change, so a
/// sweep only touches cells as high as where it started, and each of those once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stroke {
    /// The left button, from the list or a cell: adds a layer of the brush. From the list, the
    /// height comes from the first cell reached.
    Add(Option<usize>),
    /// The right button, from a cell: takes the top layer off.
    Remove(usize),
}

/// How many layers a stack has.
fn height(stack: &[Option<Piece>; GRID]) -> usize {
    stack.iter().take_while(|l| l.is_some()).count()
}

/// What a stroke does to the stack under the pointer.
fn paint(stroke: &mut Option<Stroke>, brush: Option<Piece>, stack: &mut [Option<Piece>; GRID]) {
    let h = height(stack);
    match stroke {
        Some(Stroke::Add(level)) => {
            let Some(brush) = brush else {
                return;
            };
            if *level.get_or_insert(h) == h && h < GRID {
                stack[h] = Some(brush);
            }
        }
        Some(Stroke::Remove(level)) if *level == h && h > 0 => stack[h - 1] = None,
        Some(Stroke::Remove(_)) | None => {}
    }
}

/// The 4×4 grid of stacks, painted by strokes. `bad` is a cell to mark as the reason nothing
/// can be made.
fn grid(
    ui: &mut egui::Ui,
    blocks: &BlockRegistry,
    names: &PartNames,
    cells: &mut Cells,
    brush: Option<Piece>,
    stroke: &mut Option<Stroke>,
    bad: Option<(usize, usize)>,
) {
    let (primary, secondary) = ui.ctx().input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
    if !primary && !secondary {
        *stroke = None;
    }
    egui::Grid::new("crafting_grid").spacing([2.0, 2.0]).show(ui, |ui| {
        for (r, row) in cells.iter_mut().enumerate() {
            for (c, stack) in row.iter_mut().enumerate() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click_and_drag());
                // A press on a cell starts a stroke there.
                if response.is_pointer_button_down_on() && stroke.is_none() {
                    let h = height(stack);
                    *stroke = Some(if secondary { Stroke::Remove(h) } else { Stroke::Add(Some(h)) });
                }
                let over = response.contains_pointer();
                if over {
                    paint(stroke, brush, stack);
                }
                let h = height(stack);
                let painter = ui.painter_at(rect);
                // Taller stacks are lighter, so heights read at a glance.
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30 + 18 * h as u8));
                if let Some(top) = h.checked_sub(1).and_then(|i| stack[i]) {
                    paint_piece(&painter, rect.shrink(8.0), blocks, top);
                }
                if h > 0 {
                    let at = rect.right_bottom() - egui::vec2(4.0, 2.0);
                    painter.text(at, egui::Align2::RIGHT_BOTTOM, h.to_string(), egui::FontId::monospace(13.0), egui::Color32::WHITE);
                }
                let (width, color) = match () {
                    _ if bad == Some((r, c)) => (2.5_f32, egui::Color32::from_rgb(230, 90, 80)),
                    _ if over => (2.0, egui::Color32::from_gray(200)),
                    _ => (1.0, egui::Color32::from_gray(150)),
                };
                painter.rect_stroke(rect, 2.0, egui::Stroke::new(width, color), egui::StrokeKind::Inside);
                if h > 0 {
                    let layers: Vec<String> =
                        stack.iter().flatten().rev().map(|p| names.label(blocks, p.thing)).collect();
                    response.on_hover_text(format!("top to bottom:\n{}", layers.join("\n")));
                }
            }
            ui.end_row();
        }
    });
}

/// Everything carried. Pressing one picks it as the brush and starts a stroke, so holding on
/// the way to the grid paints; right-clicking drops the brush. A part also has buttons to hold
/// it, rename it and throw all of it away.
fn carried(ui: &mut egui::Ui, world: &ClientWorld, palette: &mut Palette, names: &mut PartNames, menu: &mut CraftingMenu) {
    let blocks = world.blocks();
    let things: Vec<(MaterialId, u32)> = world.inventory().iter().collect();
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
                let picked = menu.brush.is_some_and(|b| b.thing == id);
                let text = egui::WidgetText::from(format!("{} {amount}", names.label(blocks, id)));
                let galley = text.into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button);
                let size = egui::vec2(ICON + 6.0 + galley.size().x + 8.0, ICON.max(galley.size().y) + 4.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                if response.is_pointer_button_down_on() && ui.ctx().input(|i| i.pointer.primary_down()) {
                    // Picking the same thing again keeps its turn.
                    if !picked {
                        menu.brush = Some(Piece::new(id));
                    }
                    menu.stroke = Some(Stroke::Add(None));
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
                    palette.select_part(world, id);
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
                    if picked {
                        menu.brush = None;
                    }
                }
            });
        }
    });
}

/// The picture of what the stacks make, the flip switch, and the craft buttons.
fn result(
    ui: &mut egui::Ui,
    world: &ClientWorld,
    menu: &mut CraftingMenu,
    palette: &mut Palette,
    names: &PartNames,
    plan: Option<Result<sim::CraftPlan, CraftRefusal>>,
    shift: bool,
) {
    let blocks = world.blocks();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(25));
    ui.checkbox(&mut menu.flip, "Flip upside down").on_hover_text("for things easier built top first, like a table");
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
            palette.select_part(world, part);
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

/// A layer as the grid shows it: a material's color, or a part's shape turned as it goes in.
fn paint_piece(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, piece: Piece) {
    let block = blocks.material_block(piece.thing);
    match blocks.part_of(piece.thing) {
        Some(_) => {
            let color = blocks.get(block).map_or([200; 3], |d| d.color);
            let shape = blocks.shape(block).turned(Orientation::quarter_turns(2, piece.turns as i32));
            paint_shape(painter, rect, &shape, color);
        }
        None => paint_thing(painter, rect, blocks, piece.thing),
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
    fn strokes_add_and_take_off_layers_level_with_where_they_started() {
        let mut blocks = BlockRegistry::default();
        blocks.register(api::BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        let stone = Piece::new(blocks.material("test:stone").unwrap());
        let (mut low, mut high) = ([None; GRID], [None; GRID]);
        high[0] = Some(stone);
        // No stroke under way: nothing changes.
        paint(&mut None, Some(stone), &mut low);
        assert_eq!(height(&low), 0);
        // A stroke from the list takes its height from the first cell: here, empty.
        let mut stroke = Some(Stroke::Add(None));
        paint(&mut stroke, Some(stone), &mut low);
        assert_eq!((height(&low), stroke), (1, Some(Stroke::Add(Some(0)))));
        // Passing over the same cell again, or a taller one, adds nothing.
        paint(&mut stroke, Some(stone), &mut low);
        paint(&mut stroke, Some(stone), &mut high);
        assert_eq!((height(&low), height(&high)), (1, 1));
        // No brush, nothing to add.
        let mut empty = [None; GRID];
        paint(&mut Some(Stroke::Add(Some(0))), None, &mut empty);
        assert_eq!(height(&empty), 0);
        // The right button takes the top layer off cells as high as where it started.
        let mut stroke = Some(Stroke::Remove(1));
        paint(&mut stroke, None, &mut low);
        paint(&mut stroke, None, &mut empty);
        assert_eq!((height(&low), height(&empty)), (0, 0));
        // A full stack takes no more.
        let mut full = [Some(stone); GRID];
        paint(&mut Some(Stroke::Add(Some(GRID))), Some(stone), &mut full);
        assert_eq!(height(&full), GRID);
    }

    #[test]
    fn the_request_needs_an_open_table() {
        let mut menu = CraftingMenu::default();
        assert_eq!(menu.request(1), None);
        menu.open = Some(BoxelPos::new(1, 2, 3));
        menu.flip = true;
        let craft = menu.request(5).unwrap();
        assert_eq!((craft.table, craft.flip, craft.count), (BoxelPos::new(1, 2, 3), true, 5));
    }
}
