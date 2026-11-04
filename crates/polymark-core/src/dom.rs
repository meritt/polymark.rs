use std::borrow::Cow;
use std::cell::{Ref, RefCell};

use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, ParseOpts, QualName, local_name, ns, parse_fragment};

#[derive(Debug)]
pub enum Data {
    Document,
    Fragment,
    Element { name: QualName, attrs: Vec<Attribute>, contents: Option<usize> },
    Text(String),
    Comment,
    Other,
}

#[derive(Debug)]
pub struct Dom {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub data: Data,
}

#[derive(Debug)]
struct Sink {
    nodes: RefCell<Vec<Dom>>,
}

impl Sink {
    fn push(&self, data: Data) -> usize {
        let mut nodes = self.nodes.borrow_mut();

        nodes.push(Dom { parent: None, children: Vec::new(), data });
        nodes.len() - 1
    }

    fn detach(&self, node: usize) {
        let mut nodes = self.nodes.borrow_mut();

        if let Some(parent) = nodes[node].parent.take() {
            nodes[parent].children.retain(|child| *child != node);
        }
    }

    fn insert(&self, parent: usize, index: usize, child: NodeOrText<usize>) {
        match child {
            NodeOrText::AppendText(text) => {
                let mut nodes = self.nodes.borrow_mut();
                let before = index.checked_sub(1).map(|at| nodes[parent].children[at]);

                if let Some(before) = before
                    && let Data::Text(value) = &mut nodes[before].data
                {
                    value.push_str(&text);
                    return;
                }

                drop(nodes);

                let node = self.push(Data::Text(text.to_string()));
                let mut nodes = self.nodes.borrow_mut();

                nodes[node].parent = Some(parent);
                nodes[parent].children.insert(index, node);
            }
            NodeOrText::AppendNode(node) => {
                self.detach(node);

                let mut nodes = self.nodes.borrow_mut();
                let index = index.min(nodes[parent].children.len());

                nodes[node].parent = Some(parent);
                nodes[parent].children.insert(index, node);
            }
        }
    }
}

impl TreeSink for Sink {
    type Handle = usize;
    type Output = Vec<Dom>;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Vec<Dom> {
        self.nodes.into_inner()
    }

    fn parse_error(&self, _message: Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.nodes.borrow(), |nodes| match &nodes[*target].data {
            Data::Element { name, .. } => name,
            _ => panic!("only elements have names"),
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        let contents = flags.template.then(|| self.push(Data::Fragment));

        self.push(Data::Element { name, attrs, contents })
    }

    fn create_comment(&self, _text: StrTendril) -> usize {
        self.push(Data::Comment)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> usize {
        self.push(Data::Other)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        let index = self.nodes.borrow()[*parent].children.len();

        self.insert(*parent, index, child);
    }

    fn append_based_on_parent_node(&self, element: &usize, previous: &usize, child: NodeOrText<usize>) {
        if self.nodes.borrow()[*element].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(previous, child);
        }
    }

    fn append_doctype_to_document(&self, _name: StrTendril, _public: StrTendril, _system: StrTendril) {}

    fn get_template_contents(&self, target: &usize) -> usize {
        match &self.nodes.borrow()[*target].data {
            Data::Element { contents: Some(contents), .. } => *contents,
            _ => panic!("only templates have contents"),
        }
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &usize, child: NodeOrText<usize>) {
        if let NodeOrText::AppendNode(node) = &child {
            self.detach(*node);
        }

        let nodes = self.nodes.borrow();
        let Some(parent) = nodes[*sibling].parent else {
            return;
        };
        let index = nodes[parent].children.iter().position(|child| child == sibling).unwrap_or(0);

        drop(nodes);
        self.insert(parent, index, child);
    }

    fn add_attrs_if_missing(&self, target: &usize, extra: Vec<Attribute>) {
        if let Data::Element { attrs, .. } = &mut self.nodes.borrow_mut()[*target].data {
            for attribute in extra {
                if !attrs.iter().any(|known| known.name == attribute.name) {
                    attrs.push(attribute);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        self.detach(*target);
    }

    fn reparent_children(&self, node: &usize, parent: &usize) {
        let children = std::mem::take(&mut self.nodes.borrow_mut()[*node].children);

        for child in children {
            self.nodes.borrow_mut()[child].parent = None;
            self.append(parent, NodeOrText::AppendNode(child));
        }
    }
}

pub fn fragment(source: &str) -> (Vec<Dom>, usize) {
    let sink = Sink { nodes: RefCell::new(vec![Dom { parent: None, children: Vec::new(), data: Data::Document }]) };
    let context = QualName::new(None, ns!(html), local_name!("template"));
    let nodes = parse_fragment(sink, ParseOpts::default(), context, Vec::new(), false).one(source);
    let root = nodes[0].children.first().copied().unwrap_or(0);

    (nodes, root)
}
