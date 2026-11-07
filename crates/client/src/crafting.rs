//! The crafting menu: opened at a crafting table with the interact key, it shows the table's
//! top as a 4×4 grid of stacks, what the player carries, and a picture of what the stacks make.
//! Pressing something carried picks it as the brush, and holding the button on the way to the
//! grid paints, so picking and painting are one gesture. Clicking a cell adds a layer of the
//! brush on top of its stack; dragging on adds a layer to every cell as high as the first one,
//! so a sweep levels a row. The right button takes the top layer off the same way. The turn key
//! turns a part brush a quarter turn. The craft button sends the craft with the next tick. The
//! simulation decides what's made; this only asks and shows.

use std::collections::HashMap;

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
    /// Each view's stacks, the top's then the side's: each cell's stack, bottom first, by row
    /// from the far side (or the top of the wall) and column from the left. Each view keeps its
    /// own, so switching never makes the same stacks mean another shape.
    pub grids: [Cells; 2],
    /// Each view's flip: turn the made shape upside down.
    pub flips: [bool; 2],
    /// Build on the table's side: the grid is the wall, and stacks grow toward the player. Kept
    /// between openings, so the menu opens in the view last used.
    pub side: bool,
    /// What clicking a cell adds: a material, or a part turned some quarter turns.
    pub brush: Option<Piece>,
    /// The stroke under way, if a button went down on the list or the grid and is still down.
    stroke: Option<Stroke>,
    /// A craft to send with the next tick.
    pub pending: Option<Craft>,
    /// The craft last sent, so what it makes can be loaded back into the grid.
    sent: Option<Craft>,
    /// What the last craft sent did.
    pub result: Option<Result<(MaterialId, u32), CraftRefusal>>,
    /// A part to throw away with the next tick.
    pub discard: Option<MaterialId>,
    /// The part being renamed, and the name typed so far.
    renaming: Option<(MaterialId, String)>,
    /// What made each part crafted this session, by the part's name.
    recipes: HashMap<String, Recipe>,
    /// Where the grid's column, the carried list and the result's column were drawn last frame.
    columns: [Option<egui::Rect>; 3],
}

/// What makes a part: the stacks, the view they're on and whether they're flipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub cells: Cells,
    pub side: bool,
    pub flip: bool,
}

impl CraftingMenu {
    /// The open view's index into `grids` and `flips`.
    fn view(&self) -> usize {
        self.side as usize
    }

    /// The open view's stacks.
    pub fn cells(&self) -> &Cells {
        &self.grids[self.view()]
    }

    pub fn cells_mut(&mut self) -> &mut Cells {
        let view = self.view();
        &mut self.grids[view]
    }

    /// The open view's flip.
    pub fn flip(&self) -> bool {
        self.flips[self.view()]
    }

    /// The craft the open view's stacks make at the open table, as many as asked for.
    fn request(&self, count: u32) -> Option<Craft> {
        let table = self.open?;
        Some(Craft { table, cells: *self.cells(), side: self.side, flip: self.flip(), count })
    }

    /// The craft to send with this tick, if one is waiting; remembered until its outcome
    /// comes back.
    pub fn take_pending(&mut self) -> Option<Craft> {
        let craft = self.pending.take();
        if craft.is_some() {
            self.sent = craft;
        }
        craft
    }

    /// A craft's outcome came back: remembers what made the part, so it can be loaded.
    pub fn crafted(&mut self, blocks: &BlockRegistry, result: Result<(MaterialId, u32), CraftRefusal>) {
        if let (Ok((part, _)), Some(sent)) = (result, self.sent.take()) {
            let recipe = Recipe { cells: sent.cells, side: sent.side, flip: sent.flip };
            self.recipes.insert(blocks.material_name(part).to_owned(), recipe);
        }
        self.result = Some(result);
    }

    /// What makes part or bundle `id`: how it was crafted this session, or else worked out
    /// from its shape (see [`rebuild`]).
    pub fn recipe(&self, blocks: &BlockRegistry, id: MaterialId) -> Option<Recipe> {
        self.recipes.get(blocks.material_name(id)).copied().or_else(|| rebuild(blocks, id))
    }

    /// Puts `recipe` on the table: switches to its view and sets that view's stacks and flip.
    pub fn load(&mut self, recipe: Recipe) {
        self.side = recipe.side;
        let view = self.view();
        self.grids[view] = recipe.cells;
        self.flips[view] = recipe.flip;
    }
}

/// Stacks that make part or bundle `id`, worked out from its shape: each whole quarter of
/// each piece a layer of its material. Tries the top view, then flipped, then the side, then
/// flipped, and gives the first whose stacks rest. `None` for a material, or a shape with
/// detail finer than quarters (which needed parts in layers to make).
pub fn rebuild(blocks: &BlockRegistry, id: MaterialId) -> Option<Recipe> {
    let pieces: Vec<(MaterialId, ShapeGrid)> = match blocks.pieces(id) {
        [] => vec![(blocks.part_of(id)?, blocks.shape(blocks.material_block(id)))],
        pieces => pieces.iter().map(|&b| Some((blocks.made_of(b)?, blocks.shape(b)))).collect::<Option<_>>()?,
    };
    for (side, flip) in [(false, false), (false, true), (true, false), (true, true)] {
        // As built: undo the flip, then the tip (see `api::crafting::finished`).
        let as_built = |shape: &ShapeGrid| {
            let shape = if flip { api::crafting::flipped(shape) } else { shape.clone() };
            if side { api::crafting::untipped(&shape) } else { shape }
        };
        let mut cells: Cells = Default::default();
        let whole = pieces.iter().all(|(m, shape)| {
            let Some(quarters) = as_built(shape).quarters() else {
                return false;
            };
            for bit in (0..64).filter(|b| quarters & (1 << b) != 0) {
                let (qx, qy, qz) = (bit % 4, bit / 4 % 4, bit / 16);
                cells[GRID - 1 - qy][qx][qz] = Some(Piece::new(*m));
            }
            true
        });
        let stacks: api::Stacks = cells.map(|row| row.map(|stack| stack.map(|p| p.map(|_| api::Layer::Solid))));
        if whole && api::crafting::stack_shape(&stacks).is_ok() {
            return Some(Recipe { cells, side, flip });
        }
    }
    None
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

/// The material of the trigger piece the player looks at, if it's one.
fn aimed_trigger(world: &ClientWorld, look: &Look) -> Option<String> {
    let hit = world.target(look.yaw, look.pitch)?;
    let blocks = world.blocks();
    let occupant = *world.universe().main().cell(hit.pos).get(hit.occupant)?;
    blocks.get(occupant.block).filter(|def| def.has(api::Property::Trigger))?;
    Some(blocks.made_of(occupant.block).map_or("trigger", |m| short(blocks.material_name(m))).to_owned())
}

/// A hint by the crosshair when it's on a table or a trigger.
pub(crate) fn update_table_hint(
    world: Res<ClientWorld>,
    look: Res<Look>,
    menu: Res<CraftingMenu>,
    controls: Res<Controls>,
    mut text: Single<&mut Text, With<TableHint>>,
) {
    let key = label(controls.interact);
    let line = match (aimed_table(&world, &look), aimed_trigger(&world, &look)) {
        _ if menu.open.is_some() => String::new(),
        (Some(_), _) => format!("crafting table: {key} to craft"),
        (_, Some(material)) => format!("{material}: {key} to work it"),
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
/// The grid's width: four cells and the gaps between them.
const GRID_WIDTH: f32 = 4.0 * CELL + 3.0 * 2.0;
/// The carried list's width. Fixed, like every column, so what's carried never moves the grid.
const LIST_WIDTH: f32 = 380.0;
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
    show_menu(ctx, &world, &controls, &mut menu, &mut palette, &mut names);
    Ok(())
}

/// The menu's window, drawn into `ctx`.
fn show_menu(
    ctx: &egui::Context,
    world: &ClientWorld,
    controls: &Controls,
    menu: &mut CraftingMenu,
    palette: &mut Palette,
    names: &mut PartNames,
) {
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
            // Columns have fixed widths, and text wraps to them, so the window never changes
            // width (it stays centered, so a wider window would move the grid).
            let gaps = 2.0 * (2.0 * ui.spacing().item_spacing.x + 6.0);
            ui.set_max_width(GRID_WIDTH + LIST_WIDTH + PREVIEW + gaps);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut menu.side, false, "Build on the top");
                ui.selectable_value(&mut menu.side, true, "Build on the side");
            });
            ui.label(if menu.side {
                "Build on the table's side, the grid as the wall in front of you. Stacks grow out of the wall toward \
                 you, and nothing may float off it. Placed, the wall side points the way you look."
            } else {
                "Build on the table top, seen from its front. Stacks rise from the table, and nothing may float. \
                 Placed, the far side points the way you look."
            });
            ui.horizontal_top(|ui| {
                let grid_column = ui.vertical(|ui| {
                    ui.set_width(GRID_WIDTH);
                    ui.label("Click to add a layer, hold to level a row; right button takes one off");
                    let view = menu.view();
                    let CraftingMenu { grids, brush, stroke, .. } = &mut *menu;
                    grid(ui, blocks, names, &mut grids[view], *brush, stroke, bad, view == 1);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            *menu.cells_mut() = Default::default();
                        }
                        let turn = format!("Turn brush ({})", label(controls.turn_part));
                        let can_turn = menu.brush.is_some_and(|b| blocks.part_of(b.thing).is_some());
                        if ui.add_enabled(can_turn, egui::Button::new(turn)).clicked()
                            && let Some(brush) = &mut menu.brush
                        {
                            brush.turns = (brush.turns + 1) % 4;
                        }
                    });
                    brush_line(ui, blocks, names, menu.brush, menu.side);
                });
                ui.separator();
                // The tallest column's bottom: the result's is last frame's, as it comes after.
                let result_bottom = menu.columns[2].map_or(f32::MIN, |r| r.bottom());
                let bottom = grid_column.response.rect.bottom().max(result_bottom);
                ui.vertical(|ui| {
                    ui.set_width(LIST_WIDTH);
                    ui.label("Carried: press to pick, hold to paint");
                    // Down to the bottom of the tallest column.
                    let height = (bottom - ui.cursor().top()).max(CELL);
                    carried(ui, world, palette, names, menu, height);
                });
                menu.columns[0] = Some(grid_column.response.rect);
                ui.separator();
                let result_column = ui.vertical(|ui| {
                    ui.set_width(PREVIEW);
                    result(ui, world, menu, palette, names, plan, shift);
                });
                menu.columns[2] = Some(result_column.response.rect);
            });
        });
}

/// What the brush is, and how it's turned.
fn brush_line(ui: &mut egui::Ui, blocks: &BlockRegistry, names: &PartNames, brush: Option<Piece>, side: bool) {
    let Some(brush) = brush else {
        ui.label("brush: none (press something carried)");
        return;
    };
    ui.horizontal(|ui| {
        ui.label("brush:");
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
        paint_piece(ui.painter(), rect, blocks, brush, side);
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
#[allow(clippy::too_many_arguments)]
fn grid(
    ui: &mut egui::Ui,
    blocks: &BlockRegistry,
    names: &PartNames,
    cells: &mut Cells,
    brush: Option<Piece>,
    stroke: &mut Option<Stroke>,
    bad: Option<(usize, usize)>,
    side: bool,
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
                    paint_piece(&painter, rect.shrink(8.0), blocks, top, side);
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

/// Everything carried, in a list `height` tall. Pressing one picks it as the brush and starts a stroke, so holding on
/// the way to the grid paints; right-clicking drops the brush. A part also has buttons to hold
/// it, rename it and throw all of it away.
fn carried(
    ui: &mut egui::Ui,
    world: &ClientWorld,
    palette: &mut Palette,
    names: &mut PartNames,
    menu: &mut CraftingMenu,
    height: f32,
) {
    let blocks = world.blocks();
    let things: Vec<(MaterialId, u32)> = world.inventory().iter().collect();
    if things.is_empty() {
        ui.label("nothing");
    }
    let list = egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(height).show(ui, |ui| {
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
                let recipe = menu.recipe(blocks, id);
                let load = ui
                    .add_enabled(recipe.is_some(), egui::Button::new("load").small())
                    .on_hover_text("put what makes it on the table, to change it")
                    .on_disabled_hover_text("not made this session, and too fine to work out from its shape");
                if load.clicked()
                    && let Some(recipe) = recipe
                {
                    menu.load(recipe);
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
    menu.columns[1] = Some(list.inner_rect);
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
    let view = menu.view();
    ui.checkbox(&mut menu.flips[view], "Flip upside down").on_hover_text("for things easier built top first, like a table");
    match &plan {
        Some(Ok(plan)) => {
            let pieces: Vec<(ShapeGrid, [u8; 3])> =
                plan.pieces.iter().map(|(m, shape)| (shape.clone(), color_of(blocks, *m))).collect();
            paint_pieces(ui.painter(), rect.shrink(12.0), &pieces);
            let names: Vec<&str> = plan.pieces.iter().map(|(m, _)| short(blocks.material_name(*m))).collect();
            let material = names.join(" and ");
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

/// A layer as the grid shows it: a material's color, or a part's shape as it goes in: on the
/// side, tipped back into the wall's frame, then turned (see `sim::crafting::plan`).
fn paint_piece(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, piece: Piece, side: bool) {
    match blocks.part_of(piece.thing) {
        Some(_) => {
            let turn = Orientation::quarter_turns(2, piece.turns as i32);
            let framed = |s: ShapeGrid| if side { api::crafting::untipped(&s) } else { s }.turned(turn);
            let pieces: Vec<_> = pieces_of(blocks, piece.thing).into_iter().map(|(s, c)| (framed(s), c)).collect();
            paint_pieces(painter, rect, &pieces);
        }
        None => paint_thing(painter, rect, blocks, piece.thing),
    }
}

/// A material as a solid square of its color, or a part or bundle as a picture of its shape.
pub(crate) fn paint_thing(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, id: MaterialId) {
    match blocks.part_of(id) {
        Some(_) => paint_pieces(painter, rect, &pieces_of(blocks, id)),
        None => {
            let color = color_of(blocks, id);
            painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(color[0], color[1], color[2]));
        }
    }
}

/// A part's shape in its color, or each of a bundle's pieces in its own.
pub(crate) fn pieces_of(blocks: &BlockRegistry, id: MaterialId) -> Vec<(ShapeGrid, [u8; 3])> {
    let block_color = |b| blocks.get(b).map_or([200; 3], |d| d.color);
    match blocks.pieces(id) {
        [] => {
            let block = blocks.material_block(id);
            vec![(blocks.shape(block), block_color(block))]
        }
        pieces => pieces.iter().map(|&b| (blocks.shape(b), block_color(b))).collect(),
    }
}

/// A material's color: its block's.
fn color_of(blocks: &BlockRegistry, material: MaterialId) -> [u8; 3] {
    blocks.get(blocks.material_block(material)).map_or([200; 3], |d| d.color)
}

/// Where the pictures look from: in front of the shape (-Y, where someone at the table's front
/// stands), a little to the left and above, so the front, the left side and the top show.
const VIEW: [f32; 3] = [0.6, 1.0, -0.8];

/// One face of one sixteenth, as the picture draws it.
struct Quad {
    depth: f32,
    corners: [ZVec3; 4],
    shade: f32,
    color: [u8; 3],
}

/// The visible faces of `shape` from [`VIEW`], farthest first, in sixteenths.
#[cfg(test)]
fn quads(shape: &ShapeGrid) -> Vec<Quad> {
    quads_of(&[(shape.clone(), [0; 3])])
}

/// The visible faces of the pieces from [`VIEW`], each in its piece's color, farthest first,
/// in sixteenths. Where pieces overlap, the first one's color shows.
fn quads_of(pieces: &[(ShapeGrid, [u8; 3])]) -> Vec<Quad> {
    let color_at = |x: i32, y: i32, z: i32| {
        let inside = (0..16).contains(&x) && (0..16).contains(&y) && (0..16).contains(&z);
        if !inside {
            return None;
        }
        pieces.iter().find(|(s, _)| s.get(x as usize, y as usize, z as usize)).map(|(_, c)| *c)
    };
    let forward = ZVec3::from_array(VIEW).normalize();
    // The faces that turn toward the viewer, with the corners of a unit cube on them.
    let faces: [([i32; 3], [[f32; 3]; 4], f32); 3] = [
        ([-1, 0, 0], [[0., 0., 0.], [0., 1., 0.], [0., 1., 1.], [0., 0., 1.]], 0.62),
        ([0, -1, 0], [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]], 0.8),
        ([0, 0, 1], [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]], 1.0),
    ];
    let filled = |x: i32, y: i32, z: i32| color_at(x, y, z).is_some();
    let mut out = Vec::new();
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let Some(color) = color_at(x, y, z) else {
                    continue;
                };
                for (n, corners, shade) in &faces {
                    if filled(x + n[0], y + n[1], z + n[2]) {
                        continue;
                    }
                    let at = ZVec3::new(x as f32, y as f32, z as f32);
                    let corners = corners.map(|c| at + ZVec3::from_array(c));
                    let middle = (corners[0] + corners[2]) * 0.5;
                    out.push(Quad { depth: middle.dot(forward), corners, shade: *shade, color });
                }
            }
        }
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// Draws pieces together in `rect` as a little 3D picture, each in its color, seen from the
/// front.
pub(crate) fn paint_pieces(painter: &egui::Painter, rect: egui::Rect, pieces: &[(ShapeGrid, [u8; 3])]) {
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
    for quad in quads_of(pieces) {
        let color = quad.color;
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
    use crate::world::Link;
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

    /// The menu at the base game's table, the player standing in front of it.
    fn at_table() -> (Link, ClientWorld, CraftingMenu) {
        use crate::world::testing::{base_sim, connect};
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (x, y, z) = base::TABLE_SPOT;
        sim.teleport_player(api::WorldPos::new(api::I64Vec3::new(x, y - 2, z), ZVec3::new(0.5, 0.5, 0.0)));
        let (link, world) = connect(sim);
        let menu = CraftingMenu { open: Some(BoxelPos::new(x, y, z)), ..Default::default() };
        (link, world, menu)
    }

    /// Draws the menu for a few frames in `ctx`, as egui sizes a window over its first ones,
    /// and gives where the window went.
    fn draw(ctx: &egui::Context, world: &ClientWorld, menu: &mut CraftingMenu) -> egui::Rect {
        let (mut palette, mut names) = (Palette::default(), PartNames::default());
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
            ..Default::default()
        };
        for _ in 0..4 {
            let _ = ctx.run(input(), |ctx| show_menu(ctx, world, &Controls::default(), menu, &mut palette, &mut names));
        }
        ctx.memory(|m| m.area_rect(egui::Id::new("Crafting"))).expect("the window is drawn")
    }

    #[test]
    fn the_window_stays_put_when_the_preview_appears_or_a_part_is_made() {
        use crate::world::testing::step;
        let (link, world, mut menu) = at_table();
        let ctx = egui::Context::default();
        let empty = draw(&ctx, &world, &mut menu);
        let grid_at = menu.columns[0].unwrap().min;
        let brick = world.blocks().material(base::BRICK).unwrap();
        // A refusal, a line or two longer than the empty table's.
        menu.cells_mut()[0][0][1] = Some(Piece::new(brick));
        draw(&ctx, &world, &mut menu);
        assert_eq!(menu.columns[0].unwrap().min, grid_at, "a refusal doesn't move the grid");
        menu.cells_mut()[0][0][1] = None;
        for row in menu.cells_mut() {
            for stack in row {
                stack[0] = Some(Piece::new(brick));
                stack[1] = Some(Piece::new(brick));
            }
        }
        assert!(matches!(menu.request(1).map(|c| world.craft_plan(&c)), Some(Ok(_))), "a slab to preview");
        let slab = draw(&ctx, &world, &mut menu);
        assert_eq!((slab.min.x, slab.width()), (empty.min.x, empty.width()), "the grid doesn't move");
        assert_eq!(menu.columns[0].unwrap().min, grid_at, "nor up or down");
        // A crafted part's row, with its buttons, is wider than a material's: it used to widen
        // the window and push the grid aside.
        let (mut link, mut world) = (link, world);
        let craft = menu.request(1);
        step(&mut link, &mut world, &sim::PlayerInput { craft, ..Default::default() });
        menu.result = world.take_craft();
        assert!(matches!(menu.result, Some(Ok(_))), "crafted");
        let crafted = draw(&ctx, &world, &mut menu);
        assert_eq!((crafted.min.x, crafted.width()), (empty.min.x, empty.width()), "the grid still doesn't move");
        assert_eq!(menu.columns[0].unwrap().min, grid_at);
    }

    #[test]
    fn the_carried_list_fills_its_column() {
        let (_, world, mut menu) = at_table();
        draw(&egui::Context::default(), &world, &mut menu);
        let [Some(grid), Some(list), Some(result)] = menu.columns else { panic!("drawn") };
        let bottom = grid.max.y.max(result.max.y);
        assert!((list.max.y - bottom).abs() < 2.0, "the list ends with the tallest column: {list:?} {grid:?} {result:?}");
    }

    #[test]
    fn each_view_keeps_its_own_stacks_and_flip() {
        let (_, world, mut menu) = at_table();
        let brick = world.blocks().material(base::BRICK).unwrap();
        menu.cells_mut()[3][0][0] = Some(Piece::new(brick));
        menu.flips[0] = true;
        menu.side = true;
        assert_eq!(*menu.cells(), Cells::default(), "the side's grid is its own");
        assert!(!menu.flip());
        let side = menu.request(1).unwrap();
        assert!(side.side && side.cells == Cells::default());
        menu.side = false;
        assert_eq!(menu.cells()[3][0][0], Some(Piece::new(brick)), "the top's stacks are where they were left");
        assert!(menu.flip());
    }

    #[test]
    fn a_crafted_part_loads_back_as_it_was_made() {
        use crate::world::testing::step;
        let (mut link, mut world, mut menu) = at_table();
        let brick = world.blocks().material(base::BRICK).unwrap();
        menu.side = true;
        for col in 0..GRID {
            menu.cells_mut()[3][col][0] = Some(Piece::new(brick));
        }
        menu.flips[1] = true;
        menu.pending = menu.request(1);
        let made = *menu.cells();
        let craft = menu.take_pending();
        step(&mut link, &mut world, &sim::PlayerInput { craft, ..Default::default() });
        let result = world.take_craft().expect("an outcome");
        menu.crafted(world.blocks(), result);
        let Some(Ok((part, 1))) = menu.result else { panic!("{:?}", menu.result) };
        // Clear both views and go to the top, then load.
        menu.grids = Default::default();
        menu.flips = [false; 2];
        menu.side = false;
        let recipe = menu.recipe(world.blocks(), part).unwrap();
        assert_eq!(recipe, Recipe { cells: made, side: true, flip: true });
        menu.load(recipe);
        assert!(menu.side && menu.flip() && *menu.cells() == made);
        assert_eq!(menu.grids[0], Cells::default(), "the top's grid untouched");
    }

    #[test]
    fn parts_not_made_this_session_are_rebuilt_from_whole_quarters() {
        let (_, world, _) = at_table();
        let blocks = world.blocks();
        let stone = blocks.material(base::STONE).unwrap();
        let part = |shape: &ShapeGrid| blocks.drop(blocks.find_shaped(stone, shape).unwrap()).unwrap().material;
        // The test area's stairs: on the top, two layers near, four far.
        let stairs = rebuild(blocks, part(&base::shapes::stairs())).unwrap();
        assert!(!stairs.side && !stairs.flip);
        let height = |row: usize, col: usize| stairs.cells[row][col].iter().filter(|p| p.is_some()).count();
        assert_eq!([height(0, 0), height(1, 2), height(2, 1), height(3, 3)], [4, 4, 2, 2]);
        assert!(stairs.cells.iter().flatten().flatten().flatten().all(|p| p.thing == stone));
        // A post is finer than quarters.
        let wood = blocks.material(base::WOOD).unwrap();
        let post = blocks.drop(blocks.find_shaped(wood, &base::shapes::post()).unwrap()).unwrap().material;
        assert_eq!(rebuild(blocks, post), None);
        // The door as a bundle: its knob floats on the top, so it's rebuilt on the side.
        let mut blocks = world.blocks().clone();
        let pieces: Vec<_> =
            base::shapes::door_lower().into_iter().map(|(m, shape)| (blocks.material(m).unwrap(), shape)).collect();
        let door = blocks.bundle(&pieces).unwrap();
        let recipe = rebuild(&blocks, door).unwrap();
        assert!(recipe.side && !recipe.flip);
        let brass = blocks.material(base::BRASS).unwrap();
        assert_eq!(recipe.cells[1][3][1], Some(Piece::new(brass)), "the knob, one layer out");
    }

    #[test]
    fn the_request_needs_an_open_table() {
        let mut menu = CraftingMenu::default();
        assert_eq!(menu.request(1), None);
        menu.open = Some(BoxelPos::new(1, 2, 3));
        menu.flips[0] = true;
        let craft = menu.request(5).unwrap();
        assert_eq!((craft.table, craft.flip, craft.count), (BoxelPos::new(1, 2, 3), true, 5));
    }
}
