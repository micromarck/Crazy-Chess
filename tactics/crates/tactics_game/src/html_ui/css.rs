//! A compact CSS engine: stylesheet parsing, selector matching and cascade.
//!
//! Supported selectors: type (`button`), class (`.panel`), id (`#log`),
//! universal (`*`), compounds (`button.primary`), descendant combinators
//! (`#hud .panel span`), comma lists, and the `:hover` / `:active` pseudo
//! classes (used for button states). Specificity and source order decide
//! conflicts like in browsers; inline `style="..."` wins over the sheet.
//! Comments (`/* */`) are ignored. At-rules (`@media`...) are skipped.

/// Interaction state of an element, for `:hover` / `:active` rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pseudo {
    Hover,
    Active,
}

/// One compound selector such as `button.primary:hover`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub pseudo: Option<Pseudo>,
}

/// A descendant chain; the last compound is the subject.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<Compound>,
}

/// What a selector is matched against.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElementInfo {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selector: Selector,
    pub declarations: Vec<(String, String)>,
    /// Source order (later wins at equal specificity).
    pub order: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

impl Compound {
    fn matches(&self, e: &ElementInfo) -> bool {
        self.tag.as_ref().is_none_or(|t| t == "*" || *t == e.tag)
            && self.id.as_ref().is_none_or(|id| e.id.as_ref() == Some(id))
            && self.classes.iter().all(|c| e.classes.contains(c))
    }
}

impl Selector {
    /// (ids, classes + pseudo, tags), compared lexicographically.
    pub fn specificity(&self) -> (u32, u32, u32) {
        self.parts.iter().fold((0, 0, 0), |(a, b, c), p| {
            (
                a + p.id.is_some() as u32,
                b + p.classes.len() as u32 + p.pseudo.is_some() as u32,
                c + p.tag.as_ref().is_some_and(|t| t != "*") as u32,
            )
        })
    }

    /// The pseudo class on the subject, if any.
    pub fn pseudo(&self) -> Option<Pseudo> {
        self.parts.last().and_then(|p| p.pseudo)
    }

    /// Does this selector match `subject` with the given ancestors
    /// (outermost first)? Pseudo classes are ignored here; the caller filters.
    pub fn matches(&self, subject: &ElementInfo, ancestors: &[ElementInfo]) -> bool {
        let Some((last, rest)) = self.parts.split_last() else { return false };
        if !last.matches(subject) {
            return false;
        }
        // Each remaining compound must match some ancestor, innermost first.
        let mut remaining = ancestors.iter().rev();
        rest.iter().rev().all(|part| remaining.by_ref().any(|a| part.matches(a)))
    }
}

/// Parses one selector like `#hud .panel > span:hover` (`>` treated as descendant).
fn parse_selector(text: &str) -> Option<Selector> {
    let mut parts = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == '>').filter(|t| !t.is_empty()) {
        let mut c = Compound::default();
        // Split "tag#id.cls.cls:hover" into pieces at # . :
        let mut buf = String::new();
        let mut kind = ' ';
        let flush = |kind: char, buf: &mut String, c: &mut Compound| -> bool {
            let v = std::mem::take(buf);
            match kind {
                ' ' if !v.is_empty() => c.tag = Some(v.to_ascii_lowercase()),
                '#' => c.id = Some(v),
                '.' => c.classes.push(v),
                ':' => match v.as_str() {
                    "hover" => c.pseudo = Some(Pseudo::Hover),
                    "active" => c.pseudo = Some(Pseudo::Active),
                    _ => return false, // unsupported pseudo: drop the rule
                },
                _ => {}
            }
            true
        };
        for ch in token.chars() {
            if matches!(ch, '#' | '.' | ':') {
                if !flush(kind, &mut buf, &mut c) {
                    return None;
                }
                kind = ch;
            } else {
                buf.push(ch);
            }
        }
        if !flush(kind, &mut buf, &mut c) {
            return None;
        }
        parts.push(c);
    }
    (!parts.is_empty()).then_some(Selector { parts })
}

/// Parses `a: b; c: d` into `(property, value)` pairs (lower-case properties).
pub fn parse_declarations(text: &str) -> Vec<(String, String)> {
    text.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().trim_end_matches("!important").trim().to_string();
            (!k.is_empty() && !v.is_empty()).then_some((k, v))
        })
        .collect()
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..].find("*/").map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// Parses a stylesheet. Malformed rules are skipped, never fatal.
pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let css = strip_comments(css);
    let mut rules = Vec::new();
    let mut rest = css.as_str();
    let mut order = 0;
    while let Some(open) = rest.find('{') {
        let head = rest[..open].trim();
        // Find the matching close brace (at-rules may nest one level).
        let mut depth = 0;
        let mut close = rest.len();
        for (i, ch) in rest[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &rest[open + 1..close.min(rest.len())];
        if !head.starts_with('@') {
            let declarations = parse_declarations(body);
            for sel in head.split(',').filter_map(parse_selector) {
                rules.push(Rule { selector: sel, declarations: declarations.clone(), order });
                order += 1;
            }
        }
        rest = rest.get(close + 1..).unwrap_or("");
    }
    Stylesheet { rules }
}

/// Declarations for each interaction state of one element, cascaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cascaded {
    pub normal: Vec<(String, String)>,
    /// Extra declarations applied while hovered (on top of `normal`).
    pub hover: Vec<(String, String)>,
    /// Extra declarations applied while pressed.
    pub active: Vec<(String, String)>,
}

impl Stylesheet {
    /// Runs the cascade for an element. Later entries override earlier ones.
    pub fn cascade(&self, subject: &ElementInfo, ancestors: &[ElementInfo], inline: &[(String, String)]) -> Cascaded {
        let mut matched: Vec<&Rule> = self.rules.iter().filter(|r| r.selector.matches(subject, ancestors)).collect();
        matched.sort_by_key(|r| (r.selector.specificity(), r.order));
        let mut out = Cascaded::default();
        for r in matched {
            let target = match r.selector.pseudo() {
                None => &mut out.normal,
                Some(Pseudo::Hover) => &mut out.hover,
                Some(Pseudo::Active) => &mut out.active,
            };
            target.extend(r.declarations.iter().cloned());
        }
        out.normal.extend(inline.iter().cloned());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(tag: &str, id: Option<&str>, classes: &[&str]) -> ElementInfo {
        ElementInfo { tag: tag.into(), id: id.map(Into::into), classes: classes.iter().map(|c| c.to_string()).collect() }
    }

    fn get<'a>(decls: &'a [(String, String)], key: &str) -> Option<&'a str> {
        decls.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn specificity_and_order() {
        let sheet = parse_stylesheet(
            "/* c */ button { color: red; } .btn { color: blue } #go { color: green; }
             button { color: yellow; padding: 4px }",
        );
        let c = sheet.cascade(&info("button", Some("go"), &["btn"]), &[], &[]);
        assert_eq!(get(&c.normal, "color"), Some("green"), "id beats class beats tag");
        assert_eq!(get(&c.normal, "padding"), Some("4px"));
        let c = sheet.cascade(&info("button", None, &[]), &[], &[]);
        assert_eq!(get(&c.normal, "color"), Some("yellow"), "later tag rule wins");
    }

    #[test]
    fn descendant_and_pseudo() {
        let sheet = parse_stylesheet("#hud .bar button { background: #111 } #hud button:hover { background: #222 } span:focus { x: y }");
        let ancestors = [info("div", Some("hud"), &[]), info("div", None, &["bar"])];
        let c = sheet.cascade(&info("button", None, &[]), &ancestors, &[]);
        assert_eq!(get(&c.normal, "background"), Some("#111"));
        assert_eq!(get(&c.hover, "background"), Some("#222"));
        // Not inside .bar → only the hover rule applies.
        let c = sheet.cascade(&info("button", None, &[]), &ancestors[..1], &[]);
        assert!(c.normal.is_empty());
        assert_eq!(sheet.rules.len(), 2, "unsupported :focus rule dropped");
    }

    #[test]
    fn inline_wins_and_comma_lists() {
        let sheet = parse_stylesheet("h1, .title { font-size: 30px } @media (x) { p { color: red } } p { color: blue }");
        let inline = parse_declarations("font-size: 12px; ");
        let c = sheet.cascade(&info("h1", None, &[]), &[], &inline);
        assert_eq!(get(&c.normal, "font-size"), Some("12px"));
        let c = sheet.cascade(&info("p", None, &[]), &[], &[]);
        assert_eq!(get(&c.normal, "color"), Some("blue"), "@media skipped");
    }
}
