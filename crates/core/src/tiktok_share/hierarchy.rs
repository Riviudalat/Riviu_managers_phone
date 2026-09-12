//! TikTok publication projections over the shared accessibility tree.
use super::*;
pub(crate) use crate::ui_automation::tree::{Node, Tree};

impl Tree {
    pub fn control(
        &self,
        package: &str,
        label: Option<crate::tiktok_labels::LabelMatch>,
    ) -> Option<ElementBox> {
        let query = label?.to_query();
        let nodes = if let ElementQuery::Semantic(role) = query {
            crate::app_automation::tiktok_roles::indices(self, package, role)
        } else {
            self.matching(package, query)
        };
        let [index] = nodes.as_slice() else {
            return None;
        };
        let rect = self.nodes[*index].rect()?;
        (rect.enabled && rect.clickable).then_some(rect)
    }

    /// A text caption may sit outside the hitbox. Its own clickable ancestor is
    /// the target; a neighbouring icon or a whole-screen parent is never guessed.
    pub(crate) fn copy_control(&self, package: &str) -> Result<Option<ElementBox>, ()> {
        let mut controls = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.visible(package)
                || !self.ancestors_visible(index)
                || node.attr("enabled") != "true"
            {
                continue;
            }
            if ![node.attr("text"), node.attr("content-desc")]
                .iter()
                .any(|label| COPY_ROW_NEEDLES.contains(&label.trim().to_lowercase().as_str()))
            {
                continue;
            }
            let mut current = Some(index);
            for _ in 0..4 {
                let Some(index) = current else {
                    break;
                };
                let ancestor = &self.nodes[index];
                if !ancestor.visible(package) || ancestor.attr("enabled") != "true" {
                    break;
                }
                if ancestor.attr("clickable") == "true" {
                    // Any other labelled action below this ancestor makes it a
                    // sheet/container rather than the Copy action's hitbox.
                    let competing = self.nodes.iter().enumerate().any(|(other, n)| {
                        other != index
                            && self.inside(other, index)
                            && n.attr("clickable") == "true"
                            && ![n.attr("text"), n.attr("content-desc")].iter().all(|s| {
                                s.is_empty()
                                    || COPY_ROW_NEEDLES.contains(&s.trim().to_lowercase().as_str())
                            })
                    });
                    if !competing && ancestor.rect().is_some() && !controls.contains(&index) {
                        controls.push(index);
                    }
                    break;
                }
                current = ancestor.parent;
            }
        }
        match controls.as_slice() {
            [] => Ok(None),
            [index] => Ok(self.nodes[*index].rect()),
            _ => Err(()),
        }
    }

    pub fn grid(&self, plan: &PublishVerificationPlan) -> Vec<ElementBox> {
        let Some(tile) = plan.labels.post_tile_id() else {
            return Vec::new();
        };
        let package = plan.labels.package();
        let badges: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| {
                node.visible(package)
                    && (plan
                        .labels
                        .draft_badge_id()
                        .is_some_and(|label| node.matches(label.to_query()))
                        || plan
                            .labels
                            .pinned_badge_id()
                            .is_some_and(|label| node.matches(label.to_query()))
                        || [node.attr("text"), node.attr("content-desc")]
                            .iter()
                            .any(|text| {
                                let text = text.trim().to_lowercase();
                                text == "drafts"
                                    || text.starts_with("drafts:")
                                    || text == "bản nháp"
                                    || text.starts_with("bản nháp:")
                                    || text == "pinned"
                            }))
            })
            .filter_map(Node::rect)
            .collect();
        self.matching(package, tile.to_query())
            .into_iter()
            .filter_map(|index| self.nodes[index].rect())
            .filter(|tile| tile.enabled && !badges.iter().any(|badge| contains(tile, badge)))
            .collect()
    }

    pub fn drafts_observed(&self, plan: &PublishVerificationPlan) -> bool {
        self.nodes.iter().any(|node| {
            node.visible(plan.labels.package())
                && node.rect().is_some()
                && (plan
                    .labels
                    .draft_badge_id()
                    .is_some_and(|label| node.matches(label.to_query()))
                    || [node.attr("text"), node.attr("content-desc")]
                        .iter()
                        .any(|text| {
                            let text = text.trim().to_lowercase();
                            text == "drafts"
                                || text.starts_with("drafts:")
                                || text == "bản nháp"
                                || text.starts_with("bản nháp:")
                        }))
        })
    }

    /// Pagination is allowed only inside a live scrollable ancestor containing
    /// measured profile covers. The union of guessed coordinates is not a grid.
    pub fn grid_scroll(&self, plan: &PublishVerificationPlan) -> Option<crate::SwipeGesture> {
        let tiles = self.matching(
            plan.labels.package(),
            plan.labels.post_tile_id()?.to_query(),
        );
        let first = *tiles.first()?;
        let mut current = self.nodes[first].parent;
        while let Some(index) = current {
            let node = &self.nodes[index];
            if node.visible(plan.labels.package())
                && node.attr("scrollable") == "true"
                && tiles.iter().all(|tile| self.inside(*tile, index))
            {
                let rect = node.rect()?;
                // Both endpoints are centres of visible measured covers inside
                // the declared scroll container; no absolute screen geometry.
                let boxes: Vec<_> = tiles
                    .iter()
                    .filter_map(|i| self.nodes[*i].rect())
                    .filter(|b| contains(&rect, b))
                    .collect();
                let top = boxes.iter().min_by(|a, b| a.y.total_cmp(&b.y))?;
                let bottom = boxes.iter().max_by(|a, b| a.y.total_cmp(&b.y))?;
                if bottom.y <= top.y {
                    return None;
                }
                let x = rect.centre().x;
                return Some(crate::SwipeGesture {
                    from: crate::TapPoint {
                        x,
                        y: bottom.centre().y,
                    },
                    to: crate::TapPoint {
                        x,
                        y: top.centre().y,
                    },
                    duration_ms: 600,
                });
            }
            current = node.parent;
        }
        None
    }
}
