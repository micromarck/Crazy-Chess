//! Builds Bevy UI entities from a parsed HTML document and keeps the dynamic
//! parts (bindings, conditions, repeats, button states) up to date.
//!
//! Mapping:
//! * every element → a `Node` styled by the cascade ([`super::style`]);
//! * text → a `Text` child (whitespace trimmed; `{{ }}` bindings stay live);
//! * `<img src="...">` → `ImageNode` loaded from `assets/`;
//! * `<button>` or any element with `data-command` → a clickable button that
//!   sends [`UiCommand`];
//! * `data-if="expr"` → shown only while the condition holds;
//! * `data-for="item in list"` → the element's **children** are repeated for
//!   every entry of `list`, with `{{ item.field }}` available inside.
//!
//! Elements without a background (and all text) don't capture the pointer, so
//! invisible layout containers never block clicks on the battlefield.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::TextLayout;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, observe};

use super::UiData;
use super::bind::{Scope, condition, has_bindings, parse_for, render, render_scope};
use super::css::{ElementInfo, Stylesheet, parse_declarations};
use super::dom::{DomNode, Element};
use super::style::{ComputedStyle, Inherited, compute, unsupported};

/// A UI action requested by an HTML element (`data-command`).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct UiCommand(pub String);

/// Root of a built layout (full-screen, transparent to clicks).
#[derive(Component)]
pub struct HtmlRoot;

/// Text with live `{{ }}` bindings.
#[derive(Component)]
pub struct BoundText(pub String);

/// `data-if`: element shown only while the condition holds.
#[derive(Component)]
pub struct BoundIf {
    expr: String,
    scope: Scope,
    /// The `display` to use when visible.
    shown: Display,
}

/// `data-for`: children rebuilt whenever the list changes.
#[derive(Component)]
pub struct Repeat {
    var: String,
    list: String,
    template: Vec<DomNode>,
    ancestors: Vec<ElementInfo>,
    inherited: Inherited,
    scope: Scope,
    last_hash: Option<u64>,
}

/// Background/border colours per interaction state (from `:hover`/`:active`).
#[derive(Component, Debug, Clone, Copy)]
pub struct StateColors {
    normal: Color,
    hover: Color,
    active: Color,
}

/// Inputs shared while building.
pub struct BuildCtx<'a> {
    pub sheet: &'a Stylesheet,
    pub globals: &'a super::bind::UiValues,
    pub assets: &'a AssetServer,
    /// Diagnostics (unsupported properties...), deduplicated by the caller.
    pub warnings: &'a mut Vec<String>,
}

/// Spawns the full-screen root and the document under it.
pub fn spawn_document(commands: &mut Commands, roots: &[DomNode], ctx: &mut BuildCtx) -> Entity {
    let root = commands
        .spawn((
            HtmlRoot,
            // Full screen, except the edge margin on the right (see `touch`).
            Node { position_type: PositionType::Absolute, left: px(0), top: px(0), right: px(0), height: percent(100), ..default() },
            crate::touch::EdgeInset { base: 0.0 },
            Pickable::IGNORE,
        ))
        .id();
    spawn_nodes(commands, roots, root, &[], &Inherited::default(), &Vec::new(), ctx);
    root
}

/// Spawns `nodes` as children of `parent`.
pub fn spawn_nodes(
    commands: &mut Commands,
    nodes: &[DomNode],
    parent: Entity,
    ancestors: &[ElementInfo],
    inherited: &Inherited,
    scope: &Scope,
    ctx: &mut BuildCtx,
) {
    for node in nodes {
        match node {
            DomNode::Text(raw) => {
                let template = render_scope(raw.trim(), scope);
                if template.is_empty() {
                    continue;
                }
                let mut e = commands.spawn((
                    Text::new(render(&template, &Vec::new(), ctx.globals)),
                    TextFont { font_size: FontSize::Px(inherited.font_size), ..default() },
                    TextColor(inherited.color),
                    TextLayout { justify: inherited.justify, ..default() },
                    Pickable::IGNORE,
                    ChildOf(parent),
                ));
                if has_bindings(&template) {
                    e.insert(BoundText(template));
                }
            }
            DomNode::Element(el) => spawn_element(commands, el, parent, ancestors, inherited, scope, ctx),
        }
    }
}

fn style_for(el: &Element, attr: impl Fn(&str) -> Option<String>, ancestors: &[ElementInfo], inherited: &Inherited, ctx: &mut BuildCtx) -> (ElementInfo, ComputedStyle, Option<StateColors>) {
    let info = ElementInfo {
        tag: el.tag.clone(),
        id: attr("id"),
        classes: attr("class").unwrap_or_default().split_whitespace().map(String::from).collect(),
    };
    let inline = parse_declarations(&attr("style").unwrap_or_default());
    let cascaded = ctx.sheet.cascade(&info, ancestors, &inline);
    for prop in unsupported(&cascaded.normal).into_iter().chain(unsupported(&cascaded.hover)) {
        let w = format!("unsupported CSS property '{prop}'");
        if !ctx.warnings.contains(&w) {
            ctx.warnings.push(w);
        }
    }
    let style = compute(&el.tag, &cascaded.normal, inherited);

    // States: re-run the cascade with :hover / :active declarations on top.
    let states = if cascaded.hover.is_empty() && cascaded.active.is_empty() {
        None
    } else {
        let with = |extra: &[(String, String)]| {
            let mut d = cascaded.normal.clone();
            d.extend(extra.iter().cloned());
            compute(&el.tag, &d, inherited).background
        };
        let normal = style.background.unwrap_or(Color::NONE);
        let hover = with(&cascaded.hover).unwrap_or(normal);
        let mut active_decls = cascaded.hover.clone();
        active_decls.extend(cascaded.active.iter().cloned());
        let active = with(&active_decls).unwrap_or(hover);
        Some(StateColors { normal, hover, active })
    };
    (info, style, states)
}

fn spawn_element(
    commands: &mut Commands,
    el: &Element,
    parent: Entity,
    ancestors: &[ElementInfo],
    inherited: &Inherited,
    scope: &Scope,
    ctx: &mut BuildCtx,
) {
    // Attributes see the enclosing data-for item(s).
    let attr = |name: &str| el.attr(name).map(|v| render_scope(v, scope));
    let (info, style, states) = style_for(el, attr, ancestors, inherited, ctx);

    let command = attr("data-command");
    let is_button = el.tag == "button" || command.is_some();
    let has_background = style.background.is_some_and(|c| c.alpha() > 0.0);

    let mut e = commands.spawn((style.node.clone(), ChildOf(parent)));
    if let Some(bg) = style.background.or(states.map(|s| s.normal)) {
        e.insert(BackgroundColor(bg));
    }
    if let Some(bc) = style.border_color {
        e.insert(BorderColor::all(bc));
    }
    if let Some(z) = style.z_index {
        e.insert(ZIndex(z));
    }
    if !is_button && !has_background {
        e.insert(Pickable::IGNORE);
    }
    if let Some(states) = states {
        e.insert(states);
    }
    if el.tag == "img"
        && let Some(src) = attr("src")
    {
        e.insert(ImageNode::new(ctx.assets.load(src)));
    }
    if is_button {
        let template = command.unwrap_or_default();
        e.insert((Button, Hovered::default()));
        e.insert(observe(move |_: On<Activate>, data: Res<UiData>, drag: Res<crate::touch::UiDrag>, mut out: MessageWriter<UiCommand>| {
            // A finger that scrolled a panel doesn't press the button it started on.
            if drag.dragged {
                return;
            }
            let cmd = render(&template, &Vec::new(), &data.values);
            if !cmd.is_empty() {
                out.write(UiCommand(cmd));
            }
        }));
    }
    if let Some(expr) = attr("data-if") {
        let visible = condition(&expr, scope, ctx.globals);
        let shown = style.node.display;
        if !visible {
            e.insert(Node { display: Display::None, ..style.node.clone() });
        }
        e.insert(BoundIf { expr, scope: scope.clone(), shown });
    }
    let entity = e.id();

    let mut chain = ancestors.to_vec();
    chain.push(info);
    if let Some((var, list)) = attr("data-for").as_deref().and_then(parse_for) {
        commands.entity(entity).insert(Repeat {
            var,
            list,
            template: el.children.clone(),
            ancestors: chain,
            inherited: style.text,
            scope: scope.clone(),
            last_hash: None,
        });
    } else {
        spawn_nodes(commands, &el.children, entity, &chain, &style.text, scope, ctx);
    }
}

// ----------------------------------------------------------------------
// Live updates
// ----------------------------------------------------------------------

pub fn update_bound_text(data: Res<UiData>, mut q: Query<(&BoundText, &mut Text)>) {
    if !data.is_changed() {
        return;
    }
    for (bound, mut text) in &mut q {
        let s = render(&bound.0, &Vec::new(), &data.values);
        if text.0 != s {
            text.0 = s;
        }
    }
}

pub fn update_conditions(data: Res<UiData>, mut q: Query<(&BoundIf, &mut Node)>) {
    if !data.is_changed() {
        return;
    }
    for (bound, mut node) in &mut q {
        let display = if condition(&bound.expr, &bound.scope, &data.values) { bound.shown } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

pub fn update_repeats(
    mut commands: Commands,
    data: Res<UiData>,
    layout: Res<super::HtmlLayout>,
    assets: Res<AssetServer>,
    mut q: Query<(Entity, &mut Repeat)>,
) {
    let mut warnings = Vec::new();
    for (entity, mut repeat) in &mut q {
        // Fill newly built repeats (e.g. after an F6 reload) even if the data
        // itself hasn't changed; otherwise only re-check on data changes.
        if !data.is_changed() && repeat.last_hash.is_some() {
            continue;
        }
        let items = data.values.lists.get(&repeat.list).cloned().unwrap_or_default();
        let mut h = DefaultHasher::new();
        items.hash(&mut h);
        let hash = h.finish();
        if repeat.last_hash == Some(hash) {
            continue;
        }
        repeat.last_hash = Some(hash);
        commands.entity(entity).despawn_related::<Children>();
        let mut ctx = BuildCtx { sheet: &layout.sheet, globals: &data.values, assets: &assets, warnings: &mut warnings };
        for item in items {
            let mut scope = repeat.scope.clone();
            scope.push((repeat.var.clone(), item));
            spawn_nodes(&mut commands, &repeat.template, entity, &repeat.ancestors, &repeat.inherited, &scope, &mut ctx);
        }
    }
}

pub fn update_state_colors(mut q: Query<(&StateColors, &Hovered, Has<Pressed>, &mut BackgroundColor)>) {
    for (states, hovered, pressed, mut bg) in &mut q {
        let color = if pressed {
            states.active
        } else if hovered.get() {
            states.hover
        } else {
            states.normal
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
}
