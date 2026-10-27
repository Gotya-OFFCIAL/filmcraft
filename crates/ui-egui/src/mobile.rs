//! Touch presentation: the phone chrome (docs/mobile-ui.md).
//!
//! Same engine, same command ids, same undo as the desktop - only the chrome changes.
//! Two orthogonal switches come out of [`detect`] each frame:
//!
//! - **touch chrome** (any Android window): the header as workspace chips, and the
//!   single-column Import / Export pages. Landscape keeps the desktop docking body
//!   under this chrome - measured on device, a restacked landscape scored worse than
//!   the plain three-pane layout, which is already finger-sized at phone pixel
//!   density.
//! - **portrait stack** (a phone held upright): monitor over timeline over a tool
//!   dock, with the 44 dp metrics the stack has room for.
//!
//! `FILMCRAFT_PRESENTATION` forces one for development on any platform.

use egui::{Align2, Color32, FontId, Id, Key, PointerButton, Pos2, Rect, Response, ScrollArea, Sense, Ui, Vec2};

use crate::state::Presentation;
use crate::theme::Tokens;

/// Below this shortest window edge (logical points) a touch device held upright is a
/// phone: the portrait chrome stacks the monitor over the timeline and a tool dock.
pub(crate) const PHONE_SHORT_EDGE: f32 = 720.0;

/// The smallest touch target, in dp (Material guidance; docs/mobile-ui.md principle
/// 2). Every interactive element's hit box grows to at least this.
pub(crate) const MIN_TOUCH_DP: f32 = 44.0;

/// How long a finger must hold before a press becomes a long press (the right-click
/// stand-in), and how far it may wander and still count.
// LongPress and Ghost arrive with their P2/P4 callers (clip context menus, two-stage
// trim drags); they live here first so the shells and the widgets share one definition.
#[allow(dead_code)]
const LONG_PRESS_SECS: f64 = 0.5;
const LONG_PRESS_SLOP: f32 = 12.0;

/// Portrait when the window is taller than wide, at phone sizes.
pub(crate) fn is_portrait(content: Vec2) -> bool {
    content.y > content.x
}

/// Decide the presentation for this frame. `android` is a const the caller passes
/// (`cfg!(target_os = "android")`): the touch chrome only fires there, so a desktop
/// touchscreen laptop never gets restacked by an accidental palm. An Android window is
/// touch from its first frame - there is no other pointer to wait for.
pub(crate) fn detect(android: bool, content: Vec2) -> Presentation {
    // Development escape hatch: force a presentation on any platform.
    if let Some(forced) = forced() {
        return forced;
    }
    if !android {
        return Presentation::Desktop;
    }
    // `<=`: a 2K tablet at 320 dpi is exactly 720 dp on its short edge, and held
    // upright that is the phone the portrait stack is for.
    if content.x.min(content.y) <= PHONE_SHORT_EDGE && is_portrait(content) { Presentation::MobilePortrait } else { Presentation::MobileLandscape }
}

/// `FILMCRAFT_PRESENTATION=desktop|landscape|portrait`, for driving the mobile shells
/// from a desktop window or a unit test.
fn forced() -> Option<Presentation> {
    match std::env::var("FILMCRAFT_PRESENTATION").ok().as_deref() {
        Some("desktop") => Some(Presentation::Desktop),
        Some("landscape") => Some(Presentation::MobileLandscape),
        Some("portrait") => Some(Presentation::MobilePortrait),
        _ => None,
    }
}

/// Grow the interaction metrics to touch size for the portrait stack (docs/mobile-ui.md
/// principle 2): the restacked panes have the room, and their widgets are all the user
/// reaches for. Landscape keeps the desktop metrics - the docking layout is built
/// around them, and its own hit targets are already finger-sized at phone density.
/// Leaving the portrait chrome restores the theme's metrics (theme.rs sets them with
/// the visuals, which a presentation change does not rerun), so the switch is applied
/// once per (presentation, theme) pair.
pub(crate) fn apply_style(ctx: &egui::Context, t: &Tokens, applied: &mut Option<(Presentation, crate::theme::ThemeKind)>, p: Presentation) {
    if applied.is_some_and(|(prev, kind)| prev == p && kind == t.kind) {
        return;
    }
    *applied = Some((p, t.kind));
    if p != Presentation::MobilePortrait {
        crate::theme::apply_visuals(ctx, t);
        return;
    }
    ctx.global_style_mut(|style| {
        style.spacing.interact_size = Vec2::splat(MIN_TOUCH_DP);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 10.0);
        style.spacing.slider_width = style.spacing.slider_width.max(220.0);
    });
}

/// The long-press gesture for one rect, ticked every frame by whoever owns the widget
/// (docs/mobile-ui.md principle 3: long press replaces right-click).
///
/// [`LongPress::fire`] returns `Some(pos)` exactly once, the frame a press that began
/// inside the rect (minus [`LONG_PRESS_SLOP`]) crosses [`LONG_PRESS_SECS`] without
/// lifting. The owner then opens its context surface. [`LongPress::swallow`] answers
/// the next question: true from the fire until the pointer lifts, so the owner can
/// ignore the click the release would otherwise deliver.
#[allow(dead_code)]
pub(crate) struct LongPress;

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct LpState {
    anchor: Pos2,
    started: f64,
    fired: bool,
}

#[allow(dead_code)]
impl LongPress {
    pub(crate) fn fire(ctx: &egui::Context, id: Id, rect: Rect) -> Option<Pos2> {
        let now = ctx.input(|i| i.time);
        if let Some(anchor) = ctx.input(|i| i.pointer.press_origin().filter(|p| i.pointer.any_pressed() && rect.contains(*p))) {
            ctx.data_mut(|d| d.insert_temp(id, LpState { anchor, started: now, fired: false }));
            return None;
        }
        let Some(state) = ctx.data(|d| d.get_temp::<LpState>(id)) else {
            return None;
        };
        let (down, pos) = ctx.input(|i| (i.pointer.any_down(), i.pointer.interact_pos()));
        if !down {
            ctx.data_mut(|d| d.remove::<LpState>(id));
            return None;
        }
        if state.fired {
            return None;
        }
        let still_inside = pos.is_some_and(|p| (p - state.anchor).length() <= LONG_PRESS_SLOP);
        if still_inside && now - state.started >= LONG_PRESS_SECS {
            ctx.data_mut(|d| d.insert_temp(id, LpState { fired: true, ..state }));
            return pos;
        }
        None
    }

    pub(crate) fn swallow(ctx: &egui::Context, id: Id) -> bool {
        ctx.data(|d| d.get_temp::<LpState>(id)).is_some_and(|s| s.fired)
    }
}

/// A modal bottom sheet (the CapCut / Premiere Rush pattern the mobile panels open
/// through): full-width, pinned to the bottom, capped at 70 % of the screen height,
/// scrollable, closed by tapping the scrim, the close button, or Escape. `open` is
/// flipped to false on any dismissal; the body's value comes back only while open.
pub(crate) fn bottom_sheet<R>(ctx: &egui::Context, t: &Tokens, id: Id, open: &mut bool, heading: &str, body: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    if !*open {
        return None;
    }
    let screen = ctx.content_rect();
    // The scrim: a full-screen click target under the sheet.
    let scrim = egui::Area::new(id.with("scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(140));
            resp
        })
        .inner;
    let sheet =
        egui::Area::new(id.with("sheet")).order(egui::Order::Middle).anchor(Align2::LEFT_BOTTOM, Vec2::ZERO).default_width(screen.width()).show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.panel_bg)
                .corner_radius(12.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(heading).strong().size(16.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(egui::RichText::new("\u{00d7}").size(18.0)).clicked() {
                                *open = false;
                            }
                        });
                    });
                    ui.separator();
                    ScrollArea::vertical().id_salt(id.with("scroll")).max_height(screen.height() * 0.7).auto_shrink([false, false]).show(ui, body).inner
                })
                .inner
        });
    // A primary press that began outside the sheet closes it (the scrim already
    // covers the empty screen; this also catches presses on other panels' layers).
    let pressed_outside = ctx.input(|i| {
        i.pointer.button_pressed(PointerButton::Primary) && i.pointer.press_origin().is_some_and(|p| !sheet.response.rect.expand(LONG_PRESS_SLOP).contains(p))
    });
    if scrim.clicked() || pressed_outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
        *open = false;
        return None;
    }
    Some(sheet.inner)
}

/// Two-stage ghost drag (docs/mobile-ui.md principle 4): a finger drag previews a
/// ghost value while it moves and commits once on lift, so the thing under the finger
/// is never destroyed blind. Owners keep the start value in their own state:
///
/// ```text
/// match Ghost::frame(ctx, id, rect, |dx| start + dx / pps) {
///     Ghost::Idle => {}
///     Ghost::Dragging { value, .. } => draw the preview at `value`,
///     Ghost::Commit(value) => apply it once (the same command the desktop drag fires),
/// }
/// ```
#[allow(dead_code)]
pub(crate) enum Ghost {
    Idle,
    /// The finger is down and moving; `grab` is where it took hold.
    Dragging {
        value: f32,
        grab: Pos2,
    },
    /// The frame the finger lifted; apply the value now.
    Commit(f32),
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct GhostDrag {
    grab: Pos2,
}

#[allow(dead_code)]
impl Ghost {
    /// Track a horizontal drag of one value (a trim, a move, a level). `map` turns the
    /// pixel offset from the grab point into the new value.
    pub(crate) fn frame(ctx: &egui::Context, id: Id, rect: Rect, map: impl Fn(f32) -> f32) -> Ghost {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos().filter(|p| i.pointer.any_pressed() && rect.contains(*p))) {
            ctx.data_mut(|d| d.insert_temp(id, GhostDrag { grab: pos }));
            return Ghost::Idle;
        }
        let Some(drag) = ctx.data(|d| d.get_temp::<GhostDrag>(id)) else {
            return Ghost::Idle;
        };
        let (down, pos) = ctx.input(|i| (i.pointer.any_down(), i.pointer.interact_pos()));
        match pos {
            Some(pos) if down => Ghost::Dragging { value: map(pos.x - drag.grab.x), grab: drag.grab },
            Some(pos) => {
                ctx.data_mut(|d| d.remove::<GhostDrag>(id));
                Ghost::Commit(map(pos.x - drag.grab.x))
            }
            None => {
                ctx.data_mut(|d| d.remove::<GhostDrag>(id));
                Ghost::Idle
            }
        }
    }
}

/// A big touch row: label left, trailing control right, at least [`MIN_TOUCH_DP`]
/// tall. The mobile forms (Export, Import, settings sheets) are built from these
/// instead of the desktop's dense row/Grid layouts.
#[allow(dead_code)] // the P3 forms are the first callers
pub(crate) fn row(ui: &mut Ui, t: &Tokens, label: &str, body: impl FnOnce(&mut Ui)) -> Response {
    let width = ui.available_width();
    let trailing = (width * 0.45).clamp(140.0, 280.0);
    let desired = Vec2::new(width, MIN_TOUCH_DP.max(ui.spacing().interact_size.y));
    let (rect, resp) = ui.allocate_at_least(desired, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().text(Pos2::new(rect.min.x + 4.0, rect.center().y), Align2::LEFT_CENTER, label, Tokens::ui(14.0), t.text);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(Pos2::new(rect.max.x - trailing, rect.min.y), Vec2::new(trailing, rect.height())))
                .id_salt(("mobile-row", label)),
        );
        child.vertical_centered_justified(body);
    }
    resp
}

/// The touch header: the three workspaces as big chips (Import / Edit / Export, the
/// same [`crate::state::Mode`] the desktop header switches), the project name, and an
/// overflow button opening a bottom sheet. Taller than the desktop header so the chips
/// clear [`MIN_TOUCH_DP`].
pub(crate) fn top_bar(app: &mut crate::FilmcraftApp, ui: &mut Ui, rect: Rect) {
    let t = app.tokens;
    ui.painter().rect_filled(rect, 0.0, t.header_bg);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(6.0, 4.0))).id_salt("mobile-header"));
    child.horizontal_centered(|ui| {
        for (m, label) in [(crate::state::Mode::Import, tl!("Import")), (crate::state::Mode::Edit, tl!("Edit")), (crate::state::Mode::Export, tl!("Export"))] {
            if ui.selectable_label(app.ui.mode == m, egui::RichText::new(label).size(14.0)).clicked() {
                app.ui.mode = m;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let sheet_id = ui.id().with("more-sheet");
            let mut open = ui.ctx().data(|d| d.get_temp::<bool>(sheet_id)).unwrap_or(false);
            if ui.button(egui::RichText::new("\u{22ef}").size(18.0)).clicked() {
                open = !open;
            }
            let items = crate::menus::menu_items(app);
            let can = |id: &str| items.iter().find(|it| it.id == id).is_some_and(|it| it.enabled);
            let mut picked: Option<(&'static str, serde_json::Value)> = None;
            let _ = bottom_sheet(ui.ctx(), &t, sheet_id, &mut open, tl!("Menu"), |ui| {
                // The file work the desktop menu bar owned - the touch header
                // replaced that bar, so those commands live here now.
                for (icon, label, id, params) in menu_commands() {
                    if command_row(ui, &t, &can, icon, label, id) {
                        picked = Some((id, params));
                    }
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(sheet_id, open));
            if let Some((id, params)) = picked {
                let _ = crate::menus::invoke(app, ui.ctx(), id, params);
            }
            let name = app.session.project.name.clone();
            ui.label(egui::RichText::new(name).color(t.text_dim).size(13.0));
        });
    });
}

/// One command row of a touch sheet: icon, translated label, greyed when the engine
/// would refuse the command (the same enabled table the desktop menus read).
fn command_row(ui: &mut Ui, t: &Tokens, can: &dyn Fn(&str) -> bool, icon: crate::icons::Icon, label: &str, id: &'static str) -> bool {
    let row = ui.add_enabled(
        can(id),
        egui::Button::new(egui::RichText::new(format!("   {label}")).size(14.0))
            .fill(Color32::TRANSPARENT)
            .min_size(Vec2::new(ui.available_width(), MIN_TOUCH_DP)),
    );
    crate::icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(row.rect.min.x + 22.0, row.rect.center().y), Vec2::splat(18.0)), icon, t.text);
    row.clicked()
}

/// The commands the ⋯ sheet lists: the file work the desktop menu bar owned (the
/// touch header replaced that bar, so they need a home), the settings entry, and
/// the edits the portrait dock has no slot for. Shared by both sheets so the two
/// never drift.
fn menu_commands() -> Vec<(crate::icons::Icon, &'static str, &'static str, serde_json::Value)> {
    let j = serde_json::json!({});
    vec![
        (crate::icons::Icon::Drive, tl!("Save"), "file.save", j.clone()),
        (crate::icons::Icon::NewItem, tl!("Save As\u{2026}"), "file.saveAs", j.clone()),
        (crate::icons::Icon::Folder, tl!("Import\u{2026}"), "file.import", j.clone()),
        (crate::icons::Icon::Sequence, tl!("New Sequence"), "file.newSequence", j.clone()),
        (crate::icons::Icon::Lift, tl!("Lift"), "sequence.lift", j.clone()),
        (crate::icons::Icon::Extract, tl!("Extract"), "sequence.extract", j.clone()),
        (crate::icons::Icon::MarkIn, tl!("Mark Clip"), "markers.markClip", j.clone()),
        (crate::icons::Icon::MarkOut, tl!("Mark Selection"), "markers.markSelection", j.clone()),
        (crate::icons::Icon::Marker, tl!("Go to In"), "markers.goToIn", serde_json::json!({"target": "program"})),
        (crate::icons::Icon::Link, tl!("Linked Selection"), "sequence.linkedSelection", j.clone()),
        (crate::icons::Icon::Sequence, tl!("Copy"), "edit.copy", j.clone()),
        (crate::icons::Icon::Plus, tl!("Paste"), "edit.paste", j.clone()),
        (crate::icons::Icon::Gear, tl!("Preferences\u{2026}"), "app.settings.general", j),
    ]
}

/// The portrait Edit chrome: the Program monitor over the Timeline over the tool
/// dock (docs/mobile-ui.md), with a draggable divider remembering the monitor's
/// share. The dock's commands run the same ids the desktop menus fire.
pub(crate) fn edit(app: &mut crate::FilmcraftApp, ui: &mut Ui, body: Rect) {
    let id = ui.id().with("mobile-edit-split");
    let mut share = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(0.42);
    let handle_h = MIN_TOUCH_DP * 0.6;
    let dock_h = 64.0;
    let stack = Rect::from_min_max(body.min, Pos2::new(body.max.x, body.max.y - dock_h));
    let dock = Rect::from_min_max(Pos2::new(body.min.x, stack.max.y), body.max);
    let monitor_h = ((stack.height() - handle_h) * share).max(120.0);
    let monitor = Rect::from_min_size(stack.min, Vec2::new(stack.width(), monitor_h));
    let handle = Rect::from_min_size(Pos2::new(stack.min.x, monitor.max.y), Vec2::new(stack.width(), handle_h));
    let timeline = Rect::from_min_max(Pos2::new(stack.min.x, handle.max.y), stack.max);

    let t = app.tokens;
    ui.painter().rect_filled(handle, 0.0, t.header_bg);
    let resp = ui.interact(handle, id.with("handle"), Sense::drag());
    if resp.dragged() {
        let dy = ui.ctx().input(|i| i.pointer.delta()).y;
        share = ((monitor.max.y + dy - stack.min.y) / (stack.height() - handle_h)).clamp(0.25, 0.75);
        ui.ctx().data_mut(|d| d.insert_temp(id, share));
    }
    // The grab bar in the middle of the handle, so it reads as draggable.
    let grip = Rect::from_center_size(handle.center(), Vec2::new(44.0, 4.0));
    ui.painter().rect_filled(grip, 2.0, t.text_faint);

    for (panel, rect) in [(crate::dock::PanelKind::Program, monitor), (crate::dock::PanelKind::Timeline, timeline)] {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("mobile-panel", panel.id())));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        crate::panels::show(app, &mut child, panel, rect);
    }
    tool_dock(app, ui, dock);
}

/// The CapCut-style bottom dock: the edits a thumb makes one-handed (undo, redo,
/// split at the playhead, ripple delete), the two toggles worth pinning (snap, zoom
/// to fit), and an overflow button opening the rest as a bottom sheet. Every target
/// clears [`MIN_TOUCH_DP`]; a command the engine would refuse greys itself out from
/// the same enabled state the desktop menus read.
fn tool_dock(app: &mut crate::FilmcraftApp, ui: &mut Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    ui.painter().rect_filled(rect, 0.0, t.header_bg);
    // The enabled table is the one the desktop menus build from; take it before
    // anything else borrows the app.
    let items = crate::menus::menu_items(app);
    let can = |id: &str| items.iter().find(|it| it.id == id).is_some_and(|it| it.enabled);
    let snapping = app.session.state.snapping;
    let sheet_id = ui.id().with("dock-more-sheet");
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(sheet_id)).unwrap_or(false);

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(6.0, 4.0))).id_salt("mobile-dock"));
    let mut clicked: Option<&'static str> = None;
    child.horizontal_centered(|ui| {
        let mut dock_button = |ui: &mut Ui, icon: crate::icons::Icon, label: &str, id: &'static str, enabled: bool, pressed: bool| {
            let resp = ui.add_enabled(
                enabled,
                egui::Button::new(egui::RichText::new("").size(1.0))
                    .fill(if pressed { t.accent } else { Color32::TRANSPARENT })
                    .min_size(Vec2::new(52.0, MIN_TOUCH_DP)),
            );
            let r = resp.rect;
            let col = if !enabled {
                t.text_faint
            } else if pressed {
                Color32::WHITE
            } else {
                t.text
            };
            crate::icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(r.center().x, r.center().y - 8.0), Vec2::splat(20.0)), icon, col);
            ui.painter().text(Pos2::new(r.center().x, r.max.y - 9.0), Align2::CENTER_CENTER, label, FontId::proportional(9.5), col);
            app.auto.add(&format!("mobile.dock.{id}"), r, label);
            resp.clicked()
        };
        if dock_button(ui, crate::icons::Icon::Undo, tl!("Undo"), "edit.undo", can("edit.undo"), false) {
            clicked = Some("edit.undo");
        }
        if dock_button(ui, crate::icons::Icon::Redo, tl!("Redo"), "edit.redo", can("edit.redo"), false) {
            clicked = Some("edit.redo");
        }
        if dock_button(ui, crate::icons::Icon::Razor, tl!("Split"), "sequence.addEdit", can("sequence.addEdit"), false) {
            clicked = Some("sequence.addEdit");
        }
        if dock_button(ui, crate::icons::Icon::Ripple, tl!("Delete"), "edit.rippleDelete", can("edit.rippleDelete"), false) {
            clicked = Some("edit.rippleDelete");
        }
        if dock_button(ui, crate::icons::Icon::Magnet, tl!("Snap"), "sequence.snap", can("sequence.snap"), snapping) {
            clicked = Some("sequence.snap");
        }
        if dock_button(ui, crate::icons::Icon::Zoom, tl!("Fit"), "view.zoomToSequence", can("view.zoomToSequence"), false) {
            clicked = Some("view.zoomToSequence");
        }
        if dock_button(ui, crate::icons::Icon::Hamburger, tl!("More"), "mobile.more", can("sequence.lift"), open) {
            open = !open;
        }
    });

    // The overflow sheet: the same command list the ⋯ header sheet shows, so the
    // two never drift.
    let picked = bottom_sheet(&ctx, &t, sheet_id, &mut open, tl!("Menu"), |ui| -> Option<&'static str> {
        for (icon, label, id, _) in menu_commands() {
            if command_row(ui, &t, &can, icon, label, id) {
                return Some(id);
            }
        }
        None
    });
    ui.ctx().data_mut(|d| d.insert_temp(sheet_id, open));
    // The sheet wraps its body's Option in its own open/closed Option.
    if let Some(id) = picked.flatten().or(clicked) {
        let params = menu_commands().into_iter().find(|(_, _, cmd, _)| *cmd == id).map(|(_, _, _, p)| p).unwrap_or_else(|| serde_json::json!({}));
        let _ = crate::menus::invoke(app, &ctx, id, params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_pick_the_chrome() {
        // A desktop is never touch chrome, at any size.
        assert_eq!(detect(false, egui::vec2(400.0, 800.0)), Presentation::Desktop);
        // Phone portrait: the stacked shell.
        assert_eq!(detect(true, egui::vec2(400.0, 800.0)), Presentation::MobilePortrait);
        // The 2K tablet at 320 dpi, held upright, is exactly 720 dp short edge: a phone.
        assert_eq!(detect(true, egui::vec2(720.0, 1280.0)), Presentation::MobilePortrait);
        // The same device landscape: the docking body under the touch chrome.
        assert_eq!(detect(true, egui::vec2(1138.0, 640.0)), Presentation::MobileLandscape);
        // A tall narrow window on a big touch display stays landscape chrome: portrait
        // stacking is for phones, not for a resized tablet side-strip.
        assert_eq!(detect(true, egui::vec2(900.0, 1400.0)), Presentation::MobileLandscape);
    }
}
