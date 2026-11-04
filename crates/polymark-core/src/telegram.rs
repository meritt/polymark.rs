use std::fmt::Write;

use crate::chars::{is_space, utf16_len};
use crate::model::{Failure, Kind, Node, Result, Warnings};
use crate::write::{Format, serialize};

const LIMIT: usize = 4096;

fn escape(value: &str, special: &[char]) -> String {
    let mut result = String::with_capacity(value.len());

    for char in value.chars() {
        if special.contains(&char) {
            result.push('\\');
        }

        result.push(char);
    }

    result
}

fn escape_text(value: &str) -> String {
    escape(value, &['_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!', '\\'])
}

fn escape_code(value: &str) -> String {
    escape(value, &['`', '\\'])
}

fn web(href: &str) -> bool {
    ["http://", "https://"]
        .iter()
        .any(|scheme| href.get(..scheme.len()).is_some_and(|start| start.eq_ignore_ascii_case(scheme)))
}

fn terminator(char: char) -> bool {
    matches!(char, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn indented(text: &str, indent: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(char) = chars.next() {
        result.push(char);

        if char == '\n' && chars.peek().is_some_and(|next| !terminator(*next)) {
            result.push_str(indent);
        }
    }

    result
}

fn encode_spaces(href: &str) -> String {
    let mut result = String::with_capacity(href.len());

    for char in href.chars() {
        if is_space(char) {
            let mut buffer = [0; 4];

            for byte in char.encode_utf8(&mut buffer).bytes() {
                let _ = write!(result, "%{byte:02X}");
            }
        } else {
            result.push(char);
        }
    }

    result
}

fn inline(node: &Node, active: &[Kind]) -> Result<String> {
    match node.kind {
        Kind::Text => return Ok(escape_text(&node.value)),
        Kind::Break => return Ok("\n".into()),
        Kind::InlineCode => {
            return Ok(if active.contains(&Kind::Link) {
                escape_text(&node.value)
            } else {
                format!("`{}`", escape_code(&node.value))
            });
        }
        _ => {}
    }

    let nested: Vec<Kind> = active.iter().copied().chain([node.kind]).collect();
    let content = line(&node.children, &nested)?;

    if node.is(Kind::Link) {
        return Ok(if web(&node.href) {
            format!("[{content}]({})", escape(&encode_spaces(&node.href), &[')', '\\']))
        } else {
            content
        });
    }

    let marker = match node.kind {
        Kind::Strong => '*',
        Kind::Emphasis => '_',
        Kind::Strike => '~',
        _ => return Err(Failure::Unsupported),
    };

    Ok(if active.contains(&node.kind) || content.is_empty() { content } else { format!("{marker}{content}{marker}") })
}

fn line(children: &[Node], active: &[Kind]) -> Result<String> {
    children.iter().map(|child| inline(child, active)).collect()
}

fn grid(node: &Node) -> Result<String> {
    let mut rows: Vec<Vec<Vec<char>>> = Vec::new();

    for row in &node.children {
        let mut cells = Vec::new();

        for cell in &row.children {
            let paragraph = Node::paragraph(cell.children.clone());
            let value = serialize(&paragraph, Format::Plain, &mut Warnings::default(), false)?;

            cells.push(value.chars().map(|char| if matches!(char, '\t' | '\n' | '\r') { ' ' } else { char }).collect());
        }

        rows.push(cells);
    }

    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|column| rows.iter().map(|cells| cells.get(column).map_or(0, Vec::len)).max().unwrap_or(0))
        .collect();
    let mut lines: Vec<String> = rows
        .iter()
        .map(|cells| {
            let padded: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(column, chars)| {
                    let text: String = chars.iter().collect();

                    format!("{text}{}", " ".repeat(widths[column] - chars.len()))
                })
                .collect();

            padded.join("  ").trim_end_matches(is_space).to_owned()
        })
        .collect();

    if node.children.first().is_some_and(|row| row.children.iter().all(|cell| cell.header)) {
        let rule: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();

        lines.insert(1.min(lines.len()), rule.join("  "));
    }

    Ok(lines.join("\n"))
}

fn task(item: &Node) -> &'static str {
    match item.checked {
        None => "",
        Some(true) => "\\[x\\] ",
        Some(false) => "\\[ \\] ",
    }
}

fn flush(node: &Node) -> bool {
    matches!(node.kind, Kind::Quote | Kind::CodeBlock | Kind::Table)
}

fn list(node: &Node, quoted: bool, indent: &str) -> Result<String> {
    let mut items = Vec::new();

    for (index, item) in node.children.iter().enumerate() {
        let number = (node.start as usize + index).to_string();
        let marker =
            format!("{}{}", if node.ordered { format!("{number}\\. ") } else { "\\- ".to_owned() }, task(item));
        let padding = format!("{indent}{}", " ".repeat(if node.ordered { number.len() + 2 } else { 2 }));
        let mut parts = Vec::new();

        for (position, child) in item.children.iter().enumerate() {
            let text = block(child, quoted, &padding)?;

            parts.push(if !quoted && flush(child) {
                if position > 0 { text } else { format!("{}\n{text}", marker.trim_end()) }
            } else {
                format!("{}{text}", if position > 0 { &padding } else { &marker })
            });
        }

        items.push(if parts.is_empty() { marker } else { parts.join("\n") });
    }

    Ok(items.join(&format!("\n{indent}")))
}

fn code(value: &str, language: &str, quoted: bool, indent: &str) -> String {
    if !quoted {
        return format!("```{language}\n{}\n```", escape_code(value));
    }

    let lines: Vec<String> = value
        .strip_suffix('\n')
        .unwrap_or(value)
        .split('\n')
        .map(|text| if text.is_empty() { String::new() } else { format!("`{}`", escape_code(text)) })
        .collect();

    indented(&lines.join("\n"), indent)
}

fn quote(node: &Node) -> Result<String> {
    let parts = node.children.iter().map(|child| block(child, true, "")).collect::<Result<Vec<_>>>()?;

    Ok(parts.join("\n\n").split('\n').map(|text| format!(">{text}")).collect::<Vec<_>>().join("\n"))
}

fn block(node: &Node, quoted: bool, indent: &str) -> Result<String> {
    match node.kind {
        Kind::Document => {
            let parts = node.children.iter().map(|child| block(child, quoted, indent)).collect::<Result<Vec<_>>>()?;

            Ok(parts.join("\n\n"))
        }
        Kind::Paragraph => Ok(indented(&line(&node.children, &[])?, indent)),
        Kind::Heading => {
            let content = line(&node.children, &[Kind::Strong])?;

            Ok(if content.is_empty() { String::new() } else { indented(&format!("*{content}*"), indent) })
        }
        Kind::ThematicBreak => Ok("———".into()),
        Kind::Quote if !quoted => quote(node),
        Kind::Quote => {
            let parts = node.children.iter().map(|child| block(child, true, indent)).collect::<Result<Vec<_>>>()?;

            Ok(parts.join(&format!("\n\n{indent}")))
        }
        Kind::List => list(node, quoted, indent),
        Kind::Table => Ok(code(&grid(node)?, "", quoted, indent)),
        Kind::CodeBlock => Ok(code(&node.value, &node.language, quoted, indent)),
        _ => Err(Failure::Unsupported),
    }
}

fn inspect(node: &Node, warnings: &mut Warnings, ancestors: &mut Vec<Kind>) {
    let nestings = [
        (Kind::Quote, Kind::Quote, "quotes"),
        (Kind::Quote, Kind::CodeBlock, "quotedCode"),
        (Kind::Link, Kind::InlineCode, "linkCode"),
    ];

    for (ancestor, kind, key) in nestings {
        if node.kind == kind && ancestors.contains(&ancestor) {
            warnings.count(key);
        }
    }

    if node.is(Kind::Table) {
        warnings.count("tableCode");
    }

    if node.is(Kind::Link) && !web(&node.href) {
        warnings.count("addresses");
    }

    ancestors.push(node.kind);

    for child in &node.children {
        inspect(child, warnings, ancestors);
    }

    ancestors.pop();
}

fn unescape(chars: &[char]) -> String {
    let mut result = String::with_capacity(chars.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '\\' && chars.get(index + 1).is_some_and(|next| !terminator(*next)) {
            index += 1;
        }

        result.push(chars[index]);
        index += 1;
    }

    result
}

fn fenced(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    if chars.get(start..start + 3)? != ['`', '`', '`'] {
        return None;
    }

    let feed = start + 3 + chars[start + 3..].iter().position(|char| *char == '\n')?;
    let content = feed + 1;
    let close = (content..chars.len()).find(|at| chars.get(*at..*at + 4) == Some(&['\n', '`', '`', '`'][..]))?;

    Some((content, close, close + 4))
}

fn enclosed(chars: &[char], start: usize, open: &[char], close: char) -> Option<(usize, usize, usize)> {
    if chars.get(start..start + open.len())? != open {
        return None;
    }

    let content = start + open.len();
    let mut index = content;

    loop {
        match chars.get(index) {
            Some('\\') if chars.get(index + 1).is_some_and(|next| !terminator(*next)) => index += 2,
            Some(char) if *char != close && *char != '\\' => index += 1,
            _ => break,
        }
    }

    (chars.get(index) == Some(&close)).then_some((content, index, index + 1))
}

fn shown(result: &str) -> String {
    let chars: Vec<char> = result.chars().collect();
    let mut out = String::with_capacity(result.len());
    let mut index = 0;

    while index < chars.len() {
        if let Some((start, end, next)) = fenced(&chars, index).or_else(|| enclosed(&chars, index, &['`'], '`')) {
            out.push_str(&unescape(&chars[start..end]));
            index = next;
            continue;
        }

        if let Some((_, _, next)) = enclosed(&chars, index, &[']', '('], ')') {
            index = next;
            continue;
        }

        let char = chars[index];

        if char == '\\' && chars.get(index + 1).is_some_and(|next| !terminator(*next)) {
            out.push(chars[index + 1]);
            index += 2;
            continue;
        }

        let line_start = index == 0 || terminator(chars[index - 1]);

        if !(matches!(char, '*' | '_' | '~' | '[') || (char == '>' && line_start)) {
            out.push(char);
        }

        index += 1;
    }

    out
}

pub fn telegram(tree: &Node, warnings: &mut Warnings) -> Result<String> {
    inspect(tree, warnings, &mut Vec::new());

    let result = block(tree, false, "")?;

    if utf16_len(&shown(&result)) > LIMIT {
        warnings.count("long");
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_what_a_reader_sees() {
        assert_eq!(shown("*a* \\. `x\\`y` [b](https://e.com)\n>q"), "a . x`y b\nq");
        assert_eq!(shown("```js\na\\`\n```"), "a`");
    }

    #[test]
    fn writes_messages() {
        let tree = Node::parent(
            Kind::Document,
            vec![Node::paragraph(vec![Node::parent(Kind::Strong, vec![Node::text("a.b")])])],
        );

        assert_eq!(telegram(&tree, &mut Warnings::default()).unwrap(), "*a\\.b*");
    }
}
