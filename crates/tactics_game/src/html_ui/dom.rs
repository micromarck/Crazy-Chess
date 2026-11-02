//! A small, forgiving HTML parser producing a DOM tree.
//!
//! Supports what UI layouts need: elements with attributes (quoted, unquoted
//! or bare), text, comments, void/self-closing tags (`<br>`, `<img ...>`,
//! `<x/>`), `<style>` blocks (their text is collected as CSS), and the common
//! entities (`&amp; &lt; &gt; &quot; &#39; &nbsp;`). It never fails: unknown
//! or unbalanced markup is handled the way browsers roughly do (unclosed tags
//! close at their parent's end, stray end tags are ignored).
//!
//! Whitespace is collapsed like HTML (runs of spaces/newlines become one
//! space, whitespace-only text between tags is dropped); `<br>` is a line break.

/// A DOM node.
#[derive(Debug, Clone, PartialEq)]
pub enum DomNode {
    Element(Element),
    Text(String),
}

/// An element with its attributes and children.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Lower-case tag name.
    pub tag: String,
    /// Attributes in source order (names lower-cased).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<DomNode>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// Classes from the `class` attribute.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class").unwrap_or("").split_whitespace()
    }
}

/// A parsed document: the top-level nodes plus all `<style>` contents.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub roots: Vec<DomNode>,
    pub css: String,
}

const VOID: &[&str] = &["br", "img", "hr", "input", "meta", "link"];

/// Parses `html` into a [`Document`].
pub fn parse(html: &str) -> Document {
    let mut p = Parser { src: html, pos: 0, css: String::new(), open: Vec::new() };
    let roots = p.children();
    Document { roots, css: p.css }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    css: String,
    /// Tags of the currently open elements (innermost last).
    open: Vec<String>,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    /// Parses nodes until the innermost open element ends (its end tag is
    /// consumed), an ancestor's end tag appears (left for the ancestor, which
    /// implicitly closes this one), or the input ends.
    fn children(&mut self) -> Vec<DomNode> {
        let mut out = Vec::new();
        while self.pos < self.src.len() {
            let rest = self.rest();
            if rest.starts_with("<!--") {
                self.pos += rest.find("-->").map_or(rest.len(), |i| i + 3);
            } else if rest.starts_with("</") {
                let end = rest.find('>').map_or(rest.len(), |i| i + 1);
                let name = rest[2..end.saturating_sub(1)].trim().to_ascii_lowercase();
                if self.open.last() == Some(&name) {
                    self.pos += end;
                    return out;
                }
                if self.open.contains(&name) {
                    return out; // closes us implicitly; the ancestor consumes it
                }
                self.pos += end; // stray end tag: ignore
            } else if rest.starts_with("<!") || rest.starts_with("<?") {
                // Doctype / processing instruction.
                self.pos += rest.find('>').map_or(rest.len(), |i| i + 1);
            } else if rest.starts_with('<') && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                if let Some(el) = self.element() {
                    out.push(DomNode::Element(el));
                }
            } else {
                let end = rest[1..].find('<').map_or(rest.len(), |i| i + 1);
                let text = collapse(&decode_entities(&rest[..end]));
                self.pos += end;
                if !text.trim().is_empty() {
                    out.push(DomNode::Text(text));
                }
            }
        }
        out
    }

    /// Parses one element starting at `<`. Returns `None` for `<style>`
    /// (its text goes to `self.css`).
    fn element(&mut self) -> Option<Element> {
        self.pos += 1; // '<'
        let name_len = self.rest().find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(self.rest().len());
        let tag = self.rest()[..name_len].to_ascii_lowercase();
        self.pos += name_len;

        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            let rest = self.rest();
            if rest.is_empty() {
                break;
            }
            if rest.starts_with("/>") {
                self.pos += 2;
                self_closing = true;
                break;
            }
            if rest.starts_with('>') {
                self.pos += 1;
                break;
            }
            if rest.starts_with('/') {
                self.pos += 1;
                continue;
            }
            // Attribute name.
            let n = rest.find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/').unwrap_or(rest.len()).max(1);
            let name = rest[..n].to_ascii_lowercase();
            self.pos += n;
            self.skip_ws();
            let mut value = String::new();
            if self.rest().starts_with('=') {
                self.pos += 1;
                self.skip_ws();
                let rest = self.rest();
                if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let end = rest[1..].find(q).map_or(rest.len(), |i| i + 1);
                    value = decode_entities(&rest[1..end]);
                    self.pos += (end + 1).min(rest.len());
                } else {
                    let mut end = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
                    // `<img src=a.png/>`: the slash closes the tag, it isn't part of the value.
                    if rest[..end].ends_with('/') && rest[end..].starts_with('>') {
                        end -= 1;
                    }
                    value = decode_entities(&rest[..end]);
                    self.pos += end;
                }
            }
            attrs.push((name, value));
        }

        if tag == "style" {
            let rest = self.rest();
            let end = rest.to_ascii_lowercase().find("</style").unwrap_or(rest.len());
            self.css.push_str(&rest[..end]);
            self.css.push('\n');
            self.pos += end;
            self.pos += self.rest().find('>').map_or(self.rest().len(), |i| i + 1);
            return None;
        }

        let children = if self_closing || VOID.contains(&tag.as_str()) {
            Vec::new()
        } else {
            self.open.push(tag.clone());
            let c = self.children();
            self.open.pop();
            c
        };
        Some(Element { tag, attrs, children })
    }

    fn skip_ws(&mut self) {
        let n = self.rest().len() - self.rest().trim_start().len();
        self.pos += n;
    }
}

/// HTML whitespace collapsing: any run of whitespace becomes a single space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}

/// Decodes the handful of entities UI text needs.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(node: &DomNode) -> &Element {
        match node {
            DomNode::Element(e) => e,
            DomNode::Text(t) => panic!("expected element, got text {t:?}"),
        }
    }

    #[test]
    fn parses_nested_elements_attributes_and_text() {
        let doc = parse(
            r#"<!doctype html>
            <div id="hud" class="panel  top">
                <!-- comment -->
                <button data-command='end_turn' disabled>End &amp; go</button>
                <br>
                <img src=ui/x.png/>
                <p>HP {{ unit.hp }}</p>
            </div>"#,
        );
        assert_eq!(doc.roots.len(), 1);
        let div = el(&doc.roots[0]);
        assert_eq!(div.tag, "div");
        assert_eq!(div.id(), Some("hud"));
        assert_eq!(div.classes().collect::<Vec<_>>(), ["panel", "top"]);
        let tags: Vec<_> = div.children.iter().map(|c| el(c).tag.as_str()).collect();
        assert_eq!(tags, ["button", "br", "img", "p"]);
        let button = el(&div.children[0]);
        assert_eq!(button.attr("data-command"), Some("end_turn"));
        assert_eq!(button.attr("disabled"), Some(""));
        assert_eq!(button.children, vec![DomNode::Text("End & go".into())]);
        assert_eq!(el(&div.children[2]).attr("src"), Some("ui/x.png"));
    }

    #[test]
    fn style_blocks_become_css() {
        let doc = parse("<style>.a { color: red; }</style><div class=a>x</div><STYLE>#b{}</STYLE>");
        assert!(doc.css.contains(".a { color: red; }"));
        assert!(doc.css.contains("#b{}"));
        assert_eq!(doc.roots.len(), 1);
    }

    #[test]
    fn forgiving_with_broken_markup() {
        let doc = parse("<div><span>unclosed <b>bold</div></i>tail");
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[1], DomNode::Text("tail".into()));
    }

    #[test]
    fn whitespace_collapses() {
        let doc = parse("<p>  a \n\n   b  </p>");
        assert_eq!(el(&doc.roots[0]).children, vec![DomNode::Text(" a b ".into())]);
    }
}
