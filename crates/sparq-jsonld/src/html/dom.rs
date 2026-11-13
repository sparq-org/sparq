//! Minimal flat tree for HTML document order, namespaces, and template boundaries.
//! HTML parsing and insertion modes are entirely delegated to html5ever.

use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, QualName};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use super::Contents;

#[derive(Clone)]
pub(super) struct Handle {
    index: usize,
    name: Option<Rc<QualName>>,
}

#[derive(Default)]
pub(super) struct Node {
    name: Option<Rc<QualName>>,
    attributes: Vec<Attribute>,
    text: Option<String>,
    parent: Option<usize>,
    children: Vec<Handle>,
    template: Option<Handle>,
    mathml_integration: bool,
}

struct Sink(RefCell<Vec<Node>>);

impl Sink {
    fn create(&self, node: Node) -> Handle {
        let mut nodes = self.0.borrow_mut();
        let handle = Handle {
            index: nodes.len(),
            name: node.name.clone(),
        };
        nodes.push(node);
        handle
    }

    fn detach(nodes: &mut [Node], index: usize) {
        if let Some(parent) = nodes[index].parent.take() {
            nodes[parent].children.retain(|child| child.index != index);
        }
    }

    fn insert(&self, parent: usize, child: NodeOrText<Handle>, before: Option<usize>) {
        let mut nodes = self.0.borrow_mut();
        let position = |nodes: &[Node]| {
            before
                .and_then(|index| {
                    nodes[parent]
                        .children
                        .iter()
                        .position(|child| child.index == index)
                })
                .unwrap_or(nodes[parent].children.len())
        };
        let child = match child {
            NodeOrText::AppendNode(child) => {
                if before == Some(child.index) {
                    return;
                }
                Self::detach(&mut nodes, child.index);
                child
            }
            NodeOrText::AppendText(text) => {
                let index = position(&nodes);
                if let Some(previous) = index
                    .checked_sub(1)
                    .map(|i| nodes[parent].children[i].index)
                {
                    if let Some(contents) = &mut nodes[previous].text {
                        contents.push_str(&text);
                        return;
                    }
                }
                let handle = Handle {
                    index: nodes.len(),
                    name: None,
                };
                nodes.push(Node {
                    text: Some(text.to_string()),
                    ..Node::default()
                });
                handle
            }
        };
        let index = position(&nodes);
        nodes[child.index].parent = Some(parent);
        nodes[parent].children.insert(index, child);
    }
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Vec<Node>;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        self.0.into_inner()
    }
    fn parse_error(&self, _: Cow<'static, str>) {}
    fn get_document(&self) -> Handle {
        Handle {
            index: 0,
            name: None,
        }
    }
    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        target
            .name
            .as_deref()
            .expect("tree builder requested an element name")
    }
    fn create_element(
        &self,
        name: QualName,
        attributes: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Handle {
        let template = flags.template.then(|| self.create(Node::default()));
        self.create(Node {
            name: Some(Rc::new(name)),
            attributes,
            template,
            mathml_integration: flags.mathml_annotation_xml_integration_point,
            ..Node::default()
        })
    }
    fn create_comment(&self, _: StrTendril) -> Handle {
        self.create(Node::default())
    }
    fn create_pi(&self, _: StrTendril, _: StrTendril) -> Handle {
        self.create(Node::default())
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.insert(parent.index, child, None);
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        previous: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let attached = self.0.borrow()[element.index].parent.is_some();
        if attached {
            self.append_before_sibling(element, child);
        } else {
            self.append(previous, child);
        }
    }
    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}
    fn get_template_contents(&self, target: &Handle) -> Handle {
        self.0.borrow()[target.index]
            .template
            .clone()
            .expect("template contents")
    }
    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.index == y.index
    }
    fn set_quirks_mode(&self, _: QuirksMode) {}
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let parent = self.0.borrow()[sibling.index].parent;
        if let Some(parent) = parent {
            self.insert(parent, child, Some(sibling.index));
        }
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.0.borrow_mut();
        let attributes = &mut nodes[target.index].attributes;
        for attribute in attrs {
            if !attributes
                .iter()
                .any(|existing| existing.name == attribute.name)
            {
                attributes.push(attribute);
            }
        }
    }
    fn remove_from_parent(&self, target: &Handle) {
        Self::detach(&mut self.0.borrow_mut(), target.index);
    }
    fn reparent_children(&self, node: &Handle, parent: &Handle) {
        let children = std::mem::take(&mut self.0.borrow_mut()[node.index].children);
        for child in children {
            self.append(parent, NodeOrText::AppendNode(child));
        }
    }
    fn is_mathml_annotation_xml_integration_point(&self, target: &Handle) -> bool {
        self.0.borrow()[target.index].mathml_integration
    }
}

pub(super) fn contents(text: &str) -> Contents {
    let nodes = html5ever::parse_document(
        Sink(RefCell::new(vec![Node::default()])),
        Default::default(),
    )
    .one(text);
    let mut doc = Contents::default();
    let mut stack = vec![0];
    while let Some(index) = stack.pop() {
        let node = &nodes[index];
        stack.extend(node.children.iter().rev().map(|child| child.index));
        let Some(name) = &node.name else { continue };
        let attr = |key: &str| {
            node.attributes
                .iter()
                .find(|attribute| attribute.name.ns.is_empty() && &*attribute.name.local == key)
                .map(|attribute| attribute.value.to_string())
        };
        let html = &*name.ns == "http://www.w3.org/1999/xhtml";
        let script = html && &*name.local == "script";
        let id = attr("id");
        if let Some(id) = &id {
            doc.ids
                .push((id.clone(), script.then_some(doc.scripts.len())));
        }
        if html && &*name.local == "base" && doc.base.is_none() {
            doc.base = attr("href");
        }
        if script {
            let valid = attr("type").is_some_and(|t| {
                t.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/ld+json")
            });
            let mut text = String::new();
            let mut descendants: Vec<_> = node
                .children
                .iter()
                .rev()
                .map(|child| child.index)
                .collect();
            while let Some(child) = descendants.pop() {
                if let Some(contents) = &nodes[child].text {
                    text.push_str(contents);
                }
                descendants.extend(nodes[child].children.iter().rev().map(|child| child.index));
            }
            doc.scripts.push((id, valid, text));
        }
    }
    doc
}
