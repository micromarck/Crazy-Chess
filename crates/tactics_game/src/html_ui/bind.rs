//! Template bindings: `{{ path }}` placeholders and `data-if` conditions.
//!
//! Values are plain strings looked up by dotted path (`unit.hp`). Lists (for
//! `data-for`) are vectors of string maps. Rendering never fails: unknown
//! paths render as an empty string, so a typo shows as a blank, not a crash.

use std::collections::{BTreeMap, HashMap};

/// All data the UI can bind to, refreshed every frame by `data.rs`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiValues {
    pub values: HashMap<String, String>,
    pub lists: HashMap<String, Vec<BTreeMap<String, String>>>,
}

impl UiValues {
    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.values.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

/// Item variables of an enclosing `data-for`, e.g. `ability` → its fields.
pub type Scope = Vec<(String, BTreeMap<String, String>)>;

/// Looks a path up in the scope first (`ability.name`), then the globals.
pub fn lookup<'a>(path: &str, scope: &'a Scope, globals: &'a UiValues) -> Option<&'a str> {
    if let Some((root, field)) = path.split_once('.') {
        for (var, item) in scope.iter().rev() {
            if var == root {
                return item.get(field).map(String::as_str);
            }
        }
    }
    globals.get(path)
}

/// Replaces every `{{ path }}` in `template`.
pub fn render(template: &str, scope: &Scope, globals: &UiValues) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(lookup(after[..end].trim(), scope, globals).unwrap_or(""));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Replaces only placeholders that refer to scope variables; global
/// placeholders are kept for live updates.
pub fn render_scope(template: &str, scope: &Scope) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let path = after[..end].trim();
        let scoped = path
            .split_once('.')
            .and_then(|(root, field)| scope.iter().rev().find(|(v, _)| v == root).map(|(_, item)| item.get(field)));
        match scoped {
            Some(value) => out.push_str(value.map_or("", String::as_str)),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Does `template` contain placeholders?
pub fn has_bindings(template: &str) -> bool {
    template.contains("{{")
}

/// Truthiness for `data-if`: empty, `0`, `false`, `no` and `off` are false.
pub fn truthy(value: &str) -> bool {
    !matches!(value.trim(), "" | "0" | "false" | "no" | "off")
}

/// Evaluates a `data-if` expression: `path`, `!path`, `path == value`,
/// `path != value`, combined with `&&` and `||` (`&&` binds tighter; write
/// `&amp;&amp;` inside HTML attributes, or use the word forms `and` / `or`).
pub fn condition(expr: &str, scope: &Scope, globals: &UiValues) -> bool {
    let expr = expr.replace(" or ", " || ").replace(" and ", " && ");
    expr.split("||").any(|any| any.split("&&").all(|term| simple_condition(term, scope, globals)))
}

fn simple_condition(expr: &str, scope: &Scope, globals: &UiValues) -> bool {
    let expr = expr.trim();
    for (op, negate) in [("!=", true), ("==", false)] {
        if let Some((l, r)) = expr.split_once(op) {
            let left = lookup(l.trim(), scope, globals).unwrap_or("");
            let right = r.trim().trim_matches(|c| c == '\'' || c == '"');
            return (left == right) != negate;
        }
    }
    match expr.strip_prefix('!') {
        Some(path) => !truthy(lookup(path.trim(), scope, globals).unwrap_or("")),
        None => truthy(lookup(expr, scope, globals).unwrap_or("")),
    }
}

/// Parses `data-for="item in list"` into `(item, list)`.
pub fn parse_for(expr: &str) -> Option<(String, String)> {
    let (var, list) = expr.split_once(" in ")?;
    Some((var.trim().to_string(), list.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> (Scope, UiValues) {
        let mut g = UiValues::default();
        g.set("unit.hp", 7);
        g.set("battle.over", "false");
        g.set("turn.team", "XCOM");
        let item: BTreeMap<_, _> = [("name".to_string(), "Shoot".to_string())].into();
        (vec![("ability".into(), item)], g)
    }

    #[test]
    fn render_scope_and_globals() {
        let (scope, g) = data();
        assert_eq!(render("HP {{ unit.hp }} / {{missing}} {{ability.name}}", &scope, &g), "HP 7 /  Shoot");
        assert_eq!(render_scope("{{ability.name}} {{unit.hp}}", &scope), "Shoot {{unit.hp}}");
        assert_eq!(render("broken {{ x", &scope, &g), "broken {{ x");
    }

    #[test]
    fn conditions() {
        let (scope, g) = data();
        assert!(condition("unit.hp", &scope, &g));
        assert!(!condition("battle.over", &scope, &g));
        assert!(condition("!battle.over", &scope, &g));
        assert!(condition("turn.team == XCOM", &scope, &g));
        assert!(condition("turn.team != 'ALIEN'", &scope, &g));
        assert!(condition("ability.name == Shoot", &scope, &g));
        assert!(condition("unit.hp && !battle.over", &scope, &g));
        assert!(!condition("unit.hp && battle.over", &scope, &g));
        assert!(condition("battle.over || turn.team == XCOM", &scope, &g));
        assert!(condition("battle.over or unit.hp and turn.team == XCOM", &scope, &g));
        assert_eq!(parse_for("a in abilities"), Some(("a".into(), "abilities".into())));
    }
}
