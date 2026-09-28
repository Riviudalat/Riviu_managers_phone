//! Read-only semantic evidence. Unlike `GuiNode`, missing bounds never discard a node.
use super::Rect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_OBSERVATION_BUDGET_MS: u64 = 120_000;

fn exact_default() -> bool {
    true
}

/// All supplied predicates are ANDed; string matching is case-sensitive and exact by default.
/// `id` is the full resource/accessibility identifier, never a suffix or an action target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticLocator {
    pub role: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
    pub id: Option<String>,
    #[serde(default = "exact_default")]
    pub exact: bool,
}

impl Default for SemanticLocator {
    fn default() -> Self {
        Self {
            role: None,
            name: None,
            text: None,
            id: None,
            exact: true,
        }
    }
}

/// A root selector must resolve uniquely before its inclusive subtree can be searched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationScope {
    pub package: Option<String>,
    pub root: Option<SemanticLocator>,
}

/// Projection only: fields required by the query must still be read before masking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ObservationFieldMask {
    pub identity: bool,
    pub semantics: bool,
    pub states: bool,
    pub bounds: bool,
    pub raw_attributes: bool,
}

impl Default for ObservationFieldMask {
    fn default() -> Self {
        Self {
            identity: true,
            semantics: true,
            states: true,
            bounds: true,
            raw_attributes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationRequest {
    #[serde(default)]
    pub query: SemanticLocator,
    pub scope: Option<ObservationScope>,
    #[serde(default)]
    pub fields: ObservationFieldMask,
    /// Remaining total budget, including all transport reads and parsing; zero performs no I/O.
    pub remaining_ms: u64,
}

impl ObservationRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.remaining_ms <= MAX_OBSERVATION_BUDGET_MS,
            "observation_budget_invalid"
        );
        Ok(())
    }
}

/// Optional metadata is evidence, not a default empty string or a guessed viewport.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedAppContext {
    pub package: Option<String>,
    pub activity: Option<String>,
    pub version: Option<String>,
    pub system_locale: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationSource {
    AccessibilityHierarchy,
    NativeQuery,
}

/// Complete means exhaustive for the requested query/scope, not that every field is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    /// Snapshot-local index; never reusable across observation IDs or generations.
    pub node_id: usize,
    pub parent: Option<usize>,
    pub id: Option<String>,
    pub class_name: Option<String>,
    pub package: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
    pub password: Option<bool>,
    pub showing_hint: Option<bool>,
    pub checkable: Option<bool>,
    pub scrollable: Option<bool>,
    pub long_clickable: Option<bool>,
    pub focusable: Option<bool>,
    pub enabled: Option<bool>,
    pub clickable: Option<bool>,
    pub visible: Option<bool>,
    pub focused: Option<bool>,
    pub selected: Option<bool>,
    pub checked: Option<bool>,
    pub bounds: Option<Rect>,
    pub raw_attributes: Option<BTreeMap<String, String>>,
}

impl SemanticNode {
    /// Password evidence is never returned as text/value/name or leaked in raw attributes.
    pub fn redact_sensitive(&mut self) {
        if self.password == Some(true) {
            self.text = None;
            self.value = None;
            self.name = None;
            self.raw_attributes = None;
        }
    }

    pub fn project(&mut self, fields: &ObservationFieldMask) {
        self.redact_sensitive();
        if !fields.identity {
            self.id = None;
            self.class_name = None;
            self.package = None;
        }
        if !fields.semantics {
            self.role = None;
            self.name = None;
            self.text = None;
            self.value = None;
        }
        if !fields.states {
            self.enabled = None;
            self.clickable = None;
            self.visible = None;
            self.focused = None;
            self.selected = None;
            self.checked = None;
            self.password = None;
            self.showing_hint = None;
            self.checkable = None;
            self.scrollable = None;
            self.long_clickable = None;
            self.focusable = None;
        }
        if !fields.bounds {
            self.bounds = None;
        }
        if !fields.raw_attributes {
            self.raw_attributes = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiObservation {
    pub device_id: String,
    pub app: ObservedAppContext,
    pub session_epoch: String,
    pub observation_id: String,
    pub generation: u64,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub source: ObservationSource,
    pub completeness: ObservationCompleteness,
    pub matches: Vec<SemanticNode>,
    /// Nodes whose missing selector attributes prevent proving either match or mismatch.
    pub unknown_match_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ObservationExpectation {
    Exists,
    Absent,
    Visible,
    Hidden,
    Enabled,
    Disabled,
    Selected {
        value: bool,
    },
    Checked {
        value: bool,
    },
    Focused {
        value: bool,
    },
    Value {
        value: String,
        #[serde(default = "exact_default")]
        exact: bool,
    },
    Text {
        value: String,
        #[serde(default = "exact_default")]
        exact: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExpectationVerdict {
    Satisfied,
    NotSatisfied,
    Unknown,
    Ambiguous,
}

/// Unique cardinality is mandatory. Missing fields or incomplete reads cannot prove negatives.
pub fn expect_observation(
    observation: &UiObservation,
    expected: &ObservationExpectation,
) -> ExpectationVerdict {
    use ExpectationVerdict::{Ambiguous, NotSatisfied, Satisfied, Unknown};
    if observation.matches.len() > 1 {
        return Ambiguous;
    }
    if observation.completeness != ObservationCompleteness::Complete
        || observation.unknown_match_count > 0
    {
        return Unknown;
    }
    let Some(node) = observation.matches.first() else {
        return if *expected == ObservationExpectation::Absent {
            Satisfied
        } else {
            NotSatisfied
        };
    };
    let known = match expected {
        ObservationExpectation::Exists => Some(true),
        ObservationExpectation::Absent => Some(false),
        ObservationExpectation::Visible => node.visible,
        ObservationExpectation::Hidden => node.visible.map(|v| !v),
        ObservationExpectation::Enabled => node.enabled,
        ObservationExpectation::Disabled => node.enabled.map(|v| !v),
        ObservationExpectation::Selected { value } => node.selected.map(|v| v == *value),
        ObservationExpectation::Focused { value } => node.focused.map(|v| v == *value),
        ObservationExpectation::Checked { value } => {
            if node.checkable == Some(true) {
                node.checked.map(|v| v == *value)
            } else {
                None
            }
        }
        ObservationExpectation::Value { value, exact } => {
            if node.password == Some(true) || node.showing_hint != Some(false) {
                None
            } else {
                node.value.as_ref().map(|actual| {
                    if *exact {
                        actual == value
                    } else {
                        actual.contains(value)
                    }
                })
            }
        }
        ObservationExpectation::Text { .. } if node.password == Some(true) => None,
        ObservationExpectation::Text { value, exact } => node.text.as_ref().map(|text| {
            if *exact {
                text == value
            } else {
                text.contains(value)
            }
        }),
    };
    match known {
        Some(true) => Satisfied,
        Some(false) => NotSatisfied,
        None => Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationWaitStatus {
    Satisfied,
    DeadlineExceeded,
    Cancelled,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationWaitResult {
    pub status: ObservationWaitStatus,
    pub verdict: ExpectationVerdict,
    pub observation: Option<UiObservation>,
}
