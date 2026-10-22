//! Translate owned UI formatting without evaluating translated format strings.
//! Captured values are inserted once and never interpreted as placeholders.
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, PartialEq)]
enum Part {
    Literal(String),
    Field,
}
fn parts(source: &str) -> Option<Vec<Part>> {
    let mut out = vec![];
    let mut literal = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                out.push(Part::Literal(std::mem::take(&mut literal)));
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    } else if c == '{' {
                        return None;
                    }
                }
                if !closed {
                    return None;
                }
                out.push(Part::Field);
            }
            '}' => return None,
            _ => literal.push(c),
        }
    }
    out.push(Part::Literal(literal));
    Some(out)
}
struct Template {
    key: String,
    parts: Vec<Part>,
}
impl Template {
    fn new(source: &str) -> Option<Self> {
        let parts = parts(source)?;
        let mut index = 0;
        let mut key = String::new();
        for part in &parts {
            match part {
                Part::Literal(s) => key.push_str(s),
                Part::Field => {
                    key.push_str(&format!("{{p{index}}}"));
                    index += 1;
                }
            }
        }
        if index == 0 {
            return None;
        }
        Some(Self { key, parts })
    }
    fn values<'a>(&self, text: &'a str) -> Option<Vec<&'a str>> {
        let mut cursor = 0;
        let mut values = vec![];
        for (index, part) in self.parts.iter().enumerate() {
            match part {
                Part::Literal(s) => {
                    if !text[cursor..].starts_with(s) {
                        return None;
                    }
                    cursor += s.len();
                }
                Part::Field => {
                    let next = match self.parts.get(index + 1) {
                        Some(Part::Literal(s)) => s,
                        _ => return None,
                    };
                    let length = if next.is_empty() {
                        text.len() - cursor
                    } else {
                        text[cursor..].find(next)?
                    };
                    values.push(&text[cursor..cursor + length]);
                    cursor += length;
                }
            }
        }
        (cursor == text.len()).then_some(values)
    }
}
fn fill(translation: &str, values: &[&str]) -> Option<String> {
    let mut output = String::new();
    let mut rest = translation;
    let mut seen = vec![false; values.len()];
    while let Some(start) = rest.find("{p") {
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let end = rest.find('}')?;
        let index: usize = rest[..end].parse().ok()?;
        let value = values.get(index)?;
        if *seen.get(index)? {
            return None;
        }
        seen[index] = true;
        output.push_str(value);
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    seen.into_iter().all(|s| s).then_some(output)
}
pub(super) fn translate(text: &str, map: &HashMap<String, String>) -> Option<String> {
    if text.len() > 16384 {
        return None;
    }
    static TEMPLATES: OnceLock<Vec<Template>> = OnceLock::new();
    for template in TEMPLATES.get_or_init(|| {
        let sources: Vec<String> =
            crate::json::from_json_str(include_str!("../../lang/ui-templates.json"))
                .unwrap_or_default();
        let mut templates: Vec<_> = sources.iter().filter_map(|s| Template::new(s)).collect();
        // Prefer the most specific prefix/suffix over broad status templates.
        templates.sort_by_key(|t| {
            std::cmp::Reverse(
                t.parts
                    .iter()
                    .map(|p| match p {
                        Part::Literal(s) => s.len(),
                        _ => 0,
                    })
                    .sum::<usize>(),
            )
        });
        templates
    }) {
        let Some(translated) = map.get(&template.key) else {
            continue;
        };
        if let Some(values) = template.values(text) {
            if let Some(value) = fill(translated, &values) {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_can_reorder_without_reinterpreting_paths_or_braces() {
        let template = Template::new("Before (type {}): {}").unwrap();
        let values = template
            .values("Before (type 1): C:\\Dev\\{p0}\\日本語")
            .unwrap();
        assert_eq!(
            fill("{p1} (Typ {p0})", &values).unwrap(),
            "C:\\Dev\\{p0}\\日本語 (Typ 1)"
        );
    }
    #[test]
    fn invalid_or_missing_placeholders_fail_closed() {
        assert!(fill("No field", &["value"]).is_none());
        assert!(fill("{p99}", &["value"]).is_none());
        assert!(fill("{p0} {p0}", &["value"]).is_none());
        assert!(Template::new("Bad { nested {").is_none());
    }
    #[test]
    fn unknown_text_and_changed_literals_are_not_matched() {
        let t = Template::new("Data Folder: {}").unwrap();
        assert_eq!(t.key, "Data Folder: {p0}");
        assert!(t.values("Unrelated Data Folder: x").is_none());
        assert_eq!(t.values("Data Folder: C:\\Dev").unwrap(), ["C:\\Dev"]);
    }
}
