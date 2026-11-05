//! Touchscreen support that isn't camera work (that's in `camera.rs`):
//!
//! * [`TouchMode`]: whether the player is using a touchscreen (always on
//!   Android; on other platforms it switches on at the first touch and off
//!   again when the mouse is used). The HUD reads it as `ui.touch` to show the
//!   touch-only controls (the **Move** button and the move confirmation), and
//!   the board input uses it for the touch rules (see `input.rs`). Set
//!   `TBS_TOUCH=1` to force it on (to try the touch interface with a mouse).
//! * **UI size**: applies the `ui_scale` setting to Bevy's [`UiScale`]. On
//!   `auto`, phones (touch, short screens) get a slightly smaller HUD so the
//!   panels fit, desktops get 1.0.
//! * **Screen edge margin**: UI anchored to the right edge ([`EdgeInset`]:
//!   the HUD root, the minimap) keeps clear of the `edge_margin` setting,
//!   so Android's back / home buttons (right edge in landscape) don't cover
//!   the rotate or End Turn buttons.
//! * **Scrolling menus without a mouse**: dragging a finger over any panel
//!   with `overflow: scroll` scrolls it, with a fling when you let go; the
//!   mouse wheel scrolls HUD panels too. A press that turned into a scroll
//!   doesn't also press the button it started on ([`UiDrag::dragged`]).

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::ui::OverflowAxis;

use crate::FrameSet;

pub struct TouchPlugin;

impl Plugin for TouchPlugin {
    fn build(&self, app: &mut App) {
        let forced = forced_touch();
        app.insert_resource(TouchMode { active: cfg!(target_os = "android") || forced, forced })
            .init_resource::<UiDrag>()
            .add_systems(Startup, fold_log_on_touchscreens)
            .add_systems(PreUpdate, detect_touch_mode)
            .add_systems(Update, (apply_ui_scale, apply_edge_margin, scroll_ui).in_set(FrameSet::Input));
    }
}

/// Is the player on a touchscreen right now?
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchMode {
    pub active: bool,
    /// Kept on whatever the mouse does (Android, or `TBS_TOUCH=1`).
    pub forced: bool,
}

fn forced_touch() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        cfg!(target_os = "android") || std::env::var("TBS_TOUCH").is_ok_and(|v| v != "0")
    }
    #[cfg(target_arch = "wasm32")]
    {
        is_touch_device()
    }
}

/// Web: a phone or tablet (a coarse pointer, i.e. a finger, is the main
/// input). Such devices start in touch mode and stay there.
#[cfg(target_arch = "wasm32")]
pub fn is_touch_device() -> bool {
    let Some(w) = web_sys::window() else { return false };
    w.match_media("(pointer: coarse)").ok().flatten().is_some_and(|m| m.matches())
}

/// Switches fullscreen on or off. Web: the page goes fullscreen (needs the
/// tap that asked for it, which a HUD button is) and phones lock to
/// landscape where the browser allows it. Desktop: borderless fullscreen.
/// Android (native) is always fullscreen: nothing to do.
pub fn toggle_fullscreen(window: &mut Window) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = window;
        let Some(w) = web_sys::window() else { return };
        let Some(doc) = w.document() else { return };
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else if let Some(root) = doc.document_element() {
            let _ = root.request_fullscreen();
            if let Ok(screen) = w.screen() {
                let _ = screen.orientation().lock(web_sys::OrientationLockType::Landscape);
            }
        }
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        use bevy::window::{MonitorSelection, WindowMode};
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
    }
}

/// A finger dragging a scrollable panel.
#[derive(Resource, Default)]
pub struct UiDrag {
    /// The panel being scrolled.
    target: Option<Entity>,
    /// How far the finger has travelled since it went down (pixels).
    travelled: f32,
    /// The current press became a scroll: buttons ignore its release.
    pub dragged: bool,
    /// Fling speed after release (UI pixels per second).
    velocity: f32,
}

/// Pixels a finger must move over a panel before it scrolls instead of
/// pressing a button.
const SCROLL_SLOP: f32 = 10.0;

fn detect_touch_mode(
    touches: Res<Touches>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut cursor: MessageReader<CursorMoved>,
    mut mode: ResMut<TouchMode>,
) {
    if touches.any_just_pressed() {
        if !mode.active {
            mode.active = true;
        }
        cursor.clear();
        return;
    }
    // Android sends no real mouse; elsewhere a mouse click or movement
    // without a finger down means the mouse is in use again.
    if !mode.forced && touches.iter().next().is_none() && (buttons.get_just_pressed().next().is_some() || cursor.read().count() > 0) && mode.active {
        mode.active = false;
    }
}

/// Phones and tablets start with the mission log folded (it covers a lot of
/// a small screen); a tap opens it.
fn fold_log_on_touchscreens(mode: Res<TouchMode>, mut flags: ResMut<crate::html_ui::UiFlags>) {
    if mode.active {
        flags.0.insert("log_hidden".into(), true);
    }
}

/// The scale `auto` picks: phones (touch, under ~560 logical pixels tall)
/// shrink the HUD so its panels fit; everything else stays at 1.
pub fn auto_ui_scale(touch: bool, window_height: f32) -> f32 {
    if touch && window_height < 560.0 { (window_height / 560.0).clamp(0.72, 1.0) } else { 1.0 }
}

fn apply_ui_scale(settings: Res<crate::settings::GameSettings>, mode: Res<TouchMode>, window: Single<&Window>, mut scale: ResMut<UiScale>) {
    let want = if settings.ui_scale > 0.0 { settings.ui_scale } else { auto_ui_scale(mode.active, window.height()) };
    if (scale.0 - want).abs() > 1e-3 {
        scale.0 = want;
    }
}

/// A UI node anchored to the right screen edge: its `right` offset is
/// `base` UI pixels plus the screen edge margin.
#[derive(Component, Debug, Clone, Copy)]
pub struct EdgeInset {
    pub base: f32,
}

fn apply_edge_margin(settings: Res<crate::settings::GameSettings>, scale: Res<UiScale>, mut nodes: Query<(&EdgeInset, &mut Node)>) {
    // The margin is in screen pixels; UI pixels are scaled by UiScale.
    let margin = settings.edge_margin_px() / scale.0.max(0.1);
    for (inset, mut node) in &mut nodes {
        let want = Val::Px(inset.base + margin);
        if node.right != want {
            node.right = want;
        }
    }
}

/// The nearest ancestor-or-self that scrolls vertically.
fn scrollable_ancestor(e: Entity, parents: &Query<&ChildOf>, nodes: &Query<(&Node, &ComputedNode, &mut ScrollPosition)>) -> Option<Entity> {
    std::iter::once(e).chain(parents.iter_ancestors(e)).find(|a| nodes.get(*a).is_ok_and(|(n, ..)| n.overflow.y == OverflowAxis::Scroll))
}

/// Scrolls `target` by `dy` UI pixels, clamped to its content.
fn scroll_by(target: Entity, dy: f32, nodes: &mut Query<(&Node, &ComputedNode, &mut ScrollPosition)>) -> bool {
    let Ok((_, computed, mut pos)) = nodes.get_mut(target) else { return false };
    let max = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor()).max(0.0);
    let y = (pos.0.y + dy).clamp(0.0, max);
    let moved = (y - pos.0.y).abs() > 0.01;
    pos.0.y = y;
    moved
}

#[allow(clippy::too_many_arguments)]
fn scroll_ui(
    time: Res<Time<Real>>,
    touches: Res<Touches>,
    wheel: Res<AccumulatedMouseScroll>,
    hover_map: Res<HoverMap>,
    scale: Res<UiScale>,
    parents: Query<&ChildOf>,
    html: Query<(), With<crate::html_ui::build::HtmlRoot>>,
    mut nodes: Query<(&Node, &ComputedNode, &mut ScrollPosition)>,
    mut drag: ResMut<UiDrag>,
) {
    let dt = time.delta_secs();
    let k = 1.0 / scale.0.max(0.1);

    // A finger goes down: is it on a scrollable panel?
    if let Some(t) = touches.iter_just_pressed().next() {
        drag.travelled = 0.0;
        drag.dragged = false;
        drag.velocity = 0.0;
        drag.target = hover_map
            .get(&PointerId::Touch(t.id()))
            .into_iter()
            .flat_map(|hits| hits.keys().copied())
            .find_map(|e| scrollable_ancestor(e, &parents, &nodes));
    }
    if let Some(target) = drag.target {
        let fingers: Vec<&bevy::input::touch::Touch> = touches.iter().collect();
        if let [finger] = fingers.as_slice() {
            let dy = finger.delta().y;
            drag.travelled += finger.delta().length();
            if drag.travelled > SCROLL_SLOP {
                drag.dragged = true;
            }
            if drag.dragged && dy != 0.0 {
                // Content follows the finger: finger up = scroll down.
                scroll_by(target, -dy * k, &mut nodes);
                if dt > 0.0 {
                    drag.velocity = drag.velocity * 0.5 + (-dy * k / dt) * 0.5;
                }
            }
        } else if fingers.is_empty() {
            // Released: fling, slowing down.
            if drag.velocity.abs() > 20.0 && dt > 0.0 {
                if !scroll_by(target, drag.velocity * dt, &mut nodes) {
                    drag.velocity = 0.0;
                }
                drag.velocity *= (-4.0 * dt).exp();
            } else {
                drag.target = None;
            }
        }
    }

    // Mouse wheel over a HUD panel (the editor scrolls itself).
    let lines = match wheel.unit {
        MouseScrollUnit::Line => wheel.delta.y * 40.0,
        MouseScrollUnit::Pixel => wheel.delta.y,
    };
    if lines != 0.0
        && let Some(target) = hover_map
            .get(&PointerId::Mouse)
            .into_iter()
            .flat_map(|hits| hits.keys().copied())
            .filter_map(|e| scrollable_ancestor(e, &parents, &nodes))
            .find(|e| parents.iter_ancestors(*e).any(|a| html.contains(a)))
    {
        scroll_by(target, -lines * k, &mut nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_scale_shrinks_only_small_touchscreens() {
        assert_eq!(auto_ui_scale(false, 400.0), 1.0, "desktop windows keep 1.0");
        assert_eq!(auto_ui_scale(true, 900.0), 1.0, "tablets keep 1.0");
        let phone = auto_ui_scale(true, 392.0);
        assert!((0.72..1.0).contains(&phone), "{phone}");
    }
}
