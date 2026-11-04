use crate::chars::{is_blank, is_space, squeeze, trim, utf16_len};
use crate::model::{Kind, Node, Warnings, prose, segments_mut, visible};

type Test = fn(char) -> bool;

const HIDDEN: [(&str, Test); 4] = [
    ("zeroWidth", |char| char == '\u{200b}'),
    ("bom", |char| char == '\u{feff}'),
    ("softHyphen", |char| char == '\u{ad}'),
    (
        "bidi",
        |char| matches!(char, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'),
    ),
];

pub fn unicode_warnings(value: &str, warnings: &mut Warnings) {
    for (key, test) in HIDDEN {
        let total = value.chars().filter(|char| test(*char)).count();

        if total > 0 {
            warnings.add(key, total as u32);
        }
    }
}

pub fn clean_plain(value: &str) -> String {
    let mut spaced = String::with_capacity(value.len());

    for char in value.chars() {
        if !(char == ' ' && spaced.ends_with(' ')) {
            spaced.push(char);
        }
    }

    let lines: Vec<&str> = spaced.split('\n').map(|line| line.trim_end_matches([' ', '\t'])).collect();
    let mut result = String::with_capacity(spaced.len());
    let mut feeds = 0;

    for char in lines.join("\n").chars() {
        if char == '\n' {
            feeds += 1;

            if feeds > 2 {
                continue;
            }
        } else {
            feeds = 0;
        }

        result.push(char);
    }

    result
}

pub fn plain_tree(value: &str, warnings: &mut Warnings) -> Node {
    if value.starts_with('\n') || value.ends_with('\n') {
        warnings.count("edges");
    }

    let inner = value.trim_matches('\n');
    let mut children = Vec::new();

    for part in split_blank(inner) {
        if !is_blank(part) {
            children.push(Node::paragraph(prose(part)));
        }
    }

    Node::parent(Kind::Document, children)
}

fn split_blank(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'\n' && bytes.get(index + 1) == Some(&b'\n') {
            parts.push(&value[start..index]);

            while index < bytes.len() && bytes[index] == b'\n' {
                index += 1;
            }

            start = index;
        } else {
            index += 1;
        }
    }

    parts.push(&value[start..]);
    parts
}

pub fn link_title(value: &str) -> String {
    squeeze(value)
}

pub fn url(value: &str, warnings: &mut Warnings) -> Option<String> {
    let href = trim(value);

    if valid(href) {
        Some(href.to_owned())
    } else {
        warnings.count("links");
        None
    }
}

fn valid(href: &str) -> bool {
    let control = href.chars().any(|char| (char as u32) < 32 || char as u32 == 127);

    if href.is_empty() || utf16_len(href) > 2048 || control {
        return false;
    }

    let Some(colon) = scheme(href) else {
        return true;
    };

    match href[..colon].to_ascii_lowercase().as_str() {
        "http" | "https" => web(&href[colon + 1..]),
        "mailto" | "tel" => true,
        _ => false,
    }
}

fn scheme(href: &str) -> Option<usize> {
    let bytes = href.as_bytes();

    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }

    let end = bytes.iter().position(|byte| !(byte.is_ascii_alphanumeric() || b"+.-".contains(byte)))?;

    (bytes[end] == b':').then_some(end)
}

fn web(rest: &str) -> bool {
    let rest = rest.trim_start_matches(['/', '\\']);
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let mut authority = &rest[..end];

    if let Some(at) = authority.rfind('@') {
        if !matches!(&authority[..at], "" | ":") {
            return false;
        }

        authority = &authority[at + 1..];
    }

    let (host, port) = if authority.starts_with('[') {
        let Some(close) = authority.find(']') else {
            return false;
        };
        let after = &authority[close + 1..];

        if !after.is_empty() && !after.starts_with(':') {
            return false;
        }

        let inside = &authority[1..close];

        if inside.is_empty() || !inside.chars().all(|char| char.is_ascii_hexdigit() || char == ':' || char == '.') {
            return false;
        }

        (&authority[..=close], after.strip_prefix(':').unwrap_or(""))
    } else {
        match authority.rfind(':') {
            Some(colon) => (&authority[..colon], &authority[colon + 1..]),
            None => (authority, ""),
        }
    };

    let port_fits = port.is_empty()
        || (port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u32>().is_ok_and(|port| port <= 65535));

    !host.is_empty() && port_fits && (host.starts_with('[') || domain(host))
}

fn domain(host: &str) -> bool {
    let forbidden = host.chars().any(|char| {
        is_space(char) || matches!(char, '#' | '/' | ':' | '<' | '>' | '?' | '@' | '[' | '\\' | ']' | '^' | '|')
    });
    let bytes = host.as_bytes();
    let escapes = bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'%'
            || (bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
    });

    if forbidden || !escapes {
        return false;
    }

    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    let numeric = |label: &str| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_digit());

    if !labels.last().is_some_and(|label| numeric(label)) {
        return true;
    }

    let count = labels.len();

    count <= 4
        && labels.iter().enumerate().all(|(index, label)| {
            let Ok(value) = label.parse::<u64>() else {
                return false;
            };
            let bound = if index + 1 == count { 256_u64.pow(5 - count as u32) } else { 256 };

            numeric(label) && value < bound
        })
}

pub fn address(node: &Node) -> bool {
    let value = visible(node);
    let candidates = [value.clone(), format!("http://{value}"), format!("mailto:{value}")];

    candidates.iter().any(|candidate| url(candidate, &mut Warnings::default()).is_some_and(|href| href == node.href))
}

pub fn unlink_addresses(tree: &mut Node) {
    if !tree.kind.is_container() {
        return;
    }

    let children = std::mem::take(&mut tree.children);

    for mut child in children {
        unlink_addresses(&mut child);

        if child.is(Kind::Link) && address(&child) {
            tree.children.append(&mut child.children);
        } else {
            tree.children.push(child);
        }
    }
}

fn flow_space(char: char) -> bool {
    matches!(char, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

fn clean_flow(children: &mut [Node]) {
    let mut flow = Vec::new();
    segments_mut(children, &mut flow);

    let mut built: Vec<Option<Vec<Option<char>>>> = vec![None; flow.len()];
    let mut pending: Vec<(usize, usize)> = Vec::new();
    let mut previous_space = true;

    let trim_pending = |built: &mut Vec<Option<Vec<Option<char>>>>, pending: &mut Vec<(usize, usize)>| {
        for (part, position) in pending.drain(..) {
            if let Some(chars) = built[part].as_mut() {
                chars[position] = None;
            }
        }
    };

    for (index, part) in flow.iter().enumerate() {
        match part.kind {
            Kind::InlineCode => {
                pending.clear();
                previous_space = false;
            }
            Kind::Break => {
                trim_pending(&mut built, &mut pending);
                previous_space = true;
            }
            _ => {
                let mut chars = Vec::new();

                for char in part.value.chars().map(|char| if flow_space(char) { ' ' } else { char }) {
                    if char == ' ' && previous_space {
                        continue;
                    }

                    if char == ' ' {
                        pending.push((index, chars.len()));
                    } else {
                        pending.clear();
                    }

                    chars.push(Some(char));
                    previous_space = char == ' ';
                }

                built[index] = Some(chars);
            }
        }
    }

    trim_pending(&mut built, &mut pending);

    for (part, chars) in flow.iter_mut().zip(built) {
        if let Some(chars) = chars {
            part.value = chars.into_iter().flatten().collect();
        }
    }

    for reversed in [false, true] {
        let order: Vec<usize> = if reversed { (0..flow.len()).rev().collect() } else { (0..flow.len()).collect() };

        for index in order {
            let part = &mut *flow[index];

            if part.is(Kind::Text) && part.value.is_empty() {
                continue;
            }

            if !part.is(Kind::Break) {
                break;
            }

            part.become_text("");
        }
    }
}

fn paragraphs(paragraph: Node) -> Vec<Node> {
    let mut parts: Vec<Vec<Node>> = vec![Vec::new()];
    let mut breaks = Vec::new();

    for child in paragraph.children {
        if child.is(Kind::Break) {
            breaks.push(child);
        } else if !child.is(Kind::Text) || !child.value.is_empty() {
            if breaks.len() > 1 {
                parts.push(Vec::new());
                breaks.clear();
            } else if let Some(last) = parts.last_mut() {
                last.append(&mut breaks);
            }

            if let Some(last) = parts.last_mut() {
                last.push(child);
            }
        }
    }

    parts.into_iter().filter(|children| !children.is_empty()).map(Node::paragraph).collect()
}

pub fn clean_tree(tree: &mut Node) {
    if !tree.kind.is_container() {
        return;
    }

    for child in &mut tree.children {
        if matches!(child.kind, Kind::Paragraph | Kind::Heading | Kind::Cell) {
            clean_flow(&mut child.children);
        } else if !child.is(Kind::CodeBlock) {
            clean_tree(child);
        }
    }

    let children = std::mem::take(&mut tree.children);

    for child in children {
        if child.is(Kind::Paragraph) {
            tree.children.extend(paragraphs(child));
        } else {
            tree.children.push(child);
        }
    }
}

fn trim_flow(flow: &mut [&mut Node]) {
    for reversed in [false, true] {
        let order: Vec<usize> = if reversed { (0..flow.len()).rev().collect() } else { (0..flow.len()).collect() };
        let mut edge = true;
        let mut outer = true;

        for index in order {
            let part = &mut *flow[index];

            match part.kind {
                Kind::Break => {
                    if outer {
                        part.become_text("");
                    }

                    edge = true;
                }
                Kind::InlineCode => {
                    edge = false;
                    outer = false;
                }
                _ => {
                    if edge {
                        if reversed {
                            let kept = part.value.trim_end_matches([' ', '\t']).len();

                            part.value.truncate(kept);
                        } else {
                            let cut = part.value.len() - part.value.trim_start_matches([' ', '\t']).len();

                            part.value.drain(..cut);
                        }

                        edge = part.value.is_empty();
                    }

                    outer = outer && part.value.is_empty();
                }
            }
        }
    }
}

pub fn trim_lines(tree: &mut Node) {
    for child in &mut tree.children {
        if matches!(child.kind, Kind::Paragraph | Kind::Heading | Kind::Cell) {
            let mut flow = Vec::new();
            segments_mut(&mut child.children, &mut flow);
            trim_flow(&mut flow);
        } else if !child.is(Kind::CodeBlock) {
            trim_lines(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        assert_eq!(clean_plain("a  b \n\n\n\nc\t"), "a b\n\nc");
        assert_eq!(clean_plain("\n\n\na"), "\n\na");
        assert_eq!(clean_plain("a\n\n\n"), "a\n\n");
        assert_eq!(clean_plain("a\n"), "a\n");
    }

    #[test]
    fn addresses() {
        let mut warnings = Warnings::default();

        assert!(url("https://example.com/a b", &mut warnings).is_some());
        assert!(url("/relative", &mut warnings).is_some());
        assert!(url("mailto:a@b.c", &mut warnings).is_some());
        assert!(url("https://[::1]:8080/", &mut warnings).is_some());
        assert!(url("javascript:alert(1)", &mut warnings).is_none());
        assert!(url("https://user@host", &mut warnings).is_none());
        assert!(url("https://", &mut warnings).is_none());
        assert!(url("http://1.2.3.999", &mut warnings).is_none());
        assert!(url("http://a b.com", &mut warnings).is_none());
        assert!(warnings.has("links"));
    }
}
