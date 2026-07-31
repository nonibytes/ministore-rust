use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};
use yaml_rust2::Yaml;

use crate::document::parse_finding;
use crate::finding::{Finding, FindingCode, Severity};

const RECOGNIZED_KEYS: &[&str] = &[
    "type",
    "title",
    "description",
    "resource",
    "tags",
    "sources",
    "usage_window",
    "generated",
    "verified",
    "status",
    "stale_after",
    "timestamp",
    "runtime",
    "parameters",
    "computation",
    "executor",
    "attester",
    "okf_version",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarKind {
    String,
    Timestamp,
    Boolean,
    Integer,
    Float,
    Null,
    Tagged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scalar {
    pub value: String,
    pub kind: ScalarKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Scalar(Scalar),
    Sequence(Vec<Node>),
    Mapping(Vec<(Node, Node)>),
    Alias(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub position: Position,
}

impl Node {
    pub fn as_string(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Scalar(Scalar {
                value,
                kind: ScalarKind::String,
            }) => Some(value),
            _ => None,
        }
    }

    pub fn as_date(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Scalar(Scalar {
                value,
                kind: ScalarKind::String | ScalarKind::Timestamp,
            }) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Metadata {
    root: Node,
    first: HashMap<String, usize>,
    anchors: HashMap<usize, Node>,
}

impl Metadata {
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Return the first value for an exact string top-level key.
    pub fn get(&self, key: &str) -> Option<&Node> {
        let NodeKind::Mapping(entries) = &self.root.kind else {
            return None;
        };
        self.first.get(key).map(|index| &entries[*index].1)
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key)
            .and_then(|node| self.resolve(node))
            .and_then(Node::as_string)
    }

    pub fn date(&self, key: &str) -> Option<&str> {
        self.get(key)
            .and_then(|node| self.resolve(node))
            .and_then(Node::as_date)
    }

    /// Resolve an alias chain without expanding recursive aliases.
    pub fn resolve<'a>(&'a self, mut node: &'a Node) -> Option<&'a Node> {
        for _ in 0..=self.anchors.len() {
            let NodeKind::Alias(anchor) = node.kind else {
                return Some(node);
            };
            node = self.anchors.get(&anchor)?;
        }
        None
    }

    pub fn node_string<'a>(&'a self, node: &'a Node) -> Option<&'a str> {
        self.resolve(node).and_then(Node::as_string)
    }

    pub fn node_date<'a>(&'a self, node: &'a Node) -> Option<&'a str> {
        self.resolve(node).and_then(Node::as_date)
    }

    /// Return the first exact string key from a mapping, resolving aliases in
    /// both keys and the returned value.
    pub fn mapping_get<'a>(&'a self, mapping: &'a Node, key: &str) -> Option<&'a Node> {
        let mapping = self.resolve(mapping)?;
        let NodeKind::Mapping(entries) = &mapping.kind else {
            return None;
        };
        entries.iter().find_map(|(entry_key, value)| {
            (self.node_string(entry_key) == Some(key))
                .then(|| self.resolve(value))
                .flatten()
        })
    }

    /// Return string values from a scalar or sequence. `valid` is false when
    /// any sequence entry is not a YAML string scalar.
    pub fn strings(&self, key: &str) -> (Vec<&str>, CollectionForm, bool) {
        let Some(node) = self.get(key).and_then(|node| self.resolve(node)) else {
            return (Vec::new(), CollectionForm::Missing, false);
        };
        if let Some(value) = node.as_string() {
            return (vec![value], CollectionForm::Scalar, true);
        }
        let NodeKind::Sequence(items) = &node.kind else {
            return (Vec::new(), collection_form(node), false);
        };
        let mut values = Vec::with_capacity(items.len());
        let mut valid = true;
        for item in items {
            if let Some(value) = self.resolve(item).and_then(Node::as_string) {
                values.push(value);
            } else {
                valid = false;
            }
        }
        (values, CollectionForm::Sequence, valid)
    }

    /// Return a mapping as one entry or mappings from a sequence. `valid` is
    /// false when any sequence entry is not a mapping.
    pub fn mappings(&self, key: &str) -> (Vec<&Node>, CollectionForm, bool) {
        let Some(node) = self.get(key).and_then(|node| self.resolve(node)) else {
            return (Vec::new(), CollectionForm::Missing, false);
        };
        if matches!(node.kind, NodeKind::Mapping(_)) {
            return (vec![node], CollectionForm::Mapping, true);
        }
        let NodeKind::Sequence(items) = &node.kind else {
            return (Vec::new(), collection_form(node), false);
        };
        let mut values = Vec::with_capacity(items.len());
        let mut valid = true;
        for item in items {
            if let Some(item) = self.resolve(item) {
                if matches!(item.kind, NodeKind::Mapping(_)) {
                    values.push(item);
                } else {
                    valid = false;
                }
            } else {
                valid = false;
            }
        }
        (values, CollectionForm::Sequence, valid)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionForm {
    Missing,
    Scalar,
    Sequence,
    Mapping,
    Other,
}

fn collection_form(node: &Node) -> CollectionForm {
    match node.kind {
        NodeKind::Scalar(_) => CollectionForm::Scalar,
        NodeKind::Sequence(_) => CollectionForm::Sequence,
        NodeKind::Mapping(_) => CollectionForm::Mapping,
        NodeKind::Alias(_) => CollectionForm::Other,
    }
}

pub(crate) fn parse_metadata(
    path: &str,
    frontmatter: &str,
) -> std::result::Result<(Metadata, Vec<Finding>), Finding> {
    let mut builder = TreeBuilder::default();
    let mut parser = Parser::new_from_str(frontmatter);
    if let Err(error) = parser.load(&mut builder, true) {
        return Err(Finding {
            severity: Severity::Error,
            code: FindingCode::OKF103,
            path: path.to_owned(),
            line: Some(error.marker().line() + 2),
            column: Some(error.marker().col() + 1),
            spec_section: Some("4.1".to_owned()),
            message: format!("frontmatter is not parseable YAML: {}", error.info()),
        });
    }
    if let Some((message, position)) = builder.error {
        return Err(parse_finding(
            Severity::Error,
            FindingCode::OKF103,
            path,
            position.line,
            position.column,
            format!("frontmatter is not parseable YAML: {message}"),
        ));
    }
    if builder.documents.len() != 1 {
        return Err(parse_finding(
            Severity::Error,
            if builder.documents.is_empty() {
                FindingCode::OKF104
            } else {
                FindingCode::OKF103
            },
            path,
            2,
            1,
            if builder.documents.is_empty() {
                "frontmatter root is not a mapping"
            } else {
                "frontmatter contains multiple YAML documents"
            },
        ));
    }

    let root = builder.documents.pop().expect("document length checked");
    let NodeKind::Mapping(entries) = &root.kind else {
        return Err(parse_finding(
            Severity::Error,
            FindingCode::OKF104,
            path,
            root.position.line,
            root.position.column,
            "frontmatter root is not a mapping",
        ));
    };

    let mut first = HashMap::new();
    let mut findings = Vec::new();
    for (index, (key_node, _)) in entries.iter().enumerate() {
        let Some(key) = resolve_node(key_node, &builder.anchors).and_then(Node::as_string) else {
            continue;
        };
        if first.contains_key(key) {
            if RECOGNIZED_KEYS.contains(&key) {
                findings.push(parse_finding(
                    Severity::Warning,
                    FindingCode::OKF106,
                    path,
                    key_node.position.line,
                    key_node.position.column,
                    format!("recognized top-level key {key:?} occurs more than once"),
                ));
            }
        } else {
            first.insert(key.to_owned(), index);
        }
    }

    Ok((
        Metadata {
            root,
            first,
            anchors: builder.anchors,
        },
        findings,
    ))
}

#[derive(Debug)]
enum Frame {
    Sequence {
        values: Vec<Node>,
        anchor: usize,
        position: Position,
    },
    Mapping {
        entries: Vec<(Node, Node)>,
        key: Option<Node>,
        anchor: usize,
        position: Position,
    },
}

#[derive(Default)]
struct TreeBuilder {
    documents: Vec<Node>,
    current: Option<Node>,
    frames: Vec<Frame>,
    anchors: HashMap<usize, Node>,
    error: Option<(String, Position)>,
}

impl TreeBuilder {
    fn insert(&mut self, node: Node, anchor: usize) {
        if anchor > 0 {
            self.anchors.insert(anchor, node.clone());
        }
        match self.frames.last_mut() {
            Some(Frame::Sequence { values, .. }) => values.push(node),
            Some(Frame::Mapping { entries, key, .. }) => {
                if let Some(entry_key) = key.take() {
                    entries.push((entry_key, node));
                } else {
                    *key = Some(node);
                }
            }
            None if self.current.is_none() => self.current = Some(node),
            None => self.fail(
                "multiple roots in one YAML document",
                Position { line: 2, column: 1 },
            ),
        }
    }

    fn fail(&mut self, message: impl Into<String>, position: Position) {
        if self.error.is_none() {
            self.error = Some((message.into(), position));
        }
    }
}

impl MarkedEventReceiver for TreeBuilder {
    fn on_event(&mut self, event: Event, marker: Marker) {
        if self.error.is_some() {
            return;
        }
        let position = Position {
            line: marker.line() + 2,
            column: marker.col() + 1,
        };
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd => {}
            Event::DocumentStart => {
                self.current = None;
                self.frames.clear();
                self.anchors.clear();
            }
            Event::DocumentEnd => {
                if !self.frames.is_empty() {
                    self.fail("unterminated YAML container", position);
                } else if let Some(root) = self.current.take() {
                    self.documents.push(root);
                }
            }
            Event::Alias(anchor) => self.insert(
                Node {
                    kind: NodeKind::Alias(anchor),
                    position,
                },
                0,
            ),
            Event::Scalar(value, style, anchor, tag) => {
                let kind = scalar_kind(&value, style, tag.as_ref());
                self.insert(
                    Node {
                        kind: NodeKind::Scalar(Scalar { value, kind }),
                        position,
                    },
                    anchor,
                );
            }
            Event::SequenceStart(anchor, _) => self.frames.push(Frame::Sequence {
                values: Vec::new(),
                anchor,
                position,
            }),
            Event::SequenceEnd => match self.frames.pop() {
                Some(Frame::Sequence {
                    values,
                    anchor,
                    position,
                }) => self.insert(
                    Node {
                        kind: NodeKind::Sequence(values),
                        position,
                    },
                    anchor,
                ),
                _ => self.fail("unexpected YAML sequence end", position),
            },
            Event::MappingStart(anchor, _) => self.frames.push(Frame::Mapping {
                entries: Vec::new(),
                key: None,
                anchor,
                position,
            }),
            Event::MappingEnd => match self.frames.pop() {
                Some(Frame::Mapping {
                    entries,
                    key: None,
                    anchor,
                    position,
                }) => self.insert(
                    Node {
                        kind: NodeKind::Mapping(entries),
                        position,
                    },
                    anchor,
                ),
                Some(Frame::Mapping {
                    position: start, ..
                }) => self.fail("YAML mapping key has no value", start),
                _ => self.fail("unexpected YAML mapping end", position),
            },
        }
    }
}

fn resolve_node<'a>(mut node: &'a Node, anchors: &'a HashMap<usize, Node>) -> Option<&'a Node> {
    for _ in 0..=anchors.len() {
        let NodeKind::Alias(anchor) = node.kind else {
            return Some(node);
        };
        node = anchors.get(&anchor)?;
    }
    None
}

fn scalar_kind(value: &str, style: TScalarStyle, tag: Option<&Tag>) -> ScalarKind {
    if let Some(tag) = tag {
        if tag.handle == "tag:yaml.org,2002:" {
            return match tag.suffix.as_str() {
                "str" => ScalarKind::String,
                "timestamp" => ScalarKind::Timestamp,
                "bool" => ScalarKind::Boolean,
                "int" => ScalarKind::Integer,
                "float" => ScalarKind::Float,
                "null" => ScalarKind::Null,
                _ => ScalarKind::Tagged,
            };
        }
        return ScalarKind::Tagged;
    }
    if style != TScalarStyle::Plain {
        return ScalarKind::String;
    }
    match Yaml::from_str(value) {
        Yaml::String(_) if is_yaml_timestamp(value) => ScalarKind::Timestamp,
        Yaml::String(_) => ScalarKind::String,
        Yaml::Boolean(_) => ScalarKind::Boolean,
        Yaml::Integer(_) => ScalarKind::Integer,
        Yaml::Real(_) => ScalarKind::Float,
        Yaml::Null => ScalarKind::Null,
        _ => ScalarKind::Tagged,
    }
}

fn is_yaml_timestamp(value: &str) -> bool {
    if value.len() < 6 || value.as_bytes().get(4) != Some(&b'-') {
        return false;
    }
    NaiveDate::parse_from_str(value, "%Y-%-m-%-d").is_ok()
        || DateTime::parse_from_rfc3339(value).is_ok()
        || NaiveDateTime::parse_from_str(value, "%Y-%-m-%-d %H:%M:%S%.f").is_ok()
        || DateTime::parse_from_str(value, "%Y-%-m-%-dT%H:%M:%S%.f%#z").is_ok()
        || DateTime::parse_from_str(value, "%Y-%-m-%-dt%H:%M:%S%.f%#z").is_ok()
}
