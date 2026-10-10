//! Bounded accessibility observations shared by application adapters.
use crate::{ElementBox, ElementQuery};
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Node {
    attrs: HashMap<String, String>,
    pub(crate) is_hierarchy_wrapper: bool,
    pub parent: Option<usize>,
}

impl Node {
    /// Missing attributes stay distinct from observed empty strings.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    pub fn attributes(&self) -> &HashMap<String, String> {
        &self.attrs
    }

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
                        // Preserve XML provenance: a real node's class may also be "hierarchy".
                        is_hierarchy_wrapper: parents.is_empty()
                            && start.name().as_ref() == b"hierarchy",
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

    /// The snapshot re-serialised for operator review. Password fields and editable text
    /// lose their `text`: they can hold a typed credential, comment or caption draft.
    pub(crate) fn redacted_xml(&self) -> String {
        fn escape(value: &str) -> String {
            value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        }
        let tag = |index: usize| {
            if self.nodes[index].is_hierarchy_wrapper {
                "hierarchy"
            } else {
                "node"
            }
        };
        let mut out = String::from("<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>\n");
        let mut open: Vec<usize> = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            while let Some(top) = open.last().copied().filter(|top| node.parent != Some(*top)) {
                open.pop();
                out.push_str(&format!("</{}>", tag(top)));
            }
            let secret = node.attr("password") == "true" || node.attr("class").contains("EditText");
            let mut keys: Vec<_> = node.attrs.keys().collect();
            keys.sort();
            out.push('<');
            out.push_str(tag(index));
            for key in keys {
                let value = if secret && key == "text" {
                    String::new()
                } else {
                    escape(node.attr(key))
                };
                out.push_str(&format!(" {}=\"{value}\"", escape(key)));
            }
            out.push('>');
            open.push(index);
        }
        while let Some(index) = open.pop() {
            out.push_str(&format!("</{}>", tag(index)));
        }
        out
    }
}

/// A multi-window hierarchy reduced to one package's windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSelection {
    /// `<hierarchy>` with only the kept window roots, byte-for-byte otherwise.
    pub xml: String,
    /// The `package` of every window root the server returned, in order.
    pub windows: Vec<String>,
    /// How many of them were kept.
    pub kept: usize,
}

/// Keep only the window roots of `package` in a UiAutomator2 multi-window hierarchy.
///
/// With `enableMultiWindows` the pinned server (appium-uiautomator2-server v10.6.2) dumps one
/// root per `UiAutomation.getWindows()` entry as a direct child of `<hierarchy>`, each carrying
/// its own `package`: the app, the system UI bars, an IME. The sheet classifiers were measured
/// on one-window trees, so the other windows are cut here, whole subtrees at a time, before
/// any of them reads the tree. A window root without a `package` is never the app's.
pub fn keep_package_windows(xml: &str, package: &str) -> anyhow::Result<WindowSelection> {
    use quick_xml::Writer;
    anyhow::ensure!(!package.is_empty(), "window package missing");
    anyhow::ensure!(
        xml.len() <= 16 * 1024 * 1024,
        "multi-window snapshot size limit"
    );
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut windows = Vec::new();
    let mut kept = 0usize;
    // Depth counts open elements; 0 is outside `<hierarchy>`, 1 is a window root.
    let mut depth = 0usize;
    // Depth at which a dropped window started; everything deeper is skipped.
    let mut dropping: Option<usize> = None;
    let mut roots_seen = 0usize;
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                anyhow::ensure!(depth < 256, "multi-window snapshot nesting limit");
                let empty = matches!(event, Event::Empty(_));
                if depth == 0 {
                    anyhow::ensure!(
                        start.name().as_ref() == b"hierarchy" && roots_seen == 0,
                        "multi-window snapshot does not start with one <hierarchy>"
                    );
                    roots_seen += 1;
                } else if depth == 1 && dropping.is_none() {
                    let mut window_package = String::new();
                    for attribute in start.attributes() {
                        let attribute = attribute?;
                        if attribute.key.as_ref() == b"package" {
                            window_package = attribute
                                .decoded_and_normalized_value(
                                    XmlVersion::Implicit1_0,
                                    reader.decoder(),
                                )?
                                .into_owned();
                        }
                    }
                    let keep = window_package == package;
                    windows.push(window_package);
                    if keep {
                        kept += 1;
                    } else {
                        dropping = Some(depth);
                    }
                }
                if dropping.is_none() {
                    writer.write_event(event.borrow())?;
                }
                if !empty {
                    depth += 1;
                } else if dropping == Some(depth) {
                    dropping = None;
                }
            }
            Event::End(_) => {
                anyhow::ensure!(depth > 0, "multi-window snapshot nesting");
                depth -= 1;
                if dropping.is_none() {
                    writer.write_event(event.borrow())?;
                } else if dropping == Some(depth) {
                    dropping = None;
                }
            }
            Event::DocType(_) => anyhow::bail!("doctype in multi-window snapshot"),
            Event::Eof => {
                anyhow::ensure!(depth == 0, "incomplete multi-window snapshot");
                anyhow::ensure!(roots_seen == 1, "multi-window snapshot without <hierarchy>");
                break;
            }
            // Declarations, text and whitespace between roots pass through outside a drop.
            _ => {
                if dropping.is_none() {
                    writer.write_event(event.borrow())?;
                }
            }
        }
    }
    let xml = String::from_utf8(writer.into_inner())?;
    Ok(WindowSelection { xml, windows, kept })
}
