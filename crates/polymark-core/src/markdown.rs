use std::collections::HashMap;
use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::chars::is_space;
use crate::clean::{link_title, url};
use crate::model::{Failure, Kind, Node, Result, Warnings, blocks, check, language, prose, table, visible};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    Item,
    Image,
    Code,
    Html,
    Head,
    Table,
    Link(bool),
}

struct Frame {
    node: Node,
    role: Role,
    rows: Vec<Vec<Node>>,
}

struct Reader<'a> {
    source: &'a str,
    stack: Vec<Frame>,
    pending: Vec<(String, bool)>,
    size: usize,
    numbers: Vec<String>,
    defined: HashMap<String, Events<'a>>,
    warnings: &'a mut Warnings,
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

impl<'a> Reader<'a> {
    fn top(&mut self) -> &mut Frame {
        self.stack.last_mut().expect("the document frame stays until the end")
    }

    fn grow(&mut self) -> Result<()> {
        self.size += 1;

        if self.size > 5000 || self.stack.len() > 24 {
            return Err(Failure::Structure);
        }

        Ok(())
    }

    fn in_link(&self) -> bool {
        self.stack.iter().any(|frame| matches!(frame.role, Role::Link(_)))
    }

    fn add(&mut self, node: Node) -> Result<()> {
        self.flush()?;
        self.grow()?;
        self.top().node.children.push(node);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let linkable = !self.in_link();
        let mut nodes = Vec::new();
        let mut run = String::new();

        for (value, escaped) in std::mem::take(&mut self.pending) {
            if escaped {
                nodes.extend(if linkable { autolinks(&run, self.warnings) } else { vec![Node::text(&run)] });
                run.clear();
                nodes.push(Node::text(value));
            } else {
                run.push_str(&value);
            }
        }

        nodes.extend(if linkable { autolinks(&run, self.warnings) } else { vec![Node::text(run)] });

        for node in nodes {
            self.grow()?;
            self.top().node.children.push(node);
        }

        Ok(())
    }

    fn open(&mut self, node: Node, role: Role) -> Result<()> {
        self.flush()?;
        self.stack.push(Frame { node, role, rows: Vec::new() });
        self.grow()
    }

    fn close(&mut self) -> Result<Frame> {
        self.flush()?;
        Ok(self.stack.pop().expect("every end has a start"))
    }

    fn text(&mut self, value: &str, range: Range<usize>) -> Result<()> {
        let raw = &self.source[range.clone()];

        match self.top().role {
            Role::Code | Role::Html => {
                self.top().node.value.push_str(value);
                return Ok(());
            }
            _ => {}
        }

        if raw.starts_with("&#") && value == "\u{fffd}" {
            self.flush()?;
            self.warnings.count("references");
            self.pending.push((value.to_owned(), true));
            return Ok(());
        }

        let slashes = self.source[..range.start].bytes().rev().take_while(|byte| *byte == b'\\').count();
        let first = value.chars().next().filter(|_| slashes % 2 == 1);

        match first {
            Some(char) => {
                self.pending.push((char.to_string(), true));
                self.pending.push((value[char.len_utf8()..].to_owned(), false));
            }
            None => self.pending.push((value.to_owned(), false)),
        }

        Ok(())
    }

    fn start(&mut self, tag: Tag<'a>) -> Result<()> {
        match tag {
            Tag::Paragraph => self.open(Node::new(Kind::Paragraph), Role::Plain),
            Tag::Heading { level, .. } => {
                self.open(Node { level: level as u8, ..Node::new(Kind::Heading) }, Role::Plain)
            }
            Tag::BlockQuote(_) => self.open(Node::new(Kind::Quote), Role::Plain),
            Tag::CodeBlock(kind) => {
                let info = match kind {
                    CodeBlockKind::Fenced(info) => language(&info),
                    CodeBlockKind::Indented => String::new(),
                };

                self.open(Node::code_block("", info), Role::Code)
            }
            Tag::HtmlBlock => {
                self.warnings.count("rawHtml");
                self.open(Node::new(Kind::Paragraph), Role::Html)
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let mut first = start.unwrap_or(1);

                if !(1..=999_999).contains(&first) {
                    first = 1;
                    self.warnings.count("counters");
                }

                self.open(Node::list(ordered, first as u32, Vec::new()), Role::Plain)
            }
            Tag::Item => self.open(Node::new(Kind::ListItem), Role::Item),
            Tag::Table(_) => self.open(Node::new(Kind::Table), Role::Table),
            Tag::TableHead => self.open(Node::new(Kind::Row), Role::Head),
            Tag::TableRow => self.open(Node::new(Kind::Row), Role::Plain),
            Tag::TableCell => {
                let header = self.top().role == Role::Head;

                self.open(Node::cell(header, Vec::new()), Role::Plain)
            }
            Tag::Emphasis => self.open(Node::new(Kind::Emphasis), Role::Plain),
            Tag::Strong => self.open(Node::new(Kind::Strong), Role::Plain),
            Tag::Strikethrough => self.open(Node::new(Kind::Strike), Role::Plain),
            Tag::Link { link_type, dest_url, title, .. } => {
                let auto = matches!(link_type, LinkType::Autolink | LinkType::Email);
                let href =
                    if link_type == LinkType::Email { format!("mailto:{dest_url}") } else { dest_url.into_string() };

                self.open(Node::link(href, link_title(&title), Vec::new()), Role::Link(auto))
            }
            Tag::Image { .. } => {
                self.warnings.count("images");
                self.open(Node::new(Kind::Paragraph), Role::Image)
            }
            _ => Err(Failure::Unsupported),
        }
    }

    fn end(&mut self) -> Result<()> {
        let mut frame = self.close()?;
        let node = &mut frame.node;

        match frame.role {
            Role::Code => {
                if node.value.ends_with('\n') {
                    node.value.pop();
                }
            }
            Role::Html => {
                let raw = node.value.trim_end_matches('\n').to_owned();

                node.children = prose(&raw);
                node.value.clear();
            }
            Role::Image => {
                let caption = visible(node);

                return self.add(Node::text(caption));
            }
            Role::Link(auto) => {
                let Some(href) = url(&node.href, self.warnings) else {
                    let children = std::mem::take(&mut node.children);

                    for child in children {
                        self.add(child)?;
                    }

                    return Ok(());
                };

                node.href = href;

                if auto {
                    node.children = vec![Node::text(visible(node))];
                }
            }
            Role::Item => {
                node.children = blocks(std::mem::take(&mut node.children));

                if node.checked.is_some() && !node.children.first().is_some_and(|child| child.is(Kind::Paragraph)) {
                    node.children.insert(0, Node::paragraph(Vec::new()));
                }
            }
            Role::Head => {
                let cells = std::mem::take(&mut node.children);

                self.top().rows.push(cells);
                return Ok(());
            }
            Role::Table => {
                let built = table(std::mem::take(&mut frame.rows))?;

                return self.add(built);
            }
            Role::Plain => match node.kind {
                Kind::Quote => node.children = blocks(std::mem::take(&mut node.children)),
                Kind::Cell => trim_cell(&mut node.children),
                Kind::Row => {
                    let cells = std::mem::take(&mut node.children);

                    self.top().rows.push(cells);
                    return Ok(());
                }
                _ => {}
            },
        }

        let node = frame.node;

        self.grow()?;
        self.top().node.children.push(node);
        Ok(())
    }

    fn reference(&mut self, label: &str) {
        let label = label.to_lowercase();

        if !self.defined.contains_key(&label) {
            self.pending.push((format!("[^{label}]"), true));
            return;
        }

        let number = if let Some(index) = self.numbers.iter().position(|known| *known == label) {
            index + 1
        } else {
            self.numbers.push(label);
            self.numbers.len()
        };

        self.pending.push((format!("[{number}]"), true));
    }

    fn event(&mut self, event: Event<'a>, range: Range<usize>) -> Result<()> {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(_) => self.end(),
            Event::Text(value) => self.text(&value, range),
            Event::Code(value) => self.add(Node::leaf(Kind::InlineCode, value.into_string())),
            Event::Html(value) => {
                self.top().node.value.push_str(&value);
                Ok(())
            }
            Event::InlineHtml(value) => {
                self.flush()?;
                self.warnings.count("rawHtml");
                self.pending.push((value.into_string(), true));
                Ok(())
            }
            Event::FootnoteReference(label) => {
                self.reference(&label);
                Ok(())
            }
            Event::SoftBreak | Event::HardBreak => self.add(Node::new(Kind::Break)),
            Event::Rule => self.add(Node::new(Kind::ThematicBreak)),
            Event::TaskListMarker(checked) => {
                if let Some(item) = self.stack.iter_mut().rev().find(|frame| frame.role == Role::Item) {
                    item.node.checked = Some(checked);
                }

                Ok(())
            }
            _ => Err(Failure::Unsupported),
        }
    }

    fn run(&mut self, events: Events<'a>) -> Result<Vec<Node>> {
        self.stack.push(Frame { node: Node::new(Kind::Document), role: Role::Plain, rows: Vec::new() });

        for (event, range) in events {
            self.event(event, range)?;
        }

        let frame = self.close()?;

        Ok(blocks(frame.node.children))
    }
}

type Events<'a> = Vec<(Event<'a>, Range<usize>)>;

fn definitions(source: &str) -> (Events<'_>, HashMap<String, Events<'_>>) {
    let mut main = Vec::new();
    let mut defined: HashMap<String, Events<'_>> = HashMap::new();
    let mut current: Option<(String, Events<'_>)> = None;
    let mut depth = 0;

    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match (&event, current.as_mut()) {
            (Event::Start(Tag::FootnoteDefinition(label)), None) => {
                current = Some((label.to_lowercase(), Vec::new()));
                depth = 0;
            }
            (Event::Start(_), Some((_, events))) => {
                depth += 1;
                events.push((event, range));
            }
            (Event::End(TagEnd::FootnoteDefinition), Some(_)) if depth == 0 => {
                if let Some((label, events)) = current.take() {
                    defined.entry(label).or_insert(events);
                }
            }
            (Event::End(_), Some((_, events))) => {
                depth -= 1;
                events.push((event, range));
            }
            (_, Some((_, events))) => events.push((event, range)),
            (_, None) => main.push((event, range)),
        }
    }

    (main, defined)
}

pub fn parse(source: &str, warnings: &mut Warnings) -> Result<Node> {
    let (main, defined) = definitions(source);
    let mut labels: Vec<String> = defined.keys().cloned().collect();
    let order: HashMap<String, usize> = defined
        .iter()
        .map(|(label, events)| (label.clone(), events.first().map_or(0, |(_, range)| range.start)))
        .collect();

    labels.sort_by_key(|label| order[label]);

    let mut reader =
        Reader { source, stack: Vec::new(), pending: Vec::new(), size: 0, numbers: Vec::new(), defined, warnings };
    let mut children = reader.run(main)?;

    for label in labels {
        if !reader.numbers.contains(&label) {
            reader.numbers.push(label);
        }
    }

    let mut notes = Vec::new();
    let mut index = 0;

    while index < reader.numbers.len() {
        let label = reader.numbers[index].clone();
        let events = reader.defined.get(&label).cloned().unwrap_or_default();

        notes.push(Node::parent(Kind::ListItem, reader.run(events)?));
        index += 1;
    }

    if !notes.is_empty() {
        reader.warnings.count("footnotes");
        children.push(Node::list(true, 1, notes));
    }

    let tree = Node::parent(Kind::Document, children);

    check(&tree)?;
    Ok(tree)
}

fn trim_cell(children: &mut [Node]) {
    if let Some(first) = children.first_mut().filter(|node| node.is(Kind::Text)) {
        first.value = first.value.trim_start_matches(is_space).to_owned();
    }

    if let Some(last) = children.last_mut().filter(|node| node.is(Kind::Text)) {
        last.value = last.value.trim_end_matches(is_space).to_owned();
    }
}

fn trailing(char: char) -> bool {
    matches!(char, '?' | '!' | '.' | ',' | ':' | ';' | '*' | '_' | '\'' | '"' | '~' | ')')
}

fn backpedal_once(chars: &[char]) -> usize {
    let stop = |char: char| trailing(char) || matches!(char, '(' | '&');
    let mut index = 0;

    while index < chars.len() {
        let char = chars[index];

        if !stop(char) {
            index += 1;
            continue;
        }

        if char == '(' {
            match chars[index + 1..].iter().position(|next| *next == ')') {
                Some(close) => {
                    index += close + 2;
                    continue;
                }
                None => break,
            }
        }

        if char == '&' {
            let rest = &chars[index + 1..];
            let name = rest.iter().take_while(|next| next.is_ascii_alphanumeric()).count();

            if name > 0 && rest.len() == name + 1 && rest[name] == ';' {
                break;
            }

            index += 1;
            continue;
        }

        let end = index + chars[index..].iter().take_while(|next| trailing(**next)).count();

        if end < chars.len() {
            index = end;
        } else if end - index > 1 {
            index = end - 1;
        } else {
            break;
        }
    }

    index
}

fn backpedal(chars: &[char]) -> usize {
    let mut length = chars.len();

    loop {
        let next = backpedal_once(&chars[..length]);

        if next == length {
            return length;
        }

        length = next;
    }
}

fn starts_with(chars: &[char], prefix: &str, fold: bool) -> bool {
    let prefix: Vec<char> = prefix.chars().collect();

    chars.len() >= prefix.len()
        && chars.iter().zip(&prefix).all(|(a, b)| if fold { a.eq_ignore_ascii_case(b) } else { a == b })
}

fn web_link(chars: &[char]) -> Option<(usize, String)> {
    let prefix = ["https://", "http://", "ftp://"]
        .into_iter()
        .find(|prefix| starts_with(chars, prefix, true))
        .map(str::len)
        .or_else(|| starts_with(chars, "www.", false).then_some(4))?;

    if !chars.get(prefix).is_some_and(|char| char.is_ascii_alphanumeric() || *char == '-') {
        return None;
    }

    let end = chars.iter().position(|char| is_space(*char) || *char == '<').unwrap_or(chars.len());
    let length = backpedal(&chars[..end]);
    let text: String = chars[..length].iter().collect();
    let www = chars.get(prefix - 1).is_none_or(|char| *char != '/');
    let href = if www { format!("http://{text}") } else { text };

    (length > prefix).then_some((length, href))
}

fn narrow(char: char) -> bool {
    char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '+' | '-')
}

fn domain_end(chars: &[char]) -> Option<usize> {
    let label = chars.iter().take_while(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_')).count();

    if label == 0 {
        return None;
    }

    let mut index = label;
    let mut found = None;

    while chars.get(index) == Some(&'.') {
        let run = chars[index + 1..]
            .iter()
            .take_while(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_'))
            .count();
        let last = chars[index + 1..index + 1 + run].iter().rposition(char::is_ascii_alphanumeric);

        match last {
            Some(last) if last + 1 == run => {
                index += 1 + run;
                found = Some(index);
            }
            _ => break,
        }
    }

    found
}

fn email(chars: &[char]) -> Option<usize> {
    let local = chars.iter().take_while(|char| narrow(**char)).count();

    if local == 0 || chars.get(local) != Some(&'@') {
        return None;
    }

    domain_end(&chars[local + 1..]).map(|end| local + 1 + end)
}

fn autolinks(value: &str, warnings: &mut Warnings) -> Vec<Node> {
    let chars: Vec<char> = value.chars().collect();
    let mut nodes = Vec::new();
    let mut plain = String::new();
    let mut index = 0;

    while index < chars.len() {
        let rest = &chars[index..];
        let previous = index.checked_sub(1).map(|at| chars[at]);
        let mut found = web_link(rest);

        if found.is_none()
            && starts_with(rest, "mailto:", false)
            && !previous.is_some_and(|char| char.is_ascii_alphanumeric())
        {
            found = email(&rest[7..]).map(|end| {
                let length = backpedal(&rest[..7 + end]);
                let text: String = rest[..length].iter().collect();

                (length, text)
            });
        }

        if found.is_none() && narrow(rest[0]) && !previous.is_some_and(narrow) {
            found = email(rest).map(|end| {
                let text: String = rest[..end].iter().collect();

                (end, format!("mailto:{text}"))
            });
        }

        let Some((length, href)) = found else {
            plain.push(chars[index]);
            index += 1;
            continue;
        };

        let text: String = rest[..length].iter().collect();

        if !plain.is_empty() {
            nodes.push(Node::text(std::mem::take(&mut plain)));
        }

        match url(&href, warnings) {
            Some(href) => nodes.push(Node::link(href, String::new(), vec![Node::text(text)])),
            None => nodes.push(Node::text(text)),
        }

        index += length;
    }

    if !plain.is_empty() || nodes.is_empty() {
        nodes.push(Node::text(plain));
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> (Node, Warnings) {
        let mut warnings = Warnings::default();
        let tree = parse(source, &mut warnings).unwrap();

        (tree, warnings)
    }

    fn first(tree: &Node) -> &[Node] {
        &tree.children[0].children
    }

    #[test]
    fn links_bare_addresses() {
        let (tree, _) = read("see https://example.com/a_(b), www.site.ru and me@mail.ru.");
        let links: Vec<&str> =
            first(&tree).iter().filter(|node| node.is(Kind::Link)).map(|node| node.href.as_str()).collect();

        assert_eq!(links, ["https://example.com/a_(b)", "http://www.site.ru", "mailto:me@mail.ru"]);
    }

    #[test]
    fn finds_mail_after_any_sign() {
        let (tree, _) = read("!a@b.ru 2*3c@d.ru");
        let links: Vec<&str> =
            first(&tree).iter().filter(|node| node.is(Kind::Link)).map(|node| node.href.as_str()).collect();

        assert_eq!(links, ["mailto:a@b.ru", "mailto:3c@d.ru"]);
    }

    #[test]
    fn keeps_broken_references_out_of_links() {
        let (tree, warnings) = read("https://a.ru:99999 &#0;");
        let link = first(&tree).iter().find(|node| node.is(Kind::Link));

        assert!(link.is_none());
        assert_eq!(warnings.0.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["links", "references"]);

        let (tree, _) = read("https://a.ru/b&#0;");

        assert_eq!(first(&tree)[0].href, "https://a.ru/b");
    }

    #[test]
    fn escapes_stop_links() {
        let (tree, _) = read("www\\.site.ru");

        assert!(first(&tree).iter().all(|node| !node.is(Kind::Link)));
    }

    #[test]
    fn keeps_breaks_and_tasks() {
        let (tree, _) = read("a\nb\n\n- [x] done\n- [ ] ");
        let list = &tree.children[1];

        assert_eq!(first(&tree).len(), 3);
        assert_eq!(list.children[0].checked, Some(true));
        assert_eq!(list.children[1].checked, Some(false));
        assert!(list.children[1].children[0].is(Kind::Paragraph));
    }

    #[test]
    fn numbers_footnotes() {
        let (tree, warnings) = read("a[^x] b[^y]\n\n[^y]: second\n[^x]: first\n");

        assert_eq!(visible(&tree.children[0]), "a[1] b[2]");
        assert_eq!(visible(&tree.children[1]), "firstsecond");
        assert!(warnings.has("footnotes"));
    }

    #[test]
    fn counts_broken_references() {
        let (_, warnings) = read("a &#0; b");

        assert!(warnings.has("references"));
    }

    #[test]
    fn rejects_deep_nesting() {
        let mut warnings = Warnings::default();

        assert_eq!(parse(&">".repeat(20000), &mut warnings), Err(Failure::Structure));
    }
}
