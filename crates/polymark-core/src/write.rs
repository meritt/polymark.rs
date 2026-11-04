use std::collections::HashSet;
use std::fmt::Write;

use crate::chars::{Kind as Class, is_space, kind};
use crate::clean::address;
use crate::model::{Failure, Kind, Node, Result, Warnings, segments, visible};
use crate::normalize::marker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Plain,
    Markdown,
    Html,
}

pub fn escape_html(value: &str) -> String {
    let mut result = String::with_capacity(value.len());

    for char in value.chars() {
        match char {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            _ => result.push(char),
        }
    }

    result
}

fn attribute(value: &str) -> String {
    escape_html(value).replace('"', "&quot;").replace('\'', "&#39;")
}

fn longest_run(value: &str, char: char) -> usize {
    let mut longest = 0;
    let mut current = 0;

    for next in value.chars() {
        current = if next == char { current + 1 } else { 0 };
        longest = longest.max(current);
    }

    longest
}

fn fence(value: &str) -> String {
    "`".repeat(3.max(longest_run(value, '`') + 1))
}

fn code_span(value: &str) -> Result<String> {
    if value.contains(['\r', '\n']) {
        return Err(Failure::Representation);
    }

    let delimiter = "`".repeat(1.max(longest_run(value, '`') + 1));
    let ticks = value.starts_with('`') || value.ends_with('`');
    let edges = (value.starts_with(' ') || value.ends_with(' ')) && value.chars().any(|char| char != ' ');
    let pad = if ticks || edges { " " } else { "" };

    Ok(format!("{delimiter}{pad}{value}{pad}{delimiter}"))
}

fn markup(char: char) -> bool {
    matches!(char, '\\' | '*' | '_' | '`' | '[' | ']' | '~')
}

fn ascii_punctuation(char: char) -> bool {
    char.is_ascii_punctuation()
}

fn word(char: char) -> bool {
    char.is_ascii_alphanumeric() || char == '_'
}

fn ends_with_ignore_case(chars: &[char], suffix: &str) -> bool {
    let suffix: Vec<char> = suffix.chars().collect();

    chars.len() >= suffix.len()
        && chars[chars.len() - suffix.len()..].iter().zip(&suffix).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn starts_with_ignore_case(chars: &[char], prefix: &str) -> bool {
    let prefix: Vec<char> = prefix.chars().collect();

    chars.len() >= prefix.len() && chars[..prefix.len()].iter().zip(&prefix).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn email_rest(rest: &[char]) -> bool {
    if rest.first() != Some(&'@') {
        return false;
    }

    let label = rest[1..].iter().take_while(|char| word(**char) || **char == '-').count();

    if label == 0 || rest.get(1 + label) != Some(&'.') {
        return false;
    }

    rest[2 + label..].iter().take_while(|char| word(**char) || **char == '-').any(char::is_ascii_alphanumeric)
}

fn autolinked(before: &[char], char: char, rest: &[char]) -> bool {
    match char {
        ':' => {
            starts_with_ignore_case(rest, "://")
                && ["http", "https", "ftp"].iter().any(|scheme| ends_with_ignore_case(before, scheme))
        }
        '.' => ends_with_ignore_case(before, "www"),
        '@' => before.last().is_some_and(|last| word(*last) || matches!(last, '.' | '+' | '-')) && email_rest(rest),
        _ => false,
    }
}

fn entity_at(chars: &[char]) -> bool {
    let window = &chars[..chars.len().min(40)];

    if window.first() != Some(&'&') {
        return false;
    }

    let body = &window[1..];
    let length = if body.first() == Some(&'#') {
        let hex = body.get(1).is_some_and(|char| *char == 'x' || *char == 'X');
        let digits = &body[if hex { 2 } else { 1 }..];
        let count =
            digits.iter().take_while(|char| if hex { char.is_ascii_hexdigit() } else { char.is_ascii_digit() }).count();

        if count == 0 {
            return false;
        }

        (if hex { 2 } else { 1 }) + count
    } else {
        if !body.first().is_some_and(char::is_ascii_alphabetic) {
            return false;
        }

        body.iter().take_while(|char| char.is_ascii_alphanumeric()).count()
    };

    body.get(length) == Some(&';')
}

fn escape_markdown(value: &[char], index: usize, before_link: bool, strict: bool) -> String {
    let char = value[index];
    let next = value.get(index + 1).copied();

    if char == '\\' {
        return if strict || next.is_none_or(ascii_punctuation) { "\\\\".into() } else { "\\".into() };
    }

    if char == '`' || (strict && markup(char)) {
        return format!("\\{char}");
    }

    if char == '<' && next.is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?')) {
        return "&lt;".into();
    }

    if char == '&' && entity_at(&value[index..]) {
        return "&amp;".into();
    }

    if char == '!' && next.is_none() && before_link {
        return "\\!".into();
    }

    let before = &value[index.saturating_sub(5)..index];
    let rest = &value[index..value.len().min(index + 256)];

    if strict && autolinked(before, char, rest) {
        return format!("\\{char}");
    }

    char.to_string()
}

fn line_marker(line: &[char]) -> Option<usize> {
    let hashes = line.iter().take_while(|char| **char == '#').count();
    let bullet = matches!(line.first(), Some('-' | '+' | '*'));
    let opener = if bullet { 1 } else { hashes };
    let block = (bullet || (1..=6).contains(&hashes)) && line.get(opener).is_none_or(|char| matches!(char, ' ' | '\t'));
    let rule = line.first().is_some_and(|char| matches!(char, '-' | '=' | '*' | '_'))
        && line.iter().all(|char| matches!(char, '-' | '=' | '*' | '_' | ' ' | '\t'));
    let definition = line.first() == Some(&'[') && {
        let close = line.iter().position(|char| *char == ']');

        close.is_some_and(|close| line.get(close + 1) == Some(&':'))
    };

    if block || rule || definition || line.first() == Some(&'>') {
        return Some(0);
    }

    if delimiter_row(line) && line.iter().any(|char| matches!(char, '|' | ':')) {
        return line.iter().position(|char| *char == '-');
    }

    let digits = line.iter().take_while(|char| char.is_ascii_digit()).count();
    let ordered = (1..=9).contains(&digits)
        && matches!(line.get(digits), Some('.' | ')'))
        && line.get(digits + 1).is_none_or(|char| matches!(char, ' ' | '\t'));

    ordered.then_some(digits)
}

fn delimiter_cell(line: &[char], mut index: usize) -> Option<usize> {
    while matches!(line.get(index), Some(' ' | '\t')) {
        index += 1;
    }

    if line.get(index) == Some(&':') {
        index += 1;
    }

    let dashes = line[index.min(line.len())..].iter().take_while(|char| **char == '-').count();

    if dashes == 0 {
        return None;
    }

    index += dashes;

    if line.get(index) == Some(&':') {
        index += 1;
    }

    while matches!(line.get(index), Some(' ' | '\t')) {
        index += 1;
    }

    Some(index)
}

fn delimiter_row(line: &[char]) -> bool {
    let start = usize::from(line.first() == Some(&'|'));
    let Some(mut index) = delimiter_cell(line, start) else {
        return false;
    };

    while line.get(index) == Some(&'|') {
        match delimiter_cell(line, index + 1) {
            Some(next) => index = next,
            None => break,
        }
    }

    if line.get(index) == Some(&'|') {
        index += 1;
    }

    while matches!(line.get(index), Some(' ' | '\t')) {
        index += 1;
    }

    index == line.len()
}

struct Run {
    item: usize,
    start: usize,
    end: usize,
    open: bool,
    close: bool,
}

fn runs(item: usize, value: &[char], sign: char) -> Vec<Run> {
    let mut result = Vec::new();
    let mut index = 0;

    while index < value.len() {
        if value[index] != sign {
            index += 1;
            continue;
        }

        let start = index;

        while value.get(index) == Some(&sign) {
            index += 1;
        }

        let before = kind(start.checked_sub(1).map(|at| value[at]));
        let after = kind(value.get(index).copied());
        let left = after != Class::Space && (after != Class::Punctuation || before != Class::Word);
        let right = before != Class::Space && (before != Class::Punctuation || after != Class::Word);
        let strong = sign != '_';

        result.push(Run {
            item,
            start,
            end: index,
            open: left && (strong || !right || before == Class::Punctuation),
            close: right && (strong || !left || after == Class::Punctuation),
        });
    }

    result
}

fn pairable(texts: &[(usize, &[char])], sign: char, marked: bool) -> Vec<Run> {
    let found: Vec<Run> = texts.iter().flat_map(|(item, value)| runs(*item, value, sign)).collect();
    let first = found.iter().position(|run| run.open);
    let closed = first.is_some_and(|first| found[first + 1..].iter().any(|run| run.close));

    if marked || closed { found.into_iter().filter(|run| run.open || run.close).collect() } else { Vec::new() }
}

fn web_start(value: &[char], index: usize) -> Option<usize> {
    ["https://", "http://", "ftp://", "www."]
        .into_iter()
        .find(|prefix| starts_with_ignore_case(&value[index..], prefix))
        .map(str::len)
}

fn escape_addresses(value: &[char], escaped: &mut [String], before_link: bool, closed: bool) {
    let mut index = 0;

    while index < value.len() {
        let Some(prefix) = web_start(value, index) else {
            index += 1;
            continue;
        };

        let start = index;
        let mut end = index + prefix;

        while end < value.len() && !is_space(value[end]) {
            end += 1;
        }

        let original: String = value[start..end].iter().collect();
        let touching = end == value.len() && !closed;
        let changed = escaped[start..end].concat() != original;

        if touching || changed || original.contains('>') {
            for (position, slot) in escaped.iter_mut().enumerate().take(end).skip(start) {
                *slot = escape_markdown(value, position, before_link, true);
            }
        }

        index = end;
    }
}

fn segment_count(node: &Node) -> usize {
    if node.kind.is_segment() { 1 } else { node.children.iter().map(segment_count).sum() }
}

#[derive(Default)]
struct Shape {
    kinds: HashSet<Kind>,
    underscore: bool,
    link_starts: HashSet<usize>,
    linked: HashSet<usize>,
    direct: HashSet<usize>,
}

fn survey(parent: Kind, nodes: &[Node], cursor: &mut usize, in_link: bool, top: bool, shape: &mut Shape) {
    for (index, node) in nodes.iter().enumerate() {
        shape.kinds.insert(node.kind);

        if node.kind.is_segment() {
            if node.is(Kind::Text) && in_link {
                shape.linked.insert(*cursor);
            }

            if top {
                shape.direct.insert(*cursor);
            }

            *cursor += 1;
            continue;
        }

        if node.kind.is_container() && marker(parent, nodes, index) == Some('_') {
            shape.underscore = true;
        }

        if node.is(Kind::Link) && segment_count(node) > 0 {
            shape.link_starts.insert(*cursor);
        }

        survey(node.kind, &node.children, cursor, in_link || node.is(Kind::Link), false, shape);
    }
}

struct Prose<'a> {
    flow: Vec<&'a Node>,
    escaped: Vec<Vec<String>>,
    repeated: HashSet<usize>,
}

impl Prose<'_> {
    fn standalone(&self, first: usize, count: usize) -> bool {
        let (before, after) = if count == 0 {
            (None, self.flow.first())
        } else {
            (first.checked_sub(1).and_then(|at| self.flow.get(at)), self.flow.get(first + count))
        };
        let opens = before.is_none_or(|before| {
            before.is(Kind::Break)
                || (before.is(Kind::Text)
                    && before.value.chars().next_back().is_none_or(|last| is_space(last) || last == '('))
        });
        let closes = after.is_none_or(|after| {
            let rest = after.value.strip_prefix(')').unwrap_or(&after.value);

            after.is(Kind::Break) || (after.is(Kind::Text) && rest.chars().next().is_none_or(is_space))
        });

        opens && closes
    }

    fn render(&mut self, parent: Kind, siblings: &[Node], index: usize, cursor: &mut usize) -> Result<String> {
        let node = &siblings[index];

        if node.kind.is_segment() {
            let at = *cursor;
            *cursor += 1;

            if node.is(Kind::Text) {
                return Ok(std::mem::take(&mut self.escaped[at]).concat());
            }

            if self.repeated.contains(&at) {
                return Ok("\\\n".into());
            }

            return inline(node, Format::Markdown);
        }

        let first = *cursor;
        let mut content = String::new();

        for position in 0..node.children.len() {
            content.push_str(&self.render(node.kind, &node.children, position, cursor)?);
        }

        if node.is(Kind::Link) {
            return Ok(markdown_link(node, &content, self.standalone(first, *cursor - first)));
        }

        let sign = marker(parent, siblings, index).ok_or(Failure::Unsupported)?;
        let run = if node.is(Kind::Emphasis) { sign.to_string() } else { sign.to_string().repeat(2) };

        Ok(format!("{run}{content}{run}"))
    }
}

fn protected_prose(children: &[Node], line_start: bool, strict: bool) -> Result<String> {
    let mut flow = Vec::new();
    segments(children, &mut flow);

    let mut shape = Shape::default();
    survey(Kind::Paragraph, children, &mut 0, false, true, &mut shape);

    let values: Vec<Vec<char>> = flow.iter().map(|node| node.value.chars().collect()).collect();
    let mut escaped: Vec<Vec<String>> = values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            if !flow[position].is(Kind::Text) {
                return Vec::new();
            }

            let before_link = shape.link_starts.contains(&(position + 1));

            (0..value.len()).map(|index| escape_markdown(value, index, before_link, strict)).collect()
        })
        .collect();

    escape_brackets(&flow, &values, &mut escaped, &shape.linked, strict);

    if !strict {
        let texts: Vec<(usize, &[char])> = flow
            .iter()
            .enumerate()
            .filter(|(_, node)| node.is(Kind::Text))
            .map(|(position, _)| (position, values[position].as_slice()))
            .collect();
        let present = |kinds: &[Kind]| kinds.iter().any(|kind| shape.kinds.contains(kind));

        for (sign, marked) in
            [('*', present(&[Kind::Strong, Kind::Emphasis])), ('_', shape.underscore), ('~', present(&[Kind::Strike]))]
        {
            for run in pairable(&texts, sign, marked) {
                for slot in &mut escaped[run.item][run.start..run.end] {
                    *slot = format!("\\{sign}");
                }
            }
        }

        for position in 0..flow.len() {
            if !flow[position].is(Kind::Text) {
                continue;
            }

            let next = flow.get(position + 1);
            let closed = shape.direct.contains(&position) && next.is_none_or(|next| next.is(Kind::Break));
            let before_link = shape.link_starts.contains(&(position + 1));

            escape_addresses(&values[position], &mut escaped[position], before_link, closed);
        }
    }

    let mut at_line = line_start;

    for (position, node) in flow.iter().enumerate() {
        match node.kind {
            Kind::Break => at_line = true,
            Kind::InlineCode => at_line = false,
            _ => {
                let value = &values[position];

                for index in 0..value.len() {
                    let char = value[index];

                    if at_line && !matches!(char, ' ' | '\t' | '\n') {
                        let end =
                            value[index..].iter().position(|char| *char == '\n').map_or(value.len(), |at| index + at);

                        if let Some(offset) = line_marker(&value[index..end]) {
                            escaped[position][index + offset] = format!("\\{}", value[index + offset]);
                        }

                        at_line = false;
                    }

                    if char == '\n' {
                        at_line = true;
                    }
                }
            }
        }
    }

    let shown: Vec<usize> =
        (0..flow.len()).filter(|position| !flow[*position].is(Kind::Text) || !values[*position].is_empty()).collect();
    let repeated = shown
        .windows(2)
        .filter(|pair| flow[pair[0]].is(Kind::Break) && flow[pair[1]].is(Kind::Break))
        .map(|pair| pair[1])
        .collect();

    let mut prose = Prose { flow, escaped, repeated };
    let mut cursor = 0;
    let mut result = String::new();

    for index in 0..children.len() {
        result.push_str(&prose.render(Kind::Paragraph, children, index, &mut cursor)?);
    }

    Ok(result)
}

fn escape_brackets(
    flow: &[&Node],
    values: &[Vec<char>],
    escaped: &mut [Vec<String>],
    linked: &HashSet<usize>,
    strict: bool,
) {
    let mut joined: Vec<char> = Vec::new();
    let mut offsets = Vec::new();

    for (position, node) in flow.iter().enumerate() {
        match node.kind {
            Kind::Text => {
                offsets.push((position, joined.len()));
                joined.extend(&values[position]);
            }
            Kind::Break => joined.push('\n'),
            _ => joined.push('\0'),
        }
    }

    let opener = joined.windows(2).rposition(|pair| pair[0] == ']' && matches!(pair[1], '(' | '['));

    for (position, offset) in offsets {
        let inside = strict || linked.contains(&position);

        for (index, char) in values[position].iter().enumerate() {
            let opens = *char == '[' && (inside || opener.is_some_and(|opener| offset + index < opener));

            if opens || (*char == ']' && inside) {
                escaped[position][index] = format!("\\{char}");
            }
        }
    }
}

fn markdown_link(node: &Node, content: &str, alone: bool) -> String {
    let href = &node.href;
    let scheme = ["http://", "https://"]
        .into_iter()
        .find(|scheme| href.get(..scheme.len()).is_some_and(|start| start.eq_ignore_ascii_case(scheme)));
    let bare = scheme.is_some_and(|scheme| href.len() > scheme.len())
        && !href.chars().any(|char| is_space(char) || matches!(char, '<' | '>'))
        && !href.ends_with(['.', ',', ':', ';', '!', '?', '\'', '"', ')', ']', '*', '_', '~']);

    if alone && node.title.is_empty() && bare && visible(node) == *href {
        return href.clone();
    }

    let mut destination = String::with_capacity(href.len());
    let chars: Vec<char> = href.chars().collect();

    for (index, char) in chars.iter().enumerate() {
        if matches!(char, '<' | '>' | '\\') {
            destination.push('\\');
        }

        if *char == '&' && entity_at(&chars[index..]) {
            destination.push_str("&amp;");
        } else {
            destination.push(*char);
        }
    }

    let wrap = destination.is_empty() || destination.chars().any(|char| is_space(char) || matches!(char, '(' | ')'));
    let target = if wrap { format!("<{destination}>") } else { destination };
    let caption = if node.title.is_empty() {
        String::new()
    } else {
        format!(" \"{}\"", node.title.replace('\\', "\\\\").replace('"', "\\\""))
    };

    format!("[{content}]({target}{caption})")
}

fn inline(node: &Node, format: Format) -> Result<String> {
    match node.kind {
        Kind::Text => return Ok(if format == Format::Html { escape_html(&node.value) } else { node.value.clone() }),
        Kind::Break => return Ok(if format == Format::Html { "<br>".into() } else { "\n".into() }),
        Kind::InlineCode => {
            return match format {
                Format::Plain => Ok(node.value.clone()),
                Format::Markdown => code_span(&node.value),
                Format::Html => Ok(format!("<code>{}</code>", escape_html(&node.value))),
            };
        }
        _ => {}
    }

    let content = inlines(&node.children, format)?;

    if node.is(Kind::Link) {
        if format == Format::Html {
            let title =
                if node.title.is_empty() { String::new() } else { format!(" title=\"{}\"", attribute(&node.title)) };

            return Ok(format!("<a href=\"{}\"{title}>{content}</a>", attribute(&node.href)));
        }

        return Ok(if content == node.href || address(node) { content } else { format!("{content} ({})", node.href) });
    }

    if format == Format::Plain {
        return Ok(content);
    }

    let tag = match node.kind {
        Kind::Strong => "strong",
        Kind::Emphasis => "em",
        Kind::Strike => "del",
        _ => return Err(Failure::Unsupported),
    };

    Ok(format!("<{tag}>{content}</{tag}>"))
}

fn inlines(children: &[Node], format: Format) -> Result<String> {
    children.iter().map(|child| inline(child, format)).collect()
}

fn table(node: &Node, format: Format, strict: bool) -> Result<String> {
    let mut rows = Vec::new();

    for row in &node.children {
        let mut cells = Vec::new();

        for item in &row.children {
            cells.push(if format == Format::Markdown {
                protected_prose(&item.children, false, strict)?.replace('|', "\\|")
            } else {
                inlines(&item.children, format)?
            });
        }

        rows.push(cells);
    }

    if format == Format::Plain {
        let line = |cells: &Vec<String>| {
            cells.iter().map(|value| value.replace(['\t', '\n', '\r'], " ")).collect::<Vec<_>>().join("\t")
        };

        return Ok(rows.iter().map(line).collect::<Vec<_>>().join("\n"));
    }

    if format == Format::Markdown {
        let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
        let first = rows.first().map_or(&[][..], Vec::as_slice);
        let rule = vec!["---".to_owned(); first.len()];
        let mut lines = vec![line(first), line(&rule)];

        lines.extend(rows.iter().skip(1).map(|cells| line(cells)));
        return Ok(lines.join("\n"));
    }

    let lines: Vec<String> = node
        .children
        .iter()
        .zip(&rows)
        .map(|(row, values)| {
            let cells = row.children.iter().zip(values).fold(String::new(), |mut cells, (item, value)| {
                let tag = if item.header { "th" } else { "td" };

                let _ = write!(cells, "<{tag}>{value}</{tag}>");
                cells
            });

            format!("<tr>{cells}</tr>")
        })
        .collect();
    let split =
        node.children.iter().position(|row| !row.children.iter().all(|item| item.header)).unwrap_or(lines.len());
    let mut parts = vec!["<table>".to_owned()];

    if split > 0 {
        parts.push("<thead>".into());
        parts.extend_from_slice(&lines[..split]);
        parts.push("</thead>".into());
    }

    if split < lines.len() {
        parts.push("<tbody>".into());
        parts.extend_from_slice(&lines[split..]);
        parts.push("</tbody>".into());
    }

    parts.push("</table>".into());
    Ok(parts.join("\n"))
}

fn task(item: &Node, format: Format) -> &'static str {
    match (item.checked, format) {
        (None, _) => "",
        (Some(true), Format::Html) => "<input type=\"checkbox\" disabled checked> ",
        (Some(false), Format::Html) => "<input type=\"checkbox\" disabled> ",
        (Some(true), _) => "[x] ",
        (Some(false), _) => "[ ] ",
    }
}

fn alternate(node: &Node, index: usize, siblings: &[Node]) -> bool {
    let run = siblings[..index]
        .iter()
        .rev()
        .take_while(|previous| previous.is(Kind::List) && previous.ordered == node.ordered)
        .count();

    run % 2 == 1
}

struct Writer<'a> {
    format: Format,
    strict: bool,
    warnings: &'a mut Warnings,
}

impl Writer<'_> {
    fn content(&self, node: &Node, line_start: bool) -> Result<String> {
        if self.format == Format::Markdown {
            protected_prose(&node.children, line_start, self.strict)
        } else {
            inlines(&node.children, self.format)
        }
    }

    fn children(&mut self, node: &Node) -> Result<String> {
        let mut parts = Vec::new();

        for index in 0..node.children.len() {
            parts.push(self.render(&node.children, index)?);
        }

        Ok(parts.join(if self.format == Format::Html { "\n" } else { "\n\n" }))
    }

    fn render(&mut self, siblings: &[Node], index: usize) -> Result<String> {
        let node = &siblings[index];
        let html = self.format == Format::Html;

        match node.kind {
            Kind::Document => self.children(node),
            Kind::ListItem => {
                let mut result = String::new();

                for position in 0..node.children.len() {
                    if position > 0 {
                        let child = &node.children[position];

                        result.push_str(if html || child.is(Kind::List) { "\n" } else { "\n\n" });
                    }

                    result.push_str(&self.render(&node.children, position)?);
                }

                Ok(result)
            }
            Kind::Paragraph => {
                let content = self.content(node, true)?;

                Ok(if html { format!("<p>{content}</p>") } else { content })
            }
            Kind::Heading => {
                let level = node.level;

                match self.format {
                    Format::Html => Ok(format!("<h{level}>{}</h{level}>", self.content(node, true)?)),
                    Format::Markdown => {
                        Ok(format!("{} {}", "#".repeat(level.into()), closing(&self.content(node, false)?)))
                    }
                    Format::Plain => self.content(node, false),
                }
            }
            Kind::ThematicBreak => Ok(if html { "<hr>".into() } else { "---".into() }),
            Kind::Quote => {
                let value = self.children(node)?;

                if html {
                    return Ok(format!("<blockquote>{value}</blockquote>"));
                }

                Ok(value.split('\n').map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n"))
            }
            Kind::List => self.list(node, index, siblings),
            Kind::Table => table(node, self.format, self.strict),
            Kind::CodeBlock => Ok(self.code(node)),
            _ => Err(Failure::Unsupported),
        }
    }

    fn list(&mut self, node: &Node, index: usize, siblings: &[Node]) -> Result<String> {
        if self.format == Format::Html {
            let tag = if node.ordered { "ol" } else { "ul" };
            let start =
                if node.ordered && node.start != 1 { format!(" start=\"{}\"", node.start) } else { String::new() };
            let tight =
                node.children.iter().all(|item| item.children.iter().filter(|child| !child.is(Kind::List)).count() < 2);
            let mut items = String::new();

            for item in &node.children {
                let mut body = Vec::new();

                for position in 0..item.children.len() {
                    let child = &item.children[position];

                    body.push(if tight && child.is(Kind::Paragraph) {
                        inlines(&child.children, self.format)?
                    } else {
                        self.render(std::slice::from_ref(child), 0)?
                    });
                }

                let _ = write!(items, "<li>{}{}</li>", task(item, self.format), body.join("\n"));
            }

            return Ok(format!("<{tag}{start}>{items}</{tag}>"));
        }

        let second = self.format == Format::Markdown && alternate(node, index, siblings);
        let delimiter = if second { ')' } else { '.' };
        let bullet = if second { "* " } else { "- " };
        let mut items = Vec::new();

        for (offset, child) in node.children.iter().enumerate() {
            let marker =
                if node.ordered { format!("{}{delimiter} ", node.start as usize + offset) } else { bullet.to_owned() };
            let padding = " ".repeat(if self.format == Format::Markdown { marker.len() } else { 2 });
            let body = format!("{}{}", task(child, self.format), self.render(std::slice::from_ref(child), 0)?);
            let lines: Vec<String> = body
                .split('\n')
                .enumerate()
                .map(|(position, line)| {
                    if position == 0 {
                        format!("{marker}{line}")
                    } else if line.is_empty() {
                        String::new()
                    } else {
                        format!("{padding}{line}")
                    }
                })
                .collect();

            items.push(lines.join("\n"));
        }

        Ok(items.join("\n"))
    }

    fn code(&mut self, node: &Node) -> String {
        match self.format {
            Format::Html => {
                let class = if node.language.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"language-{}\"", node.language)
                };
                let ending = if node.value.ends_with('\n') { "\n" } else { "" };

                format!("<pre><code{class}>{}{ending}</code></pre>", escape_html(&node.value))
            }
            Format::Plain => node.value.clone(),
            Format::Markdown => {
                if !node.value.ends_with('\n') {
                    self.warnings.count("codeBoundary");
                }

                let delimiter = fence(&node.value);

                format!("{delimiter}{}\n{}\n{delimiter}", node.language, node.value)
            }
        }
    }
}

fn closing(content: &str) -> String {
    let body = content.trim_end_matches([' ', '\t']);
    let hashes = body.len() - body.trim_end_matches('#').len();

    if hashes == 0 {
        return content.to_owned();
    }

    let at = body.len() - hashes;
    let opens = at == 0 || content[..at].ends_with([' ', '\t']);

    if opens { format!("{}\\{}", &content[..at], &content[at..]) } else { content.to_owned() }
}

pub fn serialize(tree: &Node, format: Format, warnings: &mut Warnings, strict: bool) -> Result<String> {
    let mut writer = Writer { format, strict, warnings };

    writer.render(std::slice::from_ref(tree), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(children: Vec<Node>) -> Node {
        Node::parent(Kind::Document, children)
    }

    fn markdown(tree: &Node) -> String {
        serialize(tree, Format::Markdown, &mut Warnings::default(), false).unwrap()
    }

    #[test]
    fn escapes_line_markers() {
        let tree = document(vec![Node::paragraph(vec![Node::text("1. a")]), Node::paragraph(vec![Node::text("# b")])]);

        assert_eq!(markdown(&tree), "1\\. a\n\n\\# b");
    }

    #[test]
    fn writes_marks_and_links() {
        let tree = document(vec![Node::paragraph(vec![
            Node::parent(Kind::Strong, vec![Node::text("a")]),
            Node::text(" "),
            Node::link("https://example.com".into(), String::new(), vec![Node::text("b")]),
            Node::text(" 2*3"),
        ])]);

        assert_eq!(markdown(&tree), "**a** [b](https://example.com) 2\\*3");
    }

    #[test]
    fn writes_html_lists() {
        let item = Node::parent(Kind::ListItem, vec![Node::paragraph(vec![Node::text("a<b")])]);
        let tree = document(vec![Node::list(true, 3, vec![item])]);

        assert_eq!(
            serialize(&tree, Format::Html, &mut Warnings::default(), false).unwrap(),
            "<ol start=\"3\"><li>a&lt;b</li></ol>"
        );
    }

    #[test]
    fn closes_headings() {
        assert_eq!(closing("a ##"), "a \\##");
        assert_eq!(closing("a##"), "a##");
    }
}
