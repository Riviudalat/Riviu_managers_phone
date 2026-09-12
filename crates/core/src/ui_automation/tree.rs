//! Bounded accessibility observations shared by application adapters.
use crate::{ElementBox, ElementQuery};
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Node {
    attrs: HashMap<String, String>,
    pub parent: Option<usize>,
}

impl Node {
    pub fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map(String::as_str).unwrap_or_default()
    }

    pub fn visible(&self, package: &str) -> bool {
        self.attr("package") == package
            && self.attr("displayed") != "false"
            && self.attr("visible-to-user") != "false"
    }
    /// Preserve missing visibility as unknown for consumers that require explicit proof.
    pub fn visibility(&self) -> Option<bool> {
        match (self.attr("displayed"), self.attr("visible-to-user")) {
            ("false", _) | (_, "false") => Some(false),
            ("true", _) | (_, "true") => Some(true),
            _ => None,
        }
    }

    pub fn rect(&self) -> Option<ElementBox> {
        let (a, b) = self
            .attr("bounds")
            .strip_prefix('[')?
            .strip_suffix(']')?
            .split_once("][")?;
        let (x, y) = a.split_once(',')?;
        let (r, b) = b.split_once(',')?;
        let (x, y, r, b) = (
            x.parse::<f64>().ok()?,
            y.parse::<f64>().ok()?,
            r.parse::<f64>().ok()?,
            b.parse::<f64>().ok()?,
        );
        if ![x, y, r, b].iter().all(|n| n.is_finite()) || x < 0.0 || y < 0.0 || r <= x || b <= y {
            return None;
        }
        Some(ElementBox {
            x,
            y,
            width: r - x,
            height: b - y,
            description: Some(self.attr("text").to_owned()),
            enabled: self.attr("enabled") == "true",
            clickable: self.attr("clickable") == "true",
        })
    }

    pub(crate) fn matches(&self, query: ElementQuery<'_>) -> bool {
        match query {
            ElementQuery::Semantic(_) => false,
            ElementQuery::Description { value, exact } => {
                let actual = self.attr("content-desc").to_lowercase();
                let expected = value.to_lowercase();
                if exact {
                    actual == expected
                } else {
                    actual.contains(&expected)
                }
            }
            ElementQuery::Text { value, exact } => {
                if exact {
                    self.attr("text") == value
                } else {
                    self.attr("text").contains(value)
                }
            }
            ElementQuery::ResourceIdSuffix(value) => self.attr("resource-id").ends_with(value),
            ElementQuery::ClassName(value) => self.attr("class") == value,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tree {
    pub generation: u64,
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn parse(snapshot: crate::HierarchySourceSnapshot) -> anyhow::Result<Self> {
        anyhow::ensure!(
            snapshot.generation > 0,
            "verification snapshot has no generation"
        );
        anyhow::ensure!(
            snapshot.xml.len() <= 16 * 1024 * 1024,
            "verification snapshot size limit"
        );
        let mut reader = Reader::from_str(&snapshot.xml);
        let mut nodes = Vec::new();
        let mut parents = Vec::new();
        loop {
            let event = reader.read_event()?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start) => {
                    anyhow::ensure!(
                        parents.len() < 256 && nodes.len() < 32768,
                        "verification snapshot structure limit"
                    );
                    let mut node = Node {
                        attrs: HashMap::new(),
                        parent: parents.last().copied(),
                    };
                    for attribute in start.attributes() {
                        let attribute = attribute?;
                        node.attrs.insert(
                            std::str::from_utf8(attribute.key.as_ref())?.to_owned(),
                            attribute
                                .decoded_and_normalized_value(
                                    XmlVersion::Implicit1_0,
                                    reader.decoder(),
                                )?
                                .into_owned(),
                        );
                    }
                    nodes.push(node);
                    if matches!(event, Event::Start(_)) {
                        parents.push(nodes.len() - 1);
                    }
                }
                Event::End(_) => {
                    anyhow::ensure!(parents.pop().is_some(), "verification snapshot nesting");
                }
                Event::DocType(_) => anyhow::bail!("doctype in verification snapshot"),
                Event::Eof => {
                    anyhow::ensure!(parents.is_empty(), "incomplete verification snapshot");
                    break;
                }
                _ => {}
            }
        }
        Ok(Self {
            generation: snapshot.generation,
            nodes,
        })
    }

    pub fn matching(&self, package: &str, query: ElementQuery<'_>) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.visible(package) && node.matches(query) && node.rect().is_some()
            })
            .filter(|(index, _)| self.ancestors_visible(*index))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn ancestors_visible(&self, index: usize) -> bool {
        let mut parent = self.nodes[index].parent;
        while let Some(index) = parent {
            let node = &self.nodes[index];
            if node.attr("displayed") == "false" || node.attr("visible-to-user") == "false" {
                return false;
            }
            parent = node.parent;
        }
        true
    }

    pub fn inside(&self, child: usize, ancestor: usize) -> bool {
        let mut current = Some(child);
        while let Some(index) = current {
            if index == ancestor {
                return true;
            }
            current = self.nodes[index].parent;
        }
        false
    }
}
