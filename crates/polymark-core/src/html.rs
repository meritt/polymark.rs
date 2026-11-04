use crate::chars::{is_blank, is_space, number, trim};
use crate::clean::{link_title, url};
use crate::dom::{Data, Dom, fragment};
use crate::model::{Failure, Kind, Node, Result, Warnings, blocks, check, language, table, visible};

const REMOVED: [&str; 20] = [
    "script", "style", "template", "noscript", "iframe", "object", "embed", "svg", "math", "canvas", "form", "input",
    "button", "select", "textarea", "head", "base", "link", "meta", "title",
];
const BLOCKS: [&str; 17] = [
    "p",
    "div",
    "article",
    "section",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "details",
    "summary",
    "address",
    "dl",
    "dt",
    "dd",
];
const WRAPPERS: [&str; 4] = ["span", "font", "html", "body"];
const TABLE_PARTS: [&str; 6] = ["tbody", "thead", "tfoot", "tr", "td", "th"];

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub tags: usize,
    pub nodes: usize,
}

pub const BOUNDS: Bounds = Bounds { tags: 2000, nodes: 5000 };

fn allowed(tag: &str) -> &'static [&'static str] {
    match tag {
        "a" => &["href", "title"],
        "code" => &["class"],
        "ol" => &["start", "reversed"],
        "li" => &["value"],
        "td" | "th" => &["rowspan", "colspan"],
        "img" => &["alt"],
        _ => &[],
    }
}

fn emphasis(tag: &str) -> Option<Kind> {
    match tag {
        "strong" | "b" => Some(Kind::Strong),
        "em" | "i" => Some(Kind::Emphasis),
        "del" | "s" => Some(Kind::Strike),
        _ => None,
    }
}

fn heading(tag: &str) -> Option<u8> {
    let bytes = tag.as_bytes();

    (bytes.len() == 2 && bytes[0] == b'h' && (b'1'..=b'6').contains(&bytes[1])).then(|| bytes[1] - b'0')
}

#[derive(Debug, Default)]
struct Element {
    tag: String,
    value: String,
    attrs: Vec<(String, String)>,
    marks: Vec<Kind>,
    children: Vec<usize>,
    checked: bool,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(known, _)| known == name).map(|(_, value)| value.as_str())
    }

    fn has(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }
}

struct Tree {
    dom: Vec<Dom>,
}

impl Tree {
    fn tag(&self, node: usize) -> Option<&str> {
        match &self.dom[node].data {
            Data::Element { name, .. } => Some(&name.local),
            _ => None,
        }
    }

    fn attr(&self, node: usize, wanted: &str) -> Option<&str> {
        match &self.dom[node].data {
            Data::Element { attrs, .. } => {
                attrs.iter().find(|attribute| &*attribute.name.local == wanted).map(|attribute| &*attribute.value)
            }
            _ => None,
        }
    }

    fn closest(&self, node: usize, tags: &[&str]) -> bool {
        let mut current = Some(node);

        while let Some(at) = current {
            if self.tag(at).is_some_and(|tag| tags.contains(&tag)) {
                return true;
            }

            current = self.dom[at].parent;
        }

        false
    }

    fn declaration(&self, node: usize, name: &str) -> String {
        let style = self.attr(node, "style").unwrap_or("");

        for segment in style.split(';') {
            let segment = segment.trim_start_matches(is_space);
            let Some(start) = segment.get(..name.len()).filter(|start| start.eq_ignore_ascii_case(name)) else {
                continue;
            };
            let rest = segment[start.len()..].trim_start_matches(is_space);
            let Some(value) = rest.strip_prefix(':') else {
                continue;
            };
            let value = value.split('!').next().unwrap_or("");

            if !value.is_empty() {
                return trim(value).to_lowercase();
            }
        }

        String::new()
    }

    fn weight(&self, node: usize) -> f64 {
        match self.declaration(node, "font-weight").as_str() {
            "" => f64::NAN,
            "normal" => 400.0,
            "lighter" => 300.0,
            "bold" | "bolder" => 700.0,
            value => number(value),
        }
    }

    fn shape(&self, node: usize, tag: &str) -> (String, Vec<Kind>) {
        let bold = self.weight(node);
        let mut marks = Vec::new();
        let orphan = (TABLE_PARTS.contains(&tag) && !self.closest(node, &["table"]))
            || (tag == "li" && !self.closest(node, &["ul", "ol"]));

        if orphan {
            return ("div".into(), marks);
        }

        if tag == "b" && bold < 600.0 {
            return ("span".into(), marks);
        }

        if tag == "span" && bold >= 600.0 {
            marks.push(Kind::Strong);
        }

        if tag == "span" && self.declaration(node, "font-style") == "italic" {
            marks.push(Kind::Emphasis);
        }

        (tag.to_owned(), marks)
    }
}

struct Projector<'a> {
    elements: Vec<Element>,
    warnings: &'a mut Warnings,
}

fn read(source: &str, bounds: Bounds, warnings: &mut Warnings) -> Result<Vec<Element>> {
    let (dom, root) = fragment(source);
    let tree = Tree { dom };
    let mut elements = vec![Element { tag: "root".into(), ..Element::default() }];
    let mut stack = vec![(root, 0_usize, 0_usize)];
    let mut size = 0;

    while let Some((node, target, depth)) = stack.pop() {
        size += 1;

        if size > bounds.nodes || depth > 64 {
            return Err(Failure::Structure);
        }

        let mut parent = target;

        match &tree.dom[node].data {
            Data::Text(value) => {
                let id = elements.len();

                elements.push(Element { tag: "#text".into(), value: value.clone(), ..Element::default() });
                elements[target].children.push(id);
                continue;
            }
            Data::Comment | Data::Other => {
                warnings.count("comments");
                continue;
            }
            Data::Element { name, attrs, .. } if node != root => {
                let (tag, marks) = tree.shape(node, &name.local);

                if tag == "input" && tree.attr(node, "type").is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox")) {
                    let id = elements.len();

                    elements.push(Element {
                        tag: "checkbox".into(),
                        checked: tree.attr(node, "checked").is_some(),
                        ..Element::default()
                    });
                    elements[target].children.push(id);
                    continue;
                }

                if REMOVED.contains(&tag.as_str()) {
                    warnings.count("active");
                    continue;
                }

                let hidden = tree.attr(node, "hidden").is_some()
                    || tree.attr(node, "aria-hidden").is_some_and(|value| trim(value).eq_ignore_ascii_case("true"));

                if hidden {
                    warnings.count("hidden");
                    continue;
                }

                let permitted = allowed(&tag);
                let mut kept = Vec::new();

                for attribute in attrs {
                    let name = attribute.name.local.to_string();

                    if name.starts_with("on") || name == "data-style" {
                        continue;
                    }

                    if permitted.contains(&name.as_str()) {
                        kept.push((name, attribute.value.to_string()));
                    } else {
                        warnings.count("attributes");
                    }
                }

                parent = elements.len();
                elements.push(Element { tag, attrs: kept, marks, ..Element::default() });
                elements[target].children.push(parent);
            }
            Data::Document | Data::Fragment | Data::Element { .. } => {}
        }

        for child in tree.dom[node].children.iter().rev() {
            stack.push((*child, parent, depth + 1));
        }
    }

    Ok(elements)
}

fn block_tag(tag: &str) -> bool {
    BLOCKS.contains(&tag)
        || heading(tag).is_some()
        || ["ul", "ol", "li", "blockquote", "pre", "hr", "caption"].contains(&tag)
        || TABLE_PARTS.contains(&tag)
}

fn span(value: Option<&str>) -> usize {
    let digits: String = value.map_or("", trim).chars().take_while(char::is_ascii_digit).collect();
    let signed = value.map(trim).is_some_and(|value| value.starts_with('-'));

    match digits.parse::<usize>() {
        Ok(number) if number > 1 && !signed => number.min(100),
        _ => 1,
    }
}

struct Slot {
    item: Option<Node>,
    header: bool,
}

impl Projector<'_> {
    fn payload(&self, id: usize, images: bool, boundaries: bool) -> String {
        let mut stack = vec![Some(id)];
        let mut chunks = String::new();

        while let Some(current) = stack.pop() {
            let Some(current) = current else {
                chunks.push('\n');
                continue;
            };
            let element = &self.elements[current];

            match element.tag.as_str() {
                "#text" => chunks.push_str(&element.value),
                "img" if images => chunks.push_str(element.attr("alt").unwrap_or("")),
                "br" => chunks.push('\n'),
                tag => {
                    if boundaries && block_tag(tag) {
                        chunks.push('\n');
                        stack.push(None);
                    }

                    stack.extend(element.children.iter().rev().map(|child| Some(*child)));
                }
            }
        }

        chunks
    }

    fn cell(&mut self, id: usize, children: Vec<Node>) -> (Node, usize, usize) {
        let parts: Vec<Node> = blocks(children)
            .into_iter()
            .filter(|part| !part.is(Kind::Paragraph) || !is_blank(&visible(part)))
            .collect();
        let flat = parts.len() < 2 && parts.iter().all(|part| part.is(Kind::Paragraph));

        if !flat {
            self.warnings.count("cells");
        }

        let content = if flat {
            parts.into_iter().next().map(|part| part.children).unwrap_or_default()
        } else {
            let text = self.payload(id, true, true);
            let squeezed: String = text
                .split(['\t', '\n', '\u{c}', '\r', ' '])
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ");

            vec![Node::text(trim(&squeezed))]
        };
        let element = &self.elements[id];

        (Node::cell(element.tag == "th", content), span(element.attr("rowspan")), span(element.attr("colspan")))
    }

    fn grid(&mut self, rows: Vec<Vec<(Node, usize, usize)>>) -> Result<Vec<Vec<Node>>> {
        let count = rows.len();
        let mut slots: Vec<Vec<Option<Slot>>> = (0..count).map(|_| Vec::new()).collect();

        for (y, row) in rows.into_iter().enumerate() {
            let mut x = 0;

            for (item, down, across) in row {
                while slots[y].get(x).is_some_and(Option::is_some) {
                    x += 1;
                }

                if x + across > 1000 {
                    return Err(Failure::Table);
                }

                let header = item.header;
                let mut item = Some(item);

                for dy in 0..down.min(count - y) {
                    for dx in 0..across {
                        let line = &mut slots[y + dy];

                        if line.len() <= x + dx {
                            line.resize_with(x + dx + 1, || None);
                        }

                        line[x + dx] = Some(if dy > 0 || dx > 0 {
                            Slot { item: None, header }
                        } else {
                            Slot { item: item.take(), header }
                        });
                    }
                }

                x += across;
            }
        }

        let width = slots.iter().map(Vec::len).max().unwrap_or(0);
        let columns: Vec<usize> = (0..width)
            .filter(|x| {
                slots
                    .iter()
                    .any(|row| row.get(*x).is_some_and(|slot| slot.as_ref().is_some_and(|slot| slot.item.is_some())))
            })
            .collect();
        let mut result = Vec::new();

        for mut row in slots {
            let mut cells = Vec::new();

            for x in &columns {
                match row.get_mut(*x).and_then(Option::take) {
                    Some(Slot { item: Some(item), .. }) => cells.push(item),
                    Some(Slot { header, .. }) => {
                        self.warnings.count("merged");
                        cells.push(Node::cell(header, Vec::new()));
                    }
                    None => cells.push(Node::cell(false, Vec::new())),
                }
            }

            result.push(cells);
        }

        Ok(result)
    }

    fn table(&mut self, children: Vec<Projected>) -> Result<Vec<Node>> {
        let mut rows = Vec::new();
        let mut caption = Vec::new();

        for child in children {
            match child {
                Projected::Row(cells) => rows.push(cells),
                Projected::Node(node) if !is_blank(&visible(&node)) => caption.push(node),
                _ => {}
            }
        }

        if rows.len() > 100 {
            return Err(Failure::Table);
        }

        let mut result = blocks(caption);
        let cells = self.grid(rows)?;

        if cells.first().is_some_and(|row| !row.is_empty()) {
            result.push(table(cells)?);
        }

        Ok(result)
    }

    fn project(&mut self, id: usize) -> Result<Vec<Projected>> {
        let tag = self.elements[id].tag.clone();
        let code = tag == "pre" || tag == "code";
        let mut children = Vec::new();

        if !code {
            for child in self.elements[id].children.clone() {
                children.extend(self.project(child)?);
            }
        }

        let element = &self.elements[id];

        let nodes =
            |children: Vec<Projected>| -> Vec<Node> { children.into_iter().filter_map(Projected::node).collect() };
        let plain = |nodes: Vec<Node>| -> Vec<Projected> { nodes.into_iter().map(Projected::Node).collect() };

        let result = match tag.as_str() {
            "#text" => vec![Node::text(element.value.clone())],
            "root" | "tbody" | "thead" | "tfoot" => return Ok(children),
            "tr" => {
                let cells = children
                    .into_iter()
                    .filter_map(|child| match child {
                        Projected::Cell(cell, down, across) => Some((cell, down, across)),
                        _ => None,
                    })
                    .collect();

                return Ok(vec![Projected::Row(cells)]);
            }
            "td" | "th" => {
                let (cell, down, across) = self.cell(id, nodes(children));

                return Ok(vec![Projected::Cell(cell, down, across)]);
            }
            "code" => {
                let value = self.payload(id, false, false);

                if value.is_empty() { Vec::new() } else { vec![Node::leaf(Kind::InlineCode, value)] }
            }
            "pre" => {
                let class = element
                    .children
                    .iter()
                    .find(|child| self.elements[**child].tag == "code")
                    .and_then(|child| self.elements[*child].attr("class"))
                    .and_then(|class| class.split(is_space).find_map(|name| name.strip_prefix("language-")))
                    .unwrap_or("");
                let name = language(class);
                let value = self.payload(id, false, false);
                let value = value.strip_suffix('\n').unwrap_or(&value).to_owned();

                vec![Node::code_block(value, name)]
            }
            "table" => return Ok(plain(self.table(children)?)),
            "img" => {
                self.warnings.count("images");

                match element.attr("alt") {
                    Some(alt) if !alt.is_empty() => vec![Node::text(alt)],
                    _ => Vec::new(),
                }
            }
            "checkbox" => Vec::new(),
            "br" => vec![Node::new(Kind::Break)],
            "hr" => vec![Node::new(Kind::ThematicBreak)],
            "a" => {
                let href = element.attr("href").unwrap_or("").to_owned();
                let title = link_title(element.attr("title").unwrap_or(""));
                let children = nodes(children);

                match url(&href, self.warnings) {
                    Some(href) => decorate(children, &Node::link(href, title, Vec::new())),
                    None => children,
                }
            }
            _ => self.container(id, &tag, nodes(children)),
        };

        Ok(plain(result))
    }

    fn container(&mut self, id: usize, tag: &str, children: Vec<Node>) -> Vec<Node> {
        let element = &self.elements[id];

        if let Some(kind) = emphasis(tag) {
            return decorate(children, &Node::new(kind));
        }

        if let Some(level) = heading(tag) {
            return blocks(children)
                .into_iter()
                .map(|child| {
                    if child.is(Kind::Paragraph) {
                        Node { level, ..Node::parent(Kind::Heading, child.children) }
                    } else {
                        child
                    }
                })
                .collect();
        }

        match tag {
            "blockquote" => return vec![Node::parent(Kind::Quote, blocks(children))],
            "li" => {
                if element.has("value") {
                    self.warnings.count("counters");
                }

                let checked = element
                    .children
                    .iter()
                    .find(|child| self.elements[**child].tag == "checkbox")
                    .map(|child| self.elements[*child].checked);

                return vec![Node { checked, ..Node::parent(Kind::ListItem, blocks(children)) }];
            }
            "ul" | "ol" => return vec![self.list(id, tag == "ol", children)],
            _ => {}
        }

        if BLOCKS.contains(&tag) {
            if !matches!(tag, "p" | "div") {
                self.warnings.count("containers");
            }

            return blocks(children);
        }

        if !WRAPPERS.contains(&tag) {
            self.warnings.count("elements");
        }

        let marks = element.marks.clone();

        marks.into_iter().fold(children, |result, mark| decorate(result, &Node::new(mark)))
    }

    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "start is a whole number in 1..=999999"
    )]
    fn list(&mut self, id: usize, ordered: bool, children: Vec<Node>) -> Node {
        let element = &self.elements[id];
        let mut start = number(element.attr("start").unwrap_or("1"));
        let numbered = element.children.iter().any(|child| {
            let child = &self.elements[*child];

            child.tag == "li" && child.has("value")
        });

        if element.has("reversed") || start.fract() != 0.0 || !(1.0..=999_999.0).contains(&start) {
            start = 1.0;
            self.warnings.count("counters");
        }

        if numbered {
            start = 1.0;
        }

        let (mut items, outside): (Vec<Node>, Vec<Node>) =
            children.into_iter().partition(|child| child.is(Kind::ListItem));
        let outside: Vec<Node> = outside.into_iter().filter(|child| !is_blank(&visible(child))).collect();

        if !outside.is_empty() {
            items.insert(0, Node::parent(Kind::ListItem, blocks(outside)));
            self.warnings.count("elements");
        }

        Node::list(ordered, if ordered { start as u32 } else { 1 }, items)
    }
}

#[derive(Debug)]
enum Projected {
    Node(Node),
    Row(Vec<(Node, usize, usize)>),
    Cell(Node, usize, usize),
}

impl Projected {
    fn node(self) -> Option<Node> {
        match self {
            Projected::Node(node) => Some(node),
            Projected::Row(_) | Projected::Cell(..) => None,
        }
    }
}

fn decorate(children: Vec<Node>, wrapper: &Node) -> Vec<Node> {
    let mut result = Vec::new();
    let mut pending = Vec::new();

    for mut child in children {
        if child.kind.is_inline() {
            pending.push(child);
            continue;
        }

        if !pending.is_empty() {
            result.push(Node { children: std::mem::take(&mut pending), ..wrapper.clone() });
        }

        if child.kind.is_container() {
            child.children = decorate(std::mem::take(&mut child.children), wrapper);
        }

        result.push(child);
    }

    if !pending.is_empty() {
        result.push(Node { children: pending, ..wrapper.clone() });
    }

    result
}

fn doctype(source: &str) -> bool {
    let rest = source.trim_start_matches(is_space);

    rest.get(..9).is_some_and(|start| start.eq_ignore_ascii_case("<!doctype"))
        && rest[9..].chars().next().is_some_and(is_space)
        && rest[9..].contains('>')
}

pub fn parse(source: &str, bounds: Bounds, warnings: &mut Warnings) -> Result<Node> {
    if source.matches('<').count() > bounds.tags {
        return Err(Failure::HtmlLimit);
    }

    if doctype(source) {
        warnings.count("declarations");
    }

    let elements = read(source, bounds, warnings)?;
    let mut projector = Projector { elements, warnings };
    let children = projector.project(0)?.into_iter().filter_map(Projected::node).collect();
    let tree = Node::parent(Kind::Document, blocks(children));

    check(&tree)?;
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::{Format, serialize};

    fn html(source: &str) -> (String, Warnings) {
        let mut warnings = Warnings::default();
        let mut tree = parse(source, BOUNDS, &mut warnings).unwrap();

        crate::clean::clean_tree(&mut tree);
        crate::normalize::normalize(&mut tree, &mut warnings);

        (serialize(&tree, Format::Html, &mut Warnings::default(), false).unwrap(), warnings)
    }

    #[test]
    fn keeps_structure_drops_noise() {
        let (result, warnings) = html(
            "<p style=\"color:red\" onclick=\"x()\">a <b>b</b><script>x</script></p><ul><li><input type=checkbox checked>c</li></ul>",
        );

        assert_eq!(
            result,
            "<p>a <strong>b</strong></p>\n<ul><li><input type=\"checkbox\" disabled checked> c</li></ul>"
        );
        assert!(warnings.has("active"));
        assert!(warnings.has("attributes"));
    }

    #[test]
    fn reads_styled_spans() {
        let (result, _) =
            html("<span style=\"font-weight:700;font-style:italic\">a</span> <b style=\"font-weight:normal\">b</b>");

        assert_eq!(result, "<p><em><strong>a</strong></em> b</p>");
    }

    #[test]
    fn splits_merged_cells() {
        let (result, warnings) = html("<table><tr><th colspan=2>a</th></tr><tr><td>b</td><td>c</td></tr></table>");

        assert!(result.starts_with("<table>\n<thead>\n<tr><th>a</th><th></th></tr>"));
        assert!(warnings.has("merged"));
    }

    #[test]
    fn treats_orphan_cells_as_text() {
        let (result, _) = html("<td>a</td><li>b</li>");

        assert_eq!(result, "<p>a</p>\n<p>b</p>");
    }
}
