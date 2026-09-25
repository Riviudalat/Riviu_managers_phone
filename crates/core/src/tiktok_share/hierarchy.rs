//! TikTok publication projections over the shared accessibility tree.
use super::*;
pub(crate) use crate::ui_automation::tree::{Node, Tree};

impl Tree {
    pub(crate) fn profile_suggestions_hide(
        &self,
        plan: &PublishVerificationPlan,
    ) -> Option<ElementBox> {
        let package = plan.labels.package();
        if package != "com.ss.android.ugc.trill"
            || plan.labels.resource_version() != Some("38.3.2")
            || plan.labels.language() != "en"
        {
            return None;
        }
        let groups = self.matching(package, ElementQuery::ResourceIdSuffix(":id/gg5"));
        let [group] = groups.as_slice() else {
            return None;
        };
        let headings = self.matching(package, ElementQuery::ResourceIdSuffix(":id/qwr"));
        let [heading] = headings.as_slice() else {
            return None;
        };
        if self.nodes[*heading].attr("text") != "Suggested accounts"
            || !self.inside(*heading, *group)
        {
            return None;
        }
        let controls = self.matching(package, ElementQuery::ResourceIdSuffix(":id/oc5"));
        let [control] = controls.as_slice() else {
            return None;
        };
        let node = &self.nodes[*control];
        if !self.inside(*control, *group)
            || node.attr("text") != "Hide"
            || node.attr("class") != "android.widget.Button"
        {
            return None;
        }
        node.rect().filter(|r| r.enabled && r.clickable)
    }
    pub(crate) fn publication_removed(&self, package: &str) -> bool {
        // Trill 38.3.2, machine 7, 14/09/2026: Share on a removed post opens
        // a Delete sheet. This banner is evidence to skip that candidate entirely.
        self.nodes.iter().enumerate().any(|(index, node)| {
            node.visible(package)
                && self.ancestors_visible(index)
                && matches!(
                    node.attr("text"),
                    "Removed for violating Community Guidelines"
                        | "Community Guidelines violation: View details"
                )
        })
    }

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
        // S8 Trill 38.3.2, machine 1, 14/09/2026: visible-to-user covers
        // extend below the bottom tabs. Clip against their observed upper edge,
        // otherwise a cover centre hits Create and opens the camera.
        let navigation_top = [TikTokControl::ProfileTab, TikTokControl::FeedTab]
            .into_iter()
            .filter_map(|control| self.control(package, plan.labels.label(control)))
            .map(|rect| rect.y)
            .min_by(f64::total_cmp);
        self.matching(package, tile.to_query())
            .into_iter()
            .filter_map(|index| self.nodes[index].rect())
            .filter(|tile| tile.enabled && !badges.iter().any(|badge| contains(tile, badge)))
            .filter_map(|mut tile| {
                if let Some(top) = navigation_top {
                    tile.height = tile.height.min(top - tile.y);
                }
                (tile.height > 0.0).then_some(tile)
            })
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
                if plan.labels.package() != "com.zhiliaoapp.musically"
                    || plan.labels.resource_version() != Some("45.7.3")
                {
                    let boxes: Vec<_> = self
                        .grid(plan)
                        .into_iter()
                        .filter(|box_| contains(&rect, box_))
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
                let navigation_top = [TikTokControl::ProfileTab, TikTokControl::FeedTab]
                    .into_iter()
                    .filter_map(|control| {
                        self.control(plan.labels.package(), plan.labels.label(control))
                    })
                    .map(|button| button.y)
                    .min_by(f64::total_cmp)
                    .unwrap_or(rect.y + rect.height);
                let clip = |mut cover: ElementBox| {
                    let left = cover.x.max(rect.x);
                    let right = (cover.x + cover.width).min(rect.x + rect.width);
                    let top = cover.y.max(rect.y);
                    let bottom = (cover.y + cover.height)
                        .min(rect.y + rect.height)
                        .min(navigation_top);
                    if right <= left || bottom <= top {
                        return None;
                    }
                    cover.x = left;
                    cover.y = top;
                    cover.width = right - left;
                    cover.height = bottom - top;
                    Some(cover)
                };
                let mut boxes: Vec<_> = self.grid(plan).into_iter().filter_map(clip).collect();
                // A pinned cover is excluded from the candidate search, but its
                // measured rectangle can anchor a longer swipe inside this grid.
                if let Some(label) = plan.labels.pinned_badge_id() {
                    let badges: Vec<_> = self
                        .matching(plan.labels.package(), label.to_query())
                        .into_iter()
                        .filter_map(|badge| self.nodes[badge].rect())
                        .collect();
                    for tile in &tiles {
                        let Some(cover) = self.nodes[*tile].rect() else {
                            continue;
                        };
                        if cover.enabled && badges.iter().any(|badge| contains(&cover, badge)) {
                            if let Some(cover) = clip(cover) {
                                boxes.push(cover);
                            }
                        }
                    }
                }
                let (top, bottom, left, right) = boxes
                    .iter()
                    .flat_map(|top| {
                        boxes.iter().filter_map(move |bottom| {
                            let left = top.x.max(bottom.x);
                            let right = (top.x + top.width).min(bottom.x + bottom.width);
                            (bottom.y > top.y && right > left).then_some((top, bottom, left, right))
                        })
                    })
                    .max_by(|(top_a, bottom_a, _, _), (top_b, bottom_b, _, _)| {
                        (bottom_a.y - top_a.y).total_cmp(&(bottom_b.y - top_b.y))
                    })?;
                let x = (left + right) / 2.0;
                return Some(crate::SwipeGesture {
                    from: crate::TapPoint {
                        x,
                        y: bottom.y + bottom.height * 0.9,
                    },
                    to: crate::TapPoint {
                        x,
                        y: top.y + top.height * 0.1,
                    },
                    duration_ms: 600,
                });
            }
            current = node.parent;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_pinned_row_extends_profile_scroll_without_crossing_navigation() {
        let plan =
            PublishVerificationPlan::for_build("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let xml = r#"<hierarchy>
            <node package="com.zhiliaoapp.musically" enabled="true" scrollable="true" bounds="[0,923][1080,1965]">
                <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/cover" enabled="true" bounds="[361,923][719,1400]">
                    <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/tv_top" text="Pinned" enabled="true" bounds="[377,940][483,977]"/>
                </node>
                <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/cover" enabled="true" bounds="[361,1403][719,1880]"/>
                <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/cover" enabled="true" bounds="[361,1883][719,2360]"/>
            </node>
            <node package="com.zhiliaoapp.musically" content-desc="Profile" enabled="true" clickable="true" bounds="[864,1965][1080,2094]"/>
        </hierarchy>"#;
        let observed = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
        .unwrap();
        assert_eq!(observed.grid(&plan).len(), 2);
        let gesture = observed.grid_scroll(&plan).expect("measured grid swipe");
        assert!((361.0..719.0).contains(&gesture.from.x));
        assert_eq!(gesture.from.x, gesture.to.x);
        assert!((1883.0..1965.0).contains(&gesture.from.y));
        assert!((923.0..1400.0).contains(&gesture.to.y));
        assert!(gesture.from.y - gesture.to.y > 950.0);
    }

    #[test]
    fn trill_profile_scroll_keeps_original_visible_cover_centres() {
        let plan =
            PublishVerificationPlan::for_build("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let xml = r#"<hierarchy>
            <node package="com.ss.android.ugc.trill" enabled="true" scrollable="true" bounds="[0,500][1080,1800]">
                <node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/cover" enabled="true" bounds="[0,550][350,1000]"/>
                <node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/cover" enabled="true" bounds="[0,1050][350,1500]"/>
            </node>
        </hierarchy>"#;
        let observed = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
        .unwrap();
        let gesture = observed
            .grid_scroll(&plan)
            .expect("measured Trill grid swipe");
        assert_eq!(gesture.from.x, 540.0);
        assert_eq!(gesture.from.y, 1275.0);
        assert_eq!(gesture.to.y, 775.0);
    }
}
