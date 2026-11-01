//! Converts cascaded CSS declarations into Bevy UI components.
//!
//! Every element is a flexbox `Node` (like `display: flex` everywhere).
//! Supported properties:
//!
//! | group | properties |
//! |---|---|
//! | box | `width height min-width min-height max-width max-height padding margin` (1-4 values) |
//! | position | `position: absolute|relative`, `top right bottom left`, `z-index` |
//! | flex | `display: flex|none`, `flex-direction`, `flex-wrap`, `justify-content`, `align-items`, `align-self`, `gap row-gap column-gap`, `flex` / `flex-grow`, `flex-shrink`, `flex-basis` |
//! | look | `background` / `background-color`, `border` (`1px solid #fff`), `border-width`, `border-color`, `border-radius`, `overflow: hidden|scroll|visible` |
//! | text | `color`, `font-size`, `text-align` (inherited by children like in CSS) |
//!
//! Units: `px`, `%`, `vw`, `vh`, `vmin`, `vmax`, `em` (font sizes), `auto`, bare
//! numbers as px. Colours: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`,
//! `rgb()/rgba()`, `transparent` and common names. Unknown properties are
//! ignored (and listed by `unsupported` for diagnostics).

use bevy::prelude::*;
use bevy::text::Justify;

/// Text properties inherited from parent to child.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inherited {
    pub color: Color,
    pub font_size: f32,
    pub justify: Justify,
}

impl Default for Inherited {
    fn default() -> Self {
        Self { color: Color::srgb(0.92, 0.93, 0.95), font_size: 14.0, justify: Justify::Left }
    }
}

/// Resolved style of one element.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub node: Node,
    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub z_index: Option<i32>,
    /// What children inherit (and what this element's own text uses).
    pub text: Inherited,
}

/// Default layout per tag, before any CSS (a tiny user-agent stylesheet).
pub fn tag_defaults(tag: &str, inherited: &Inherited) -> (Node, Inherited) {
    let mut node = Node { flex_direction: FlexDirection::Column, ..default() };
    let mut text = *inherited;
    match tag {
        "span" | "b" | "strong" | "i" | "em" | "label" | "a" | "small" | "p" | "button" | "h1" | "h2" | "h3" | "h4" => {
            node.flex_direction = FlexDirection::Row;
            node.flex_wrap = FlexWrap::Wrap;
            node.align_items = AlignItems::Center;
        }
        "br" => {
            node.width = Val::Percent(100.0);
            node.height = Val::Px(0.0);
        }
        "hr" => {
            node.width = Val::Percent(100.0);
            node.height = Val::Px(1.0);
            node.margin = UiRect::vertical(Val::Px(6.0));
        }
        _ => {}
    }
    match tag {
        "h1" => text.font_size = 28.0,
        "h2" => text.font_size = 22.0,
        "h3" => text.font_size = 18.0,
        "h4" => text.font_size = 16.0,
        "small" => text.font_size = inherited.font_size * 0.8,
        "p" => node.margin = UiRect::vertical(Val::Px(4.0)),
        "button" => {
            // Buttons lay out their label + hint without wrapping (a wrapped
            // column would push the hint *beside* the label when space is tight).
            node.flex_wrap = FlexWrap::NoWrap;
            node.justify_content = JustifyContent::Center;
            node.padding = UiRect::axes(Val::Px(10.0), Val::Px(6.0));
        }
        _ => {}
    }
    (node, text)
}

/// Parses a length. `font` is the inherited font size (for `em`).
pub fn parse_val(v: &str, font: f32) -> Option<Val> {
    let v = v.trim();
    if v == "auto" {
        return Some(Val::Auto);
    }
    let num = |suffix: &str| v.strip_suffix(suffix).and_then(|n| n.trim().parse::<f32>().ok());
    if let Some(n) = num("px") {
        Some(Val::Px(n))
    } else if let Some(n) = num("%") {
        Some(Val::Percent(n))
    } else if let Some(n) = num("vmin") {
        Some(Val::VMin(n))
    } else if let Some(n) = num("vmax") {
        Some(Val::VMax(n))
    } else if let Some(n) = num("vw") {
        Some(Val::Vw(n))
    } else if let Some(n) = num("vh") {
        Some(Val::Vh(n))
    } else if let Some(n) = num("em").or_else(|| num("rem")) {
        Some(Val::Px(n * font))
    } else {
        v.parse::<f32>().ok().map(Val::Px)
    }
}

/// Parses 1-4 lengths into a rect (CSS shorthand order: top right bottom left).
fn parse_rect(v: &str, font: f32) -> Option<UiRect> {
    let vals: Vec<Val> = v.split_whitespace().map(|p| parse_val(p, font)).collect::<Option<_>>()?;
    Some(match vals.as_slice() {
        [a] => UiRect::all(*a),
        [v, h] => UiRect::axes(*h, *v),
        [t, h, b] => UiRect::new(*h, *h, *t, *b),
        [t, r, b, l] => UiRect::new(*l, *r, *t, *b),
        _ => return None,
    })
}

/// Parses a CSS colour.
pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        let (r, g, b, a) = match digits.as_slice() {
            [r, g, b] => (r * 17, g * 17, b * 17, 255),
            [r, g, b, a] => (r * 17, g * 17, b * 17, a * 17),
            [r1, r2, g1, g2, b1, b2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255),
            [r1, r2, g1, g2, b1, b2, a1, a2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, a1 * 16 + a2),
            _ => return None,
        };
        return Some(Color::srgba_u8(r, g, b, a));
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let nums: Vec<f32> = args.trim_end_matches(')').split(',').map(|n| n.trim().parse().ok()).collect::<Option<_>>()?;
        return match nums.as_slice() {
            [r, g, b] => Some(Color::srgb(r / 255.0, g / 255.0, b / 255.0)),
            [r, g, b, a] => Some(Color::srgba(r / 255.0, g / 255.0, b / 255.0, *a)),
            _ => None,
        };
    }
    Some(match v.as_str() {
        "transparent" | "none" => Color::NONE,
        "white" => Color::WHITE,
        "black" => Color::BLACK,
        "red" => Color::srgb(0.9, 0.2, 0.2),
        "green" => Color::srgb(0.2, 0.8, 0.3),
        "blue" => Color::srgb(0.25, 0.5, 0.95),
        "yellow" => Color::srgb(1.0, 0.85, 0.2),
        "orange" => Color::srgb(1.0, 0.6, 0.15),
        "purple" => Color::srgb(0.6, 0.35, 0.9),
        "gray" | "grey" => Color::srgb(0.5, 0.5, 0.5),
        _ => return None,
    })
}

fn keyword<T>(v: &str, table: &[(&str, T)]) -> Option<T>
where
    T: Copy,
{
    table.iter().find(|(k, _)| *k == v.trim()).map(|(_, t)| *t)
}

const JUSTIFY: &[(&str, JustifyContent)] = &[
    ("flex-start", JustifyContent::FlexStart),
    ("start", JustifyContent::Start),
    ("flex-end", JustifyContent::FlexEnd),
    ("end", JustifyContent::End),
    ("center", JustifyContent::Center),
    ("space-between", JustifyContent::SpaceBetween),
    ("space-around", JustifyContent::SpaceAround),
    ("space-evenly", JustifyContent::SpaceEvenly),
];
const ALIGN: &[(&str, AlignItems)] = &[
    ("flex-start", AlignItems::FlexStart),
    ("start", AlignItems::Start),
    ("flex-end", AlignItems::FlexEnd),
    ("end", AlignItems::End),
    ("center", AlignItems::Center),
    ("stretch", AlignItems::Stretch),
    ("baseline", AlignItems::Baseline),
];
const ALIGN_SELF: &[(&str, AlignSelf)] = &[
    ("auto", AlignSelf::Auto),
    ("flex-start", AlignSelf::FlexStart),
    ("flex-end", AlignSelf::FlexEnd),
    ("center", AlignSelf::Center),
    ("stretch", AlignSelf::Stretch),
];

/// Applies declarations (in cascade order) on top of the tag defaults.
pub fn compute(tag: &str, declarations: &[(String, String)], inherited: &Inherited) -> ComputedStyle {
    let (mut node, mut text) = tag_defaults(tag, inherited);
    let mut style = ComputedStyle { node: Node::default(), background: None, border_color: None, z_index: None, text };

    // Font size first: `em` lengths depend on it.
    for (k, v) in declarations {
        if k == "font-size"
            && let Some(Val::Px(px)) = parse_val(v, inherited.font_size) {
                text.font_size = px;
            }
    }
    let font = text.font_size;
    let len = |v: &str| parse_val(v, font);

    for (k, v) in declarations {
        let v = v.as_str();
        match k.as_str() {
            "width" => node.width = len(v).unwrap_or(node.width),
            "height" => node.height = len(v).unwrap_or(node.height),
            "min-width" => node.min_width = len(v).unwrap_or(node.min_width),
            "min-height" => node.min_height = len(v).unwrap_or(node.min_height),
            "max-width" => node.max_width = len(v).unwrap_or(node.max_width),
            "max-height" => node.max_height = len(v).unwrap_or(node.max_height),
            "padding" => node.padding = parse_rect(v, font).unwrap_or(node.padding),
            "margin" => node.margin = parse_rect(v, font).unwrap_or(node.margin),
            "padding-top" => node.padding.top = len(v).unwrap_or(node.padding.top),
            "padding-bottom" => node.padding.bottom = len(v).unwrap_or(node.padding.bottom),
            "padding-left" => node.padding.left = len(v).unwrap_or(node.padding.left),
            "padding-right" => node.padding.right = len(v).unwrap_or(node.padding.right),
            "margin-top" => node.margin.top = len(v).unwrap_or(node.margin.top),
            "margin-bottom" => node.margin.bottom = len(v).unwrap_or(node.margin.bottom),
            "margin-left" => node.margin.left = len(v).unwrap_or(node.margin.left),
            "margin-right" => node.margin.right = len(v).unwrap_or(node.margin.right),
            "position" => {
                node.position_type = if v.trim() == "absolute" || v.trim() == "fixed" {
                    PositionType::Absolute
                } else {
                    PositionType::Relative
                }
            }
            "top" => node.top = len(v).unwrap_or(node.top),
            "bottom" => node.bottom = len(v).unwrap_or(node.bottom),
            "left" => node.left = len(v).unwrap_or(node.left),
            "right" => node.right = len(v).unwrap_or(node.right),
            "z-index" => style.z_index = v.trim().parse().ok(),
            "display" => node.display = if v.trim() == "none" { Display::None } else { Display::Flex },
            "flex-direction" => {
                node.flex_direction = match v.trim() {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => FlexDirection::Column,
                }
            }
            "flex-wrap" => {
                node.flex_wrap = match v.trim() {
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => FlexWrap::NoWrap,
                }
            }
            "justify-content" => node.justify_content = keyword(v, JUSTIFY).unwrap_or(node.justify_content),
            "align-items" => node.align_items = keyword(v, ALIGN).unwrap_or(node.align_items),
            "align-self" => node.align_self = keyword(v, ALIGN_SELF).unwrap_or(node.align_self),
            "gap" => {
                if let Some(g) = len(v) {
                    node.row_gap = g;
                    node.column_gap = g;
                }
            }
            "row-gap" => node.row_gap = len(v).unwrap_or(node.row_gap),
            "column-gap" => node.column_gap = len(v).unwrap_or(node.column_gap),
            "flex" | "flex-grow" => {
                node.flex_grow = v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(node.flex_grow)
            }
            "flex-shrink" => node.flex_shrink = v.trim().parse().unwrap_or(node.flex_shrink),
            "flex-basis" => node.flex_basis = len(v).unwrap_or(node.flex_basis),
            "overflow" => {
                node.overflow = match v.trim() {
                    "hidden" | "clip" => Overflow::clip(),
                    "scroll" | "auto" => Overflow::scroll_y(),
                    _ => Overflow::visible(),
                }
            }
            "background" | "background-color" => style.background = parse_color(v).or(style.background),
            "border" => {
                for part in v.split_whitespace() {
                    if let Some(w) = len(part) {
                        node.border = UiRect::all(w);
                    } else if let Some(c) = parse_color(part) {
                        style.border_color = Some(c);
                    }
                }
            }
            "border-width" => node.border = parse_rect(v, font).unwrap_or(node.border),
            "border-color" => style.border_color = parse_color(v).or(style.border_color),
            "border-radius" => {
                if let Some(r) = len(v.split_whitespace().next().unwrap_or("")) {
                    node.border_radius = BorderRadius::all(r);
                }
            }
            "color" => text.color = parse_color(v).unwrap_or(text.color),
            "text-align" => {
                text.justify = match v.trim() {
                    "center" => Justify::Center,
                    "right" | "end" => Justify::Right,
                    _ => Justify::Left,
                }
            }
            _ => {}
        }
    }
    style.node = node;
    style.text = text;
    style
}

/// Properties [`compute`] understands (used to warn about typos).
pub const SUPPORTED: &[&str] = &[
    "width", "height", "min-width", "min-height", "max-width", "max-height", "padding", "margin", "padding-top",
    "padding-bottom", "padding-left", "padding-right", "margin-top", "margin-bottom", "margin-left", "margin-right",
    "position", "top", "bottom", "left", "right", "z-index", "display", "flex-direction", "flex-wrap",
    "justify-content", "align-items", "align-self", "gap", "row-gap", "column-gap", "flex", "flex-grow",
    "flex-shrink", "flex-basis", "overflow", "background", "background-color", "border", "border-width",
    "border-color", "border-radius", "color", "font-size", "text-align",
];

/// Properties in `declarations` that are not supported.
pub fn unsupported(declarations: &[(String, String)]) -> Vec<String> {
    declarations.iter().filter(|(k, _)| !SUPPORTED.contains(&k.as_str())).map(|(k, _)| k.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html_ui::css::parse_declarations;

    #[test]
    fn values_and_colors() {
        assert_eq!(parse_val("12px", 14.0), Some(Val::Px(12.0)));
        assert_eq!(parse_val("50%", 14.0), Some(Val::Percent(50.0)));
        assert_eq!(parse_val("2em", 10.0), Some(Val::Px(20.0)));
        assert_eq!(parse_val("auto", 14.0), Some(Val::Auto));
        assert_eq!(parse_val("8", 14.0), Some(Val::Px(8.0)));
        assert_eq!(parse_val("wide", 14.0), None);
        assert_eq!(parse_color("#fff"), Some(Color::srgba_u8(255, 255, 255, 255)));
        assert_eq!(parse_color("#10203040"), Some(Color::srgba_u8(16, 32, 48, 64)));
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some(Color::srgba(1.0, 0.0, 0.0, 0.5)));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn compute_box_and_text() {
        let d = parse_declarations(
            "position: absolute; bottom: 12px; padding: 4px 8px; display: flex; flex-direction: row;
             background: #000; border: 2px solid #fff; color: red; font-size: 20px; margin: 1em; frobnicate: 3",
        );
        let s = compute("div", &d, &Inherited::default());
        assert_eq!(s.node.position_type, PositionType::Absolute);
        assert_eq!(s.node.bottom, Val::Px(12.0));
        assert_eq!(s.node.padding, UiRect::axes(Val::Px(8.0), Val::Px(4.0)));
        assert_eq!(s.node.flex_direction, FlexDirection::Row);
        assert_eq!(s.node.border, UiRect::all(Val::Px(2.0)));
        assert_eq!(s.node.margin, UiRect::all(Val::Px(20.0)), "em uses this element's font size");
        assert_eq!(s.text.font_size, 20.0);
        assert!(s.background.is_some() && s.border_color.is_some());
        assert_eq!(unsupported(&d), vec!["frobnicate".to_string()]);
    }

    #[test]
    fn inheritance() {
        let parent = compute("div", &parse_declarations("color: #00ff00; font-size: 18px"), &Inherited::default());
        let child = compute("span", &[], &parent.text);
        assert_eq!(child.text.font_size, 18.0);
        assert_eq!(child.text.color, parent.text.color);
        let h1 = compute("h1", &[], &parent.text);
        assert_eq!(h1.text.font_size, 28.0);
    }
}
