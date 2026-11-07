//! The crafting menu: opened at a crafting table with the interact key, it shows the table's
//! top as a 4×4 grid of stacks, what the player carries, and a picture of what the stacks make.
//! Pressing something carried picks it as the brush, and holding the button on the way to the
//! grid paints, so picking and painting are one gesture. Clicking a cell adds a layer of the
//! brush on top of its stack; dragging on adds a layer to every cell as high as the first one,
//! so a sweep levels a row. The right button takes the top layer off the same way. The turn key
//! turns a part brush a quarter turn. The craft button sends the craft with the next tick. The
//! simulation decides what's made; this only asks and shows.
//!
//! Beside them, the player's blueprints (see [`sim::blueprint`]) as a tree: a blueprint used in
//! another's layers folds under it, a variant under its base. A blueprint is picked and painted
//! like something carried, and the one picked can be made, loaded into the grid, renamed,
//! copied, deleted, or changed into variants: turned, mirrored, shifted, centered, a material
//! swapped. Pictures turn once now and then, and all the while they're hovered, so a shape's
//! hidden sides show.

use std::collections::{HashMap, HashSet};

use api::crafting::GRID;
use api::{BoxelPos, CraftError, Orientation, ShapeGrid, Vec3 as ZVec3};
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use sim::{
    BlockRegistry, BlueprintEdit, BlueprintId, BlueprintRefusal, Cells, Change, Craft, CraftRefusal, Design, Library, MaterialId,
    Piece, Recipe, Source, Thing, Variation,
};

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
    /// What clicking a cell adds: a material, or a part or blueprint turned some quarter turns.
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
    /// An edit to the blueprints to send with the next tick.
    pub blueprint_edit: Option<BlueprintEdit>,
    /// The edit last sent, until its outcome comes back.
    sent_edit: Option<BlueprintEdit>,
    /// The blueprint picked in the list, whose buttons show.
    pub selected: Option<BlueprintId>,
    /// The recipe blueprint whose stacks are in the grid, so they can be saved over it.
    pub editing: Option<BlueprintId>,
    /// The blueprint being renamed, and the name typed so far.
    naming: Option<(BlueprintId, String)>,
    /// Blueprints whose branch of the tree is folded shut.
    folded: HashSet<BlueprintId>,
    /// What the last blueprint edit did.
    pub blueprint_note: Option<Result<BlueprintId, BlueprintRefusal>>,
    /// For a swap: the material taken out and the one put in, as picked.
    swap: (Option<MaterialId>, Option<MaterialId>),
    /// Where the grid's column, the carried list, the result's column and the blueprints'
    /// column were drawn last frame.
    columns: [Option<egui::Rect>; 4],
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
        Some(Craft::grid(self.open?, *self.cells(), self.side, self.flip(), count))
    }

    /// Making blueprint `id` at the open table, as many as asked for.
    fn make(&self, id: BlueprintId, count: u32) -> Option<Craft> {
        Some(Craft { table: self.open?, source: Source::Blueprint(id), count })
    }

    /// The open view's stacks, as a blueprint keeps them.
    fn stacks(&self) -> Recipe {
        Recipe { cells: *self.cells(), side: self.side, flip: self.flip() }
    }

    /// Whether a name is being typed, so keys are just letters.
    fn typing(&self) -> bool {
        self.renaming.is_some() || self.naming.is_some()
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

    /// A craft's outcome came back: remembers what made the part, so it can be loaded. A part
    /// made from a blueprint goes by the blueprint's name, unless it has one already.
    pub fn crafted(
        &mut self,
        blocks: &BlockRegistry,
        library: &Library,
        names: &mut PartNames,
        result: Result<(MaterialId, u32), CraftRefusal>,
    ) {
        if let (Ok((part, _)), Some(sent)) = (result, self.sent.take()) {
            match sent.source {
                Source::Grid(recipe) => {
                    self.recipes.insert(blocks.material_name(part).to_owned(), recipe);
                }
                Source::Blueprint(id) => {
                    if let Some(b) = library.get(id)
                        && names.given(blocks, part).is_none()
                    {
                        names.set(blocks, part, &b.name);
                    }
                }
            }
        }
        self.result = Some(result);
    }

    /// The blueprint edit to send with this tick, if one is waiting; remembered until its
    /// outcome comes back.
    pub fn take_blueprint_edit(&mut self) -> Option<BlueprintEdit> {
        let edit = self.blueprint_edit.take();
        if edit.is_some() {
            self.sent_edit = edit.clone();
        }
        edit
    }

    /// A blueprint edit's outcome came back: picks what it made or changed. Saving stacks
    /// leaves them in the grid as that blueprint's, to save over again.
    pub fn blueprint_edited(&mut self, result: Result<BlueprintId, BlueprintRefusal>) {
        if let (Ok(id), Some(edit)) = (&result, self.sent_edit.take()) {
            let id = *id;
            match edit {
                BlueprintEdit::Delete { .. } => {
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                    if self.editing == Some(id) {
                        self.editing = None;
                    }
                    if self.brush.is_some_and(|b| b.thing == Thing::Blueprint(id)) {
                        self.brush = None;
                    }
                }
                BlueprintEdit::Save { .. } | BlueprintEdit::Overwrite { .. } => {
                    self.selected = Some(id);
                    self.editing = Some(id);
                }
                _ => self.selected = Some(id),
            }
        }
        self.blueprint_note = Some(result);
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
        let typing = menu.typing();
        if keys.just_pressed(controls.turn_part) && !typing && let Some(brush) = &mut menu.brush {
            brush.turns = (brush.turns + 1) % 4;
        }
        if (keys.just_pressed(controls.interact) && !typing) || unreachable {
            menu.renaming = None;
            menu.naming = None;
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
/// The size of a square in the carried grid.
const CARRIED_CELL: f32 = 46.0;
/// The grid's width: four cells and the gaps between them.
const GRID_WIDTH: f32 = 4.0 * CELL + 3.0 * 2.0;
/// The carried list's width. Fixed, like every column, so what's carried never moves the grid.
const LIST_WIDTH: f32 = 380.0;
/// How many "craft many" makes when there's no carrying limit.
const MANY: u32 = 64;
/// The blueprints' column's width.
const BLUEPRINT_WIDTH: f32 = 330.0;
/// The size of the picked blueprint's picture.
const BLUEPRINT_PREVIEW: f32 = 96.0;
/// How tall the blueprint tree gets before it scrolls.
const TREE_HEIGHT: f32 = 200.0;

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
    let library = world.blueprints();
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
        // Pinned by its top, so anything that makes it taller only pushes what's below down:
        // the grid and its buttons never move under the pointer.
        .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
        .show(ctx, |ui| {
            // Columns have fixed widths, and text wraps to them, so the window never changes
            // width (it stays centered, so a wider window would move the grid).
            let gaps = 3.0 * (2.0 * ui.spacing().item_spacing.x + 6.0);
            ui.set_max_width(GRID_WIDTH + LIST_WIDTH + PREVIEW + BLUEPRINT_WIDTH + gaps);
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
                    grid(ui, blocks, library, names, &mut grids[view], *brush, stroke, bad, view == 1);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            *menu.cells_mut() = Default::default();
                        }
                        let turn = format!("Turn brush ({})", label(controls.turn_part));
                        let can_turn = menu.brush.is_some_and(|b| match b.thing {
                            Thing::Material(m) => blocks.part_of(m).is_some(),
                            Thing::Blueprint(_) => true,
                        });
                        if ui.add_enabled(can_turn, egui::Button::new(turn)).clicked()
                            && let Some(brush) = &mut menu.brush
                        {
                            brush.turns = (brush.turns + 1) % 4;
                        }
                    });
                    save_buttons(ui, world, menu, &plan);
                    brush_line(ui, blocks, library, names, menu.brush, menu.side);
                });
                ui.separator();
                // The tallest column's bottom: the later columns' are last frame's, as they come
                // after.
                let later = [2, 3].map(|i| menu.columns[i].map_or(f32::MIN, |r| r.bottom()));
                let bottom = grid_column.response.rect.bottom().max(later[0]).max(later[1]);
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
                ui.separator();
                let blueprint_column = ui.vertical(|ui| {
                    ui.set_width(BLUEPRINT_WIDTH);
                    blueprints(ui, world, menu, shift, most);
                });
                menu.columns[3] = Some(blueprint_column.response.rect);
            });
        });
}

/// What the brush is, and how it's turned.
fn brush_line(ui: &mut egui::Ui, blocks: &BlockRegistry, library: &Library, names: &PartNames, brush: Option<Piece>, side: bool) {
    let Some(brush) = brush else {
        ui.label("brush: none (press something carried)");
        return;
    };
    ui.horizontal(|ui| {
        ui.label("brush:");
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
        paint_piece(ui.painter(), rect, blocks, library, brush, side);
        let turned = match brush.turns {
            0 => String::new(),
            n => format!(", turned {} left", ["", "a quarter", "half", "three quarters"][n as usize]),
        };
        // Cut to one line: a long name or a turn never widens the column (and so the window).
        ui.add(egui::Label::new(format!("{}{turned}", thing_label(blocks, library, names, brush.thing))).truncate());
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
    library: &Library,
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
                    paint_piece(&painter, rect.shrink(8.0), blocks, library, top, side);
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
                        stack.iter().flatten().rev().map(|p| thing_label(blocks, library, names, p.thing)).collect();
                    response.on_hover_text(format!("top to bottom:\n{}", layers.join("\n")));
                }
            }
            ui.end_row();
        }
    });
}

/// Everything carried, as a grid of squares `height` tall: each a picture and how many, named
/// in its tooltip. Pressing one picks it as the brush and starts a stroke, so holding on the
/// way to the grid paints. Right-clicking opens a menu: put the brush down, and for a part,
/// hold it, load it, rename it or throw all of it away. Squares never change size, whatever
/// their names, so nothing moves.
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
    let top = ui.cursor().top();
    // Renaming happens in a line above the grid.
    if let Some((part, typed)) = &mut menu.renaming {
        let part = *part;
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(format!("rename {}:", names.label(blocks, part))).truncate());
        });
        let done = ui
            .horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(typed).desired_width(LIST_WIDTH - 80.0).hint_text("a name"));
                field.request_focus();
                ui.small_button("ok").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
            })
            .inner;
        if done {
            names.set(blocks, part, typed);
            menu.renaming = None;
        }
    }
    let height = (height - (ui.cursor().top() - top)).max(CARRIED_CELL);
    let list = egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(height).show(ui, |ui| {
        let per_row = ((LIST_WIDTH + 2.0) / (CARRIED_CELL + 2.0)).floor().max(1.0) as usize;
        egui::Grid::new("carried_grid").spacing([2.0, 2.0]).show(ui, |ui| {
            for (n, (id, amount)) in things.into_iter().enumerate() {
                carried_square(ui, world, palette, names, menu, id, amount);
                if (n + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
    });
    menu.columns[1] = Some(list.inner_rect);
}

/// One carried thing's square in the grid, and its right-click menu.
fn carried_square(
    ui: &mut egui::Ui,
    world: &ClientWorld,
    palette: &mut Palette,
    names: &PartNames,
    menu: &mut CraftingMenu,
    id: MaterialId,
    amount: u32,
) {
    let blocks = world.blocks();
    let part = blocks.part_of(id).is_some();
    let amount = match part {
        true => format!("×{}", amount / blocks.unit(id)),
        false => crate::items::amount_label(amount),
    };
    let picked = menu.brush.is_some_and(|b| b.thing == Thing::Material(id));
    let (rect, response) = ui.allocate_exact_size(egui::vec2(CARRIED_CELL, CARRIED_CELL), egui::Sense::click_and_drag());
    if response.is_pointer_button_down_on() && ui.ctx().input(|i| i.pointer.primary_down()) {
        // Picking the same thing again keeps its turn.
        if !picked {
            menu.brush = Some(Piece::new(id));
        }
        menu.stroke = Some(Stroke::Add(None));
    }
    let visuals = ui.style().interact_selectable(&response, picked);
    ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    if picked {
        ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.5_f32, egui::Color32::from_gray(220)), egui::StrokeKind::Inside);
    }
    let icon = rect.shrink(7.0);
    match part {
        true => {
            let yaw = spin(ui, response.hovered(), id.index() as u64);
            paint_pieces_turned(ui.painter(), icon, &pieces_of(blocks, id), yaw);
        }
        false => paint_thing(ui.painter(), icon.shrink(3.0), blocks, id),
    }
    let at = rect.right_bottom() - egui::vec2(3.0, 1.0);
    let font = egui::FontId::proportional(11.0);
    // A dark edge, so the count reads on any color.
    ui.painter().text(at + egui::vec2(1.0, 1.0), egui::Align2::RIGHT_BOTTOM, &amount, font.clone(), egui::Color32::BLACK);
    ui.painter().text(at, egui::Align2::RIGHT_BOTTOM, &amount, font, egui::Color32::WHITE);
    let tip = format!("{} {amount}\npress to pick it, hold to paint on the grid; right-click for more", names.label(blocks, id));
    let response = response.on_hover_text(tip);
    response.context_menu(|ui| {
        if ui.add_enabled(picked, egui::Button::new("Put the brush down")).clicked() {
            menu.brush = None;
            ui.close();
        }
        if !part {
            return;
        }
        if ui.button("Hold it").on_hover_text("put it in hand").clicked() {
            palette.select_part(world, id);
            ui.close();
        }
        let recipe = menu.recipe(blocks, id);
        let load = ui
            .add_enabled(recipe.is_some(), egui::Button::new("Load it into the grid"))
            .on_hover_text("put what makes it on the table, to change it")
            .on_disabled_hover_text("not made this session, and too fine to work out from its shape");
        if load.clicked()
            && let Some(recipe) = recipe
        {
            menu.load(recipe);
            menu.editing = None;
            ui.close();
        }
        if ui.button("Rename").clicked() {
            menu.renaming = Some((id, names.given(blocks, id).unwrap_or_default().to_owned()));
            ui.close();
        }
        if ui.button("Throw all away").clicked() {
            menu.discard = Some(id);
            if picked {
                menu.brush = None;
            }
            ui.close();
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
    let (rect, preview) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(25));
    let view = menu.view();
    ui.checkbox(&mut menu.flips[view], "Flip upside down").on_hover_text("for things easier built top first, like a table");
    match &plan {
        Some(Ok(plan)) => {
            let pieces: Vec<(ShapeGrid, [u8; 3])> =
                plan.pieces.iter().map(|(m, shape)| (shape.clone(), color_of(blocks, *m))).collect();
            paint_pieces_turned(ui.painter(), rect.shrink(12.0), &pieces, spin(ui, preview.hovered(), u64::MAX));
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
            ui.add(egui::Label::new(format!("made {count}: {} (in hand)", names.label(blocks, *part))).truncate());
        }
        Some(Err(why)) => {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 100), why.to_string());
        }
        None => {}
    }
}

/// A layer as the grid shows it: a material's color, or a part's or blueprint's shape as it
/// goes in: on the side, tipped back into the wall's frame, then turned (see
/// `sim::crafting::design`).
fn paint_piece(painter: &egui::Painter, rect: egui::Rect, blocks: &BlockRegistry, library: &Library, piece: Piece, side: bool) {
    if let Thing::Material(m) = piece.thing
        && blocks.part_of(m).is_none()
    {
        return paint_thing(painter, rect, blocks, m);
    }
    let turn = Orientation::quarter_turns(2, piece.turns as i32);
    let framed = |s: ShapeGrid| if side { api::crafting::untipped(&s) } else { s }.turned(turn);
    match thing_pieces(blocks, library, piece.thing) {
        Some(pieces) => {
            let pieces: Vec<_> = pieces.into_iter().map(|(s, c)| (framed(s), c)).collect();
            paint_pieces(painter, rect, &pieces);
        }
        None => paint_missing(painter, rect),
    }
}

/// A blueprint that makes nothing: a dark red square with a question mark.
fn paint_missing(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(90, 30, 30));
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(rect.height() * 0.6), egui::Color32::WHITE);
}

/// The shape a part or blueprint makes, each piece in its material's color; `None` for a plain
/// material, or a blueprint that makes nothing.
fn thing_pieces(blocks: &BlockRegistry, library: &Library, thing: Thing) -> Option<Vec<(ShapeGrid, [u8; 3])>> {
    match thing {
        Thing::Material(m) => blocks.part_of(m).map(|_| pieces_of(blocks, m)),
        Thing::Blueprint(id) => library.resolve(blocks, id).ok().map(|pieces| colored(blocks, &pieces)),
    }
}

/// Each material's texels in its color.
fn colored(blocks: &BlockRegistry, pieces: &[(MaterialId, ShapeGrid)]) -> Vec<(ShapeGrid, [u8; 3])> {
    pieces.iter().map(|(m, shape)| (shape.clone(), color_of(blocks, *m))).collect()
}

/// What a layer's contents are called: a material's or part's label, or a blueprint's name.
fn thing_label(blocks: &BlockRegistry, library: &Library, names: &PartNames, thing: Thing) -> String {
    match thing {
        Thing::Material(m) => names.label(blocks, m),
        Thing::Blueprint(id) => library.get(id).map_or_else(|| "a deleted blueprint".to_owned(), |b| b.name.clone()),
    }
}

/// Seconds between one picture's occasional turns.
const SPIN_EVERY: f64 = 8.0;
/// Seconds an occasional turn takes.
const SPIN_TIME: f32 = 1.2;
/// Radians a second a hovered picture turns.
const SPIN_SPEED: f32 = 1.6;

/// How far a picture is turned about the up axis now: once every [`SPIN_EVERY`] seconds it
/// makes a full turn, each picture at its own moment (from `seed`), and while it's hovered it
/// turns all the while, starting from where it stands.
fn spin(ui: &egui::Ui, hovered: bool, seed: u64) -> f32 {
    let ctx = ui.ctx();
    let id = egui::Id::new(("picture turn", seed));
    let now = ui.input(|i| i.time);
    ctx.request_repaint();
    if hovered {
        let start = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
        return (now - start) as f32 * SPIN_SPEED;
    }
    ctx.data_mut(|d| d.remove::<f64>(id));
    let offset = (seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40) as f64 / (1u64 << 24) as f64 * SPIN_EVERY;
    let local = ((now + offset) % SPIN_EVERY) as f32;
    if local >= SPIN_TIME {
        return 0.0;
    }
    let t = local / SPIN_TIME;
    t * t * (3.0 - 2.0 * t) * std::f32::consts::TAU
}

/// Saving the grid's stacks as a blueprint, or over the blueprint they were loaded from.
fn save_buttons(ui: &mut egui::Ui, world: &ClientWorld, menu: &mut CraftingMenu, plan: &Option<Result<sim::CraftPlan, CraftRefusal>>) {
    let (blocks, library) = (world.blocks(), world.blueprints());
    let over = menu.editing.and_then(|id| Some((id, library.get(id)?))).filter(|(_, b)| matches!(b.design, Design::Recipe(_)));
    ui.horizontal_wrapped(|ui| {
        let save = ui.button("Save as blueprint").on_hover_text("keep these stacks as a new blueprint, without crafting");
        if save.clicked() {
            // Named after what it's made of; rename it in the list.
            let name = match plan {
                Some(Ok(p)) => {
                    let names: Vec<&str> = p.pieces.iter().map(|(m, _)| short(blocks.material_name(*m))).collect();
                    format!("{} gadget", names.join(" and "))
                }
                _ => "blueprint".to_owned(),
            };
            menu.blueprint_edit = Some(BlueprintEdit::Save { name, recipe: menu.stacks() });
        }
        if let Some((id, b)) = over {
            let hint = "put these stacks in it; its variants and the blueprints using it change too";
            // A long name is cut, so the button stays on its line.
            let name: String = match b.name.chars().count() > 18 {
                true => b.name.chars().take(17).chain(['…']).collect(),
                false => b.name.clone(),
            };
            if ui.button(format!("Save over {name}")).on_hover_text(format!("{}: {hint}", b.name)).clicked() {
                menu.blueprint_edit = Some(BlueprintEdit::Overwrite { id, recipe: menu.stacks() });
            }
        }
    });
}

/// The blueprints as a tree, and what can be done with the one picked.
fn blueprints(ui: &mut egui::Ui, world: &ClientWorld, menu: &mut CraftingMenu, shift: bool, most: u32) {
    let library = world.blueprints();
    ui.label("Blueprints: press to pick, hold to paint");
    if library.is_empty() {
        ui.label("none yet: save the grid's stacks as one");
    }
    egui::ScrollArea::vertical().id_salt("blueprint tree").auto_shrink([false, true]).max_height(TREE_HEIGHT).show(ui, |ui| {
        for id in library.roots() {
            tree_row(ui, world, menu, id, 0, &mut Vec::new());
        }
    });
    ui.separator();
    match menu.selected.filter(|id| library.get(*id).is_some()) {
        Some(id) => picked(ui, world, menu, id, shift, most),
        None => {
            ui.label("Pick one to make it, change it, or make variants of it.");
        }
    }
    if let Some(Err(why)) = &menu.blueprint_note {
        let text = egui::RichText::new(why.to_string()).color(egui::Color32::from_rgb(230, 120, 100));
        ui.add(egui::Label::new(text).truncate());
    }
}

/// One blueprint's row in the tree, and its branch below it unless folded. `path` holds the
/// rows above, so a branch never repeats into itself.
fn tree_row(ui: &mut egui::Ui, world: &ClientWorld, menu: &mut CraftingMenu, id: BlueprintId, depth: usize, path: &mut Vec<BlueprintId>) {
    let (blocks, library) = (world.blocks(), world.blueprints());
    let Some(b) = library.get(id) else {
        return;
    };
    let children = library.children(id);
    let open = !menu.folded.contains(&id);
    let made = library.resolve(blocks, id);
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        let fold = if children.is_empty() { " " } else if open { "-" } else { "+" };
        let toggle = ui.add_enabled(!children.is_empty(), egui::Button::new(fold).small().min_size(egui::vec2(16.0, 0.0)));
        if toggle.on_hover_text("fold or unfold what it uses and its variants").clicked() && !menu.folded.remove(&id) {
            menu.folded.insert(id);
        }
        let brushed = menu.brush.is_some_and(|p| p.thing == Thing::Blueprint(id));
        let chosen = menu.selected == Some(id);
        // Cut to fit the column, the full name in the tooltip.
        let room = BLUEPRINT_WIDTH - depth as f32 * 14.0 - 16.0 - ICON - 30.0;
        let text = egui::WidgetText::from(b.name.clone());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Truncate), room.max(40.0), egui::TextStyle::Button);
        let size = egui::vec2(ICON + 6.0 + galley.size().x + 8.0, ICON.max(galley.size().y) + 4.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        if response.is_pointer_button_down_on() && ui.ctx().input(|i| i.pointer.primary_down()) {
            menu.selected = Some(id);
            if !brushed {
                menu.brush = Some(Piece::new(id));
            }
            menu.stroke = Some(Stroke::Add(None));
        }
        if response.secondary_clicked() {
            menu.brush = None;
        }
        let visuals = ui.style().interact_selectable(&response, brushed || chosen);
        ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
        let icon = egui::Rect::from_min_size(rect.min + egui::vec2(2.0, 2.0), egui::vec2(ICON, ICON));
        match &made {
            Ok(pieces) => {
                let yaw = spin(ui, response.hovered(), 1 << 32 | id.0 as u64);
                paint_pieces_turned(ui.painter(), icon, &colored(blocks, pieces), yaw);
            }
            Err(_) => paint_missing(ui.painter(), icon),
        }
        let at = egui::pos2(icon.max.x + 6.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, visuals.text_color());
        match &made {
            Ok(_) => response.on_hover_text(format!("{}\npress to pick it and paint with it; right-click to put it down", b.name)),
            Err(why) => response.on_hover_text(format!("{}\nmakes nothing: {why}", b.name)),
        };
    });
    if open && !path.contains(&id) {
        path.push(id);
        for child in children {
            tree_row(ui, world, menu, child, depth + 1, path);
        }
        path.pop();
    }
}

/// The picked blueprint: its picture and what it is, making it, its buttons, and its
/// variants' changes.
fn picked(ui: &mut egui::Ui, world: &ClientWorld, menu: &mut CraftingMenu, id: BlueprintId, shift: bool, most: u32) {
    let (blocks, library) = (world.blocks(), world.blueprints());
    let Some(b) = library.get(id) else {
        return;
    };
    let made = library.resolve(blocks, id);
    let recipe = match &b.design {
        Design::Recipe(r) => Some(*r),
        Design::Variant { .. } => None,
    };
    ui.horizontal_top(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(BLUEPRINT_PREVIEW, BLUEPRINT_PREVIEW), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(25));
        match &made {
            Ok(pieces) => {
                let yaw = spin(ui, response.hovered(), 2 << 32 | id.0 as u64);
                paint_pieces_turned(ui.painter(), rect.shrink(8.0), &colored(blocks, pieces), yaw);
            }
            Err(_) => paint_missing(ui.painter(), rect.shrink(24.0)),
        }
        ui.vertical(|ui| {
            match &mut menu.naming {
                Some((naming, typed)) if *naming == id => {
                    let field = ui.add(egui::TextEdit::singleline(typed).desired_width(150.0).hint_text("a name"));
                    field.request_focus();
                    if ui.small_button("ok").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        menu.blueprint_edit = Some(BlueprintEdit::Rename { id, name: typed.clone() });
                        menu.naming = None;
                    }
                }
                _ => {
                    ui.add(egui::Label::new(egui::RichText::new(&b.name).strong()).truncate());
                }
            }
            match &b.design {
                Design::Recipe(r) => {
                    let view = if r.side { "built on the side" } else { "built on the top" };
                    ui.add(egui::Label::new(if r.flip { format!("{view}, flipped") } else { view.to_owned() }).truncate());
                }
                Design::Variant { base, changes } => {
                    let base = library.get(*base).map_or("a deleted blueprint", |b| b.name.as_str());
                    ui.add(egui::Label::new(format!("variant of {base}")).truncate());
                    let changes: Vec<String> = changes.iter().map(|c| c.describe(blocks)).collect();
                    ui.add(egui::Label::new(if changes.is_empty() { "no changes".to_owned() } else { changes.join(", ") }).truncate());
                }
            }
            if let Err(why) = &made {
                let text = egui::RichText::new(format!("makes nothing: {why}")).color(egui::Color32::from_rgb(230, 120, 100));
                ui.add(egui::Label::new(text).truncate());
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        match menu.make(id, most).map(|craft| world.craft_plan(&craft)) {
            Some(Ok(plan)) => {
                let one = ui.button("Make").on_hover_text("shift-click makes as many at once; free, like crafting");
                let all = ui.button(format!("Make {}", plan.count));
                let count = if all.clicked() || (one.clicked() && shift) {
                    Some(plan.count)
                } else {
                    one.clicked().then_some(1)
                };
                if let Some(count) = count {
                    menu.pending = menu.make(id, count);
                }
            }
            Some(Err(why)) if made.is_ok() => {
                ui.add(egui::Label::new(format!("can't make it: {why}")).truncate());
            }
            _ => {}
        }
        if let Some(recipe) = recipe
            && ui.button("Load").on_hover_text("put its stacks in the grid, to change them and save over it").clicked()
        {
            menu.load(recipe);
            menu.editing = Some(id);
        }
        if ui.button("Rename").clicked() {
            menu.naming = Some((id, b.name.clone()));
        }
        if ui.button("Copy").on_hover_text("a copy of it; a variant's copy is linked to the same base").clicked() {
            menu.blueprint_edit = Some(BlueprintEdit::Copy { id });
        }
        if ui.button("Delete").on_hover_text("only once no other blueprint uses it").clicked() {
            menu.blueprint_edit = Some(BlueprintEdit::Delete { id });
        }
    });
    ui.add_space(4.0);
    ui.label(match recipe {
        Some(_) => "Changes make a new variant, linked: changing this one later changes it too.",
        None => "Changes change this variant.",
    });
    let mut vary = None;
    let mut row = |ui: &mut egui::Ui, title: &str, buttons: &[(&str, Variation)]| {
        ui.horizontal_wrapped(|ui| {
            ui.label(title);
            for (text, variation) in buttons {
                if ui.small_button(*text).clicked() {
                    vary = Some(*variation);
                }
            }
        });
    };
    let turn = |axis, quarters| Variation::Change(Change::Turn { axis, quarters });
    let mirror = |axis| Variation::Change(Change::Mirror { axis });
    let slide = |by| Variation::Change(Change::Shift(by));
    let center = |axis| Variation::Center { axis };
    row(ui, "Turn", &[("left", turn(2, 1)), ("right", turn(2, 3)), ("tip toward", turn(0, 1)), ("tip away", turn(0, 3)), ("roll left", turn(1, 3)), ("roll right", turn(1, 1))]);
    row(ui, "Mirror", &[("left-right", mirror(0)), ("front-back", mirror(1)), ("top-bottom", mirror(2))]);
    const Q: i8 = api::crafting::GRID as i8;
    row(ui, "Shift a quarter", &[
        ("left", slide([-Q, 0, 0])),
        ("right", slide([Q, 0, 0])),
        ("away", slide([0, Q, 0])),
        ("toward", slide([0, -Q, 0])),
        ("up", slide([0, 0, Q])),
        ("down", slide([0, 0, -Q])),
    ]);
    row(ui, "Center", &[("left-right", center(0)), ("front-back", center(1)), ("up-down", center(2))]);
    if let Some(variation) = vary {
        menu.blueprint_edit = Some(BlueprintEdit::Vary { id, variation });
    }
    // Swapping one material for another: from those it's made of, to any shapeable material.
    if let Ok(pieces) = &made {
        let from_list: Vec<MaterialId> = pieces.iter().map(|(m, _)| *m).collect();
        let to_list = shapeable(blocks);
        let (from, to) = &mut menu.swap;
        if !from.is_some_and(|f| from_list.contains(&f)) {
            *from = from_list.first().copied();
        }
        if to.is_none() {
            *to = to_list.iter().copied().find(|m| Some(*m) != *from);
        }
        let mut swap = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Swap");
            let name = |m: Option<MaterialId>| m.map_or("-", |m| short(blocks.material_name(m))).to_owned();
            egui::ComboBox::from_id_salt("swap from").selected_text(name(*from)).show_ui(ui, |ui| {
                for m in &from_list {
                    ui.selectable_value(from, Some(*m), short(blocks.material_name(*m)));
                }
            });
            ui.label("for");
            egui::ComboBox::from_id_salt("swap to").selected_text(name(*to)).show_ui(ui, |ui| {
                for m in &to_list {
                    ui.selectable_value(to, Some(*m), short(blocks.material_name(*m)));
                }
            });
            if let (Some(f), Some(t)) = (*from, *to)
                && ui.add_enabled(f != t, egui::Button::new("swap").small()).clicked()
            {
                swap = Some(Change::Swap { from: f, to: t });
            }
        });
        if let Some(change) = swap {
            menu.blueprint_edit = Some(BlueprintEdit::Vary { id, variation: Variation::Change(change) });
        }
    }
    if recipe.is_none() && ui.button("Undo last change").clicked() {
        menu.blueprint_edit = Some(BlueprintEdit::Undo { id });
    }
}

/// Every material that takes shapes: those with a full block, not parts or bundles.
fn shapeable(blocks: &BlockRegistry) -> Vec<MaterialId> {
    (0..blocks.material_count())
        .filter_map(|i| blocks.material_by_index(i))
        .filter(|&m| blocks.part_of(m).is_none() && blocks.pieces(m).is_empty())
        .filter(|&m| blocks.get(blocks.material_block(m)).is_some_and(|d| d.is_full()))
        .collect()
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

/// Where the pictures look from, unturned: in front of the shape (-Y, where someone at the
/// table's front stands), a little to the left and above, so the front, the left side and the
/// top show.
const VIEW: [f32; 3] = [0.6, 1.0, -0.8];

/// The way a picture looks, turned `yaw` radians about the up axis: turning the view one way
/// turns the shape the other.
fn view_at(yaw: f32) -> ZVec3 {
    let (sin, cos) = yaw.sin_cos();
    let [x, y, z] = VIEW;
    ZVec3::new(x * cos - y * sin, x * sin + y * cos, z).normalize()
}

/// Each side of a texel, and how much light it gets in a picture: the top the most, the
/// bottom the least.
const FACES: [([i32; 3], f32); 6] =
    [([-1, 0, 0], 0.62), ([1, 0, 0], 0.7), ([0, -1, 0], 0.8), ([0, 1, 0], 0.72), ([0, 0, 1], 1.0), ([0, 0, -1], 0.5)];

/// The corners of a unit cube's side facing `normal`.
fn face_corners(normal: [i32; 3]) -> [ZVec3; 4] {
    let axis = normal.iter().position(|&n| n != 0).expect("a side's normal");
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let plane = if normal[axis] > 0 { 1.0 } else { 0.0 };
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(a, b)| {
        let mut c = [0.0; 3];
        (c[axis], c[u], c[v]) = (plane, a, b);
        ZVec3::from_array(c)
    })
}

/// One face of one texel, as the picture draws it.
struct Quad {
    depth: f32,
    corners: [ZVec3; 4],
    shade: f32,
    color: [u8; 3],
    /// How squarely it faces the viewer, 0 (edge-on) to 1.
    facing: f32,
}

/// Faces turned less than this toward the viewer aren't drawn: edge-on, they'd be slivers.
const EDGE_ON: f32 = 0.08;
/// Faces turned less than this toward the viewer get no hairline: on a thin sliver it spikes
/// out past its corners.
const STEEP: f32 = 0.3;

/// The visible faces of `shape` from [`VIEW`], farthest first, in texels.
#[cfg(test)]
fn quads(shape: &ShapeGrid) -> Vec<Quad> {
    quads_of(&[(shape.clone(), [0; 3])], 0.0)
}

/// The visible faces of the pieces seen from [`view_at`] `yaw`, each in its piece's color,
/// farthest first, in texels. Where pieces overlap, the first one's color shows.
fn quads_of(pieces: &[(ShapeGrid, [u8; 3])], yaw: f32) -> Vec<Quad> {
    let color_at = |x: i32, y: i32, z: i32| {
        let inside = (0..16).contains(&x) && (0..16).contains(&y) && (0..16).contains(&z);
        if !inside {
            return None;
        }
        pieces.iter().find(|(s, _)| s.get(x as usize, y as usize, z as usize)).map(|(_, c)| *c)
    };
    let forward = view_at(yaw);
    // The sides that turn toward the viewer, and how squarely.
    let faces: Vec<([i32; 3], [ZVec3; 4], f32, f32)> = FACES
        .iter()
        .map(|&(n, shade)| (n, shade, -ZVec3::new(n[0] as f32, n[1] as f32, n[2] as f32).dot(forward)))
        .filter(|&(_, _, facing)| facing > EDGE_ON)
        .map(|(n, shade, facing)| (n, face_corners(n), shade, facing))
        .collect();
    let filled = |x: i32, y: i32, z: i32| color_at(x, y, z).is_some();
    let mut out = Vec::new();
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let Some(color) = color_at(x, y, z) else {
                    continue;
                };
                for (n, corners, shade, facing) in &faces {
                    if filled(x + n[0], y + n[1], z + n[2]) {
                        continue;
                    }
                    let at = ZVec3::new(x as f32, y as f32, z as f32);
                    let corners = corners.map(|c| at + c);
                    let middle = (corners[0] + corners[2]) * 0.5;
                    out.push(Quad { depth: middle.dot(forward), corners, shade: *shade, color, facing: *facing });
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
    paint_pieces_turned(painter, rect, pieces, 0.0);
}

/// [`paint_pieces`], the view turned `yaw` radians about the up axis. The scale stays the same
/// at every turn, so a turning picture doesn't grow and shrink.
fn paint_pieces_turned(painter: &egui::Painter, rect: egui::Rect, pieces: &[(ShapeGrid, [u8; 3])], yaw: f32) {
    let axes = |yaw: f32| {
        let forward = view_at(yaw);
        let right = forward.cross(ZVec3::Z).normalize();
        (right, right.cross(forward))
    };
    let flat = |(right, up): (ZVec3, ZVec3), p: ZVec3| {
        let c = p - ZVec3::splat(8.0);
        egui::vec2(c.dot(right), -c.dot(up))
    };
    // Scale so the whole boxel's outline fits, however it's turned.
    let mut extent = egui::Vec2::ZERO;
    for step in 0..8 {
        let at = axes(step as f32 * std::f32::consts::FRAC_PI_4);
        for i in 0..8 {
            let corner = ZVec3::new((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32) * 16.0;
            extent = extent.max(flat(at, corner).abs());
        }
    }
    let scale = (rect.width() / (2.0 * extent.x)).min(rect.height() / (2.0 * extent.y));
    let center = rect.center();
    let at = axes(yaw);
    for quad in quads_of(pieces, yaw) {
        let color = quad.color;
        let shade = |c: u8| (c as f32 * quad.shade) as u8;
        let fill = egui::Color32::from_rgb(shade(color[0]), shade(color[1]), shade(color[2]));
        let points = quad.corners.iter().map(|&p| center + flat(at, p) * scale).collect();
        // A hairline of the same color closes the seams between neighboring faces.
        let hairline: f32 = if quad.facing > STEEP { 0.6 } else { 0.0 };
        painter.add(egui::Shape::convex_polygon(points, fill, egui::Stroke::new(hairline, fill)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Link;
    use api::BlockBox;

    #[test]
    fn pictures_draw_only_the_faces_that_show() {
        // A full block shows one big face on each of three sides, in texels.
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
        let [Some(grid), Some(list), Some(result), Some(blueprints)] = menu.columns else { panic!("drawn") };
        let bottom = grid.max.y.max(result.max.y).max(blueprints.max.y);
        assert!((list.max.y - bottom).abs() < 2.0, "the list ends with the tallest column: {list:?} {grid:?} {result:?} {blueprints:?}");
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
        assert_eq!(side.source, Source::Grid(Recipe { cells: Cells::default(), side: true, flip: false }));
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
        menu.crafted(world.blocks(), &Library::default(), &mut PartNames::default(), result);
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
        assert!(stairs.cells.iter().flatten().flatten().flatten().all(|p| p.thing == Thing::Material(stone)));
        // A post is finer than quarters.
        let wood = blocks.material(base::WOOD).unwrap();
        let post = blocks.drop(blocks.find_shaped(wood, &base::shapes::post()).unwrap()).unwrap().material;
        assert_eq!(rebuild(blocks, post), None);
        // A door as a bundle, a panel against the far side with a knob out from it: the knob
        // floats on the top, so it's rebuilt on the side.
        let mut blocks = world.blocks().clone();
        let (phasite, brass) = (blocks.material(base::PHASITE).unwrap(), blocks.material(base::BRASS).unwrap());
        let cube = |min, max| ShapeGrid::from_boxes(&[api::BlockBox::new(min, max)]);
        let pieces = [(phasite, cube([0, 12, 0], [16, 16, 16])), (brass, cube([12, 8, 8], [16, 12, 12]))];
        let door = blocks.bundle(&pieces).unwrap();
        let recipe = rebuild(&blocks, door).unwrap();
        assert!(recipe.side && !recipe.flip);
        assert_eq!(recipe.cells[1][3][1], Some(Piece::new(brass)), "the knob, one layer out");
    }

    #[test]
    fn the_request_needs_an_open_table() {
        let mut menu = CraftingMenu::default();
        assert_eq!(menu.request(1), None);
        menu.open = Some(BoxelPos::new(1, 2, 3));
        menu.flips[0] = true;
        let craft = menu.request(5).unwrap();
        let Source::Grid(recipe) = craft.source else { panic!("from the grid") };
        assert_eq!((craft.table, recipe.flip, craft.count), (BoxelPos::new(1, 2, 3), true, 5));
    }

    #[test]
    fn pictures_turned_half_way_show_the_back() {
        let shows_back = |yaw: f32| quads_of(&[(ShapeGrid::FULL, [0; 3])], yaw).iter().any(|q| q.corners.iter().all(|c| c.y == 16.0));
        assert!(!shows_back(0.0));
        assert!(shows_back(std::f32::consts::PI));
        assert_eq!(quads_of(&[(ShapeGrid::FULL, [0; 3])], 1.0).len(), 3 * 16 * 16, "three sides at any turn");
    }

    /// Sends what the menu has waiting with one tick, and hands it the outcomes.
    fn send(link: &mut Link, world: &mut ClientWorld, menu: &mut CraftingMenu, names: &mut PartNames) {
        use crate::world::testing::step;
        let input = sim::PlayerInput { craft: menu.take_pending(), blueprint: menu.take_blueprint_edit(), ..Default::default() };
        step(link, world, &input);
        if let Some(result) = world.take_craft() {
            menu.crafted(world.blocks(), world.blueprints(), names, result);
        }
        if let Some(result) = world.take_blueprint_note() {
            menu.blueprint_edited(result);
        }
    }

    #[test]
    fn saved_stacks_become_the_picked_blueprint_and_make_parts_named_after_it() {
        let (mut link, mut world, mut menu) = at_table();
        let mut names = PartNames::default();
        let brick = world.blocks().material(base::BRICK).unwrap();
        menu.cells_mut()[3][0][0] = Some(Piece::new(brick));
        let empty = draw(&egui::Context::default(), &world, &mut menu);
        menu.blueprint_edit = Some(BlueprintEdit::Save { name: "nub".into(), recipe: menu.stacks() });
        send(&mut link, &mut world, &mut menu, &mut names);
        let id = world.blueprints().find("nub").expect("saved");
        assert_eq!((menu.selected, menu.editing), (Some(id), Some(id)), "picked, and the grid holds its stacks");
        // Picked, its buttons show, and the window keeps its place and width.
        let picked = draw(&egui::Context::default(), &world, &mut menu);
        assert_eq!((picked.min.x, picked.width()), (empty.min.x, empty.width()));
        menu.pending = menu.make(id, 2);
        send(&mut link, &mut world, &mut menu, &mut names);
        let Some(Ok((part, 2))) = menu.result else { panic!("{:?}", menu.result) };
        assert_eq!(names.label(world.blocks(), part), "nub");
        // A variant is picked once made; deleting the base is refused while it's used.
        menu.blueprint_edit = Some(BlueprintEdit::Vary { id, variation: Variation::Change(Change::Mirror { axis: 0 }) });
        send(&mut link, &mut world, &mut menu, &mut names);
        let variant = menu.selected.unwrap();
        assert_ne!(variant, id);
        draw(&egui::Context::default(), &world, &mut menu);
        menu.blueprint_edit = Some(BlueprintEdit::Delete { id });
        send(&mut link, &mut world, &mut menu, &mut names);
        assert!(matches!(&menu.blueprint_note, Some(Err(BlueprintRefusal::InUse(by))) if by == "nub mirrored X"));
    }

    #[test]
    fn long_names_turned_brushes_and_long_changes_never_move_the_grid() {
        let (mut link, mut world, mut menu) = at_table();
        let ctx = egui::Context::default();
        let (mut palette, mut names) = (Palette::default(), PartNames::default());
        let mut frame = |world: &ClientWorld, menu: &mut CraftingMenu, names: &mut PartNames| {
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                ..Default::default()
            };
            for _ in 0..4 {
                let _ = ctx.run(input(), |ctx| show_menu(ctx, world, &Controls::default(), menu, &mut palette, names));
            }
            let window = ctx.memory(|m| m.area_rect(egui::Id::new("Crafting"))).unwrap();
            (window.min.x, window.width(), menu.columns[0].unwrap().min)
        };
        let start = frame(&world, &mut menu, &mut names);
        // A part with a very long name, as the brush, turned.
        let brick = world.blocks().material(base::BRICK).unwrap();
        menu.cells_mut()[3][0][0] = Some(Piece::new(brick));
        menu.pending = menu.request(1);
        send(&mut link, &mut world, &mut menu, &mut names);
        let Some(Ok((part, _))) = menu.result else { panic!("crafted") };
        names.set(world.blocks(), part, &"a very long name for a little brick nub ".repeat(4));
        menu.brush = Some(Piece { thing: part.into(), turns: 3 });
        assert_eq!(frame(&world, &mut menu, &mut names), start, "a long, turned brush");
        // A picked variant with a long name and a long list of changes, and a refusal.
        let long = "an insulated corner for the left door, upper half ".repeat(3);
        menu.blueprint_edit = Some(BlueprintEdit::Save { name: long, recipe: menu.stacks() });
        send(&mut link, &mut world, &mut menu, &mut names);
        let base = menu.selected.unwrap();
        menu.blueprint_edit = Some(BlueprintEdit::Vary { id: base, variation: Variation::Change(Change::Mirror { axis: 0 }) });
        send(&mut link, &mut world, &mut menu, &mut names);
        let variant = menu.selected.unwrap();
        for axis in 0..3 {
            for change in [Change::Turn { axis, quarters: 1 }, Change::Mirror { axis }] {
                menu.blueprint_edit = Some(BlueprintEdit::Vary { id: variant, variation: Variation::Change(change) });
                send(&mut link, &mut world, &mut menu, &mut names);
            }
        }
        menu.blueprint_edit = Some(BlueprintEdit::Delete { id: base });
        send(&mut link, &mut world, &mut menu, &mut names);
        assert!(matches!(menu.blueprint_note, Some(Err(_))), "a refusal to show");
        menu.renaming = Some((part, "typing".into()));
        assert_eq!(frame(&world, &mut menu, &mut names), start, "a long variant, a refusal and a rename");
    }
}
