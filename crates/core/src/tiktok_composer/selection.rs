//! A thumbnail opens preview; the measured corner button selects the photo.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PickerControls {
    pub package: &'static str,
    pub selector: ElementQuery<'static>,
    pub next: ElementQuery<'static>,
}

impl PickerControls {
    pub fn for_labels(labels: &TikTokControls) -> Option<Self> {
        if labels.adaptive() {
            return Some(Self {
                package: labels.package(),
                selector: ElementQuery::Semantic("selector"),
                next: ElementQuery::Semantic("pickerNext"),
            });
        }
        // 07/09/2026, 98895a3355424e484f: h4b text changes empty -> 1..6;
        // Next becomes Next (6). A thumbnail-centre tap opens preview instead.
        match (labels.package(), labels.resource_version()) {
            // AGENTS.md §9.189: measured independently on ce04171435f104080c.
            ("com.zhiliaoapp.musically", Some("45.4.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/k3j"),
                next: ElementQuery::ResourceIdSuffix(":id/w86"),
            }),
            // AGENTS.md §9.189: measured independently on ce051715e15b2c2e02.
            ("com.zhiliaoapp.musically", Some("46.1.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kfk"),
                next: ElementQuery::ResourceIdSuffix(":id/wrj"),
            }),
            // AGENTS.md §9.189: measured independently on ce031713aadf361905.
            ("com.zhiliaoapp.musically", Some("46.4.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/knq"),
                next: ElementQuery::ResourceIdSuffix(":id/x4j"),
            }),

            // 08/09/2026, ce031713b0c610ab0c, code 2024507030; each corner
            // carries its ordinal and Next confirms exactly three selected photos.
            ("com.zhiliaoapp.musically", Some("45.7.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/k_x"),
                next: ElementQuery::ResourceIdSuffix(":id/wjp"),
            }),
            // 08/09/2026, ce011711c354be2005, versionCode 2024600410:
            // kek carries ordinals 1,2,3 and wpw confirms Next (3).
            ("com.zhiliaoapp.musically", Some("46.0.41")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kek"),
                next: ElementQuery::ResourceIdSuffix(":id/wpw"),
            }),
            ("com.ss.android.ugc.trill", Some("38.3.2")) => Some(Self {
                package: "com.ss.android.ugc.trill",
                selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
                next: ElementQuery::ResourceIdSuffix(":id/q4g"),
            }),
            // 07/09/2026, ce0517155ab38c390d: six kh7 corner controls in the
            // isolated album, measured separately from the 38.3.2 picker.
            ("com.zhiliaoapp.musically", Some("46.2.42")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kh7"),
                next: ElementQuery::ResourceIdSuffix(":id/wud"),
            }),
            // 07/09/2026, ce0717171c2a64d50d: 46.2.1 uses kir at the same
            // measured corner positions, not the 46.2.42 resource ID.
            ("com.zhiliaoapp.musically", Some("46.2.1")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kir"),
                next: ElementQuery::ResourceIdSuffix(":id/wwo"),
            }),
            _ => None,
        }
    }
}

fn label_count(text: &str) -> Option<usize> {
    let text = text.trim();
    let number = text
        .strip_prefix("Next")
        .or_else(|| text.strip_prefix("Tiếp"))?
        .trim();
    let number = number
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(number);
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

pub(super) fn explicit_count(rows: &[ElementBox]) -> Option<usize> {
    let [row] = rows else {
        return None;
    };
    label_count(row.description.as_deref()?)
}

fn ordered_controls(
    mut rows: Vec<ElementBox>,
    screen: Screen,
    bottom: f64,
) -> Option<Vec<ElementBox>> {
    if rows.is_empty() {
        return None;
    }
    if rows.iter().any(|row| {
        !on_screen(row, screen) || row.y + row.height > bottom || !row.enabled || !row.clickable
    }) {
        return None;
    }
    rows.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    if rows
        .windows(2)
        .any(|p| p[0].x == p[1].x && p[0].y == p[1].y)
    {
        return None;
    }
    Some(rows)
}

fn selected_prefix(rows: &[ElementBox], count: usize) -> bool {
    rows.iter().enumerate().all(|(index, row)| {
        let text = row.description.as_deref().unwrap_or("").trim();
        if index < count {
            text.parse::<usize>() == Ok(index + 1)
        } else {
            text.is_empty()
        }
    })
}

fn picker_snapshot(
    xml: &str,
    controls: PickerControls,
) -> anyhow::Result<(Vec<ElementBox>, Vec<ElementBox>)> {
    let tree = crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: xml.into(),
    })?;
    let mut selectors = Vec::new();
    let mut next = Vec::new();
    for (index, node) in tree.nodes.iter().enumerate() {
        if !node.visible(controls.package) || !tree.ancestors_visible(index) {
            continue;
        }
        let semantic = |query| match query {
            ElementQuery::Semantic(role) => {
                crate::app_automation::tiktok_roles::indices(&tree, controls.package, role)
                    .contains(&index)
            }
            _ => node.matches(query),
        };
        let selector = semantic(controls.selector);
        let is_next = semantic(controls.next);
        if !selector && !is_next {
            continue;
        }
        if !matches!(controls.selector, ElementQuery::Semantic(_)) {
            anyhow::ensure!(
                node.attr("resource-id")
                    .starts_with(&format!("{}:", controls.package)),
                "picker package mismatch"
            );
        }
        let row = node.rect().context("picker bounds missing")?;
        if selector {
            anyhow::ensure!(
                node.attr("class") == "android.widget.Button",
                "picker selector class"
            );
            selectors.push(row);
        } else {
            next.push(row);
        }
    }
    if selectors.is_empty() && next.is_empty() {
        selectors =
            crate::app_automation::tiktok_roles::locate(&tree, controls.package, "selector");
        next = crate::app_automation::tiktok_roles::locate(&tree, controls.package, "pickerNext");
    }
    Ok((selectors, next))
}

fn snapshot_has_album(
    xml: &str,
    controls: PickerControls,
    query: ElementQuery<'_>,
    album: &str,
) -> anyhow::Result<bool> {
    let tree = crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: xml.into(),
    })?;
    // Album text is read-only metadata; its label may omit geometry while its
    // separately resolved menu ancestor owns the tap rectangle.
    let matched: Vec<_> = if let ElementQuery::Semantic(role) = query {
        crate::app_automation::tiktok_roles::indices(&tree, controls.package, role)
    } else {
        tree.nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.visible(controls.package) && tree.ancestors_visible(*i) && n.matches(query)
            })
            .map(|(i, _)| i)
            .collect()
    };
    Ok(matched.len() == 1 && tree.nodes[matched[0]].attr("text").trim() == album)
}

/// How far an extrapolated row may sit from `previous row + pitch` and still be the grid.
///
/// Measured grids are exact (45.7.3: every row 362 px apart, 09/09/2026), so this only
/// absorbs a rounding pixel in a layout no fixture has shown yet. Cells that have been seen
/// are always matched exactly.
const ROW_PITCH_TOLERANCE: f64 = 2.0;

/// The isolated album's grid, learned from the cells that have been on screen.
///
/// **The album used to have to fit on one screen.** `select_verified` read the picker once,
/// with nothing selected, and took those cells as the reference for every later snapshot —
/// which meant a bundle could hold exactly as many photos as the first screen showed, and the
/// ceiling was 13 on the one phone that was measured. A carousel is TikTok's 35, and the
/// picker is a regular grid: fixed columns, one row pitch. So the reference is now built from
/// what is visible and **extended one row at a time as scrolling reveals it**, each new row
/// admitted only where the grid says it must be — same column geometry, previous row plus the
/// pitch — and, once admitted, matched exactly like every cell before it.
///
/// `cells[k]` is cell `k` in the frame of the first snapshot; later snapshots are that frame
/// translated by one uniform `shift`, which is how a scroll shows up.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct KnownGrid {
    cells: Vec<ElementBox>,
    /// `(x, width, height)` per column, left to right.
    columns: Vec<(f64, f64, f64)>,
    /// Distance between consecutive rows, once two rows have been seen.
    row_pitch: Option<f64>,
    /// How many cells the album must hold — the bundle's image count.
    wanted: usize,
}

impl KnownGrid {
    /// Build the reference from the unselected first screen, or refuse a layout that is not a
    /// regular grid.
    ///
    /// When the album is larger than the screen the visible part must end on a full row: cells
    /// in one row share `y` and `height`, so `ordered_controls` keeps or drops a row whole, and
    /// a ragged last row with more photos still to come is a layout this never measured.
    pub(super) fn from_initial(initial: Vec<ElementBox>, wanted: usize) -> Option<Self> {
        if initial.is_empty() || initial.len() > wanted {
            return None;
        }
        let first_y = initial[0].y;
        let columns: Vec<(f64, f64, f64)> = initial
            .iter()
            .take_while(|cell| cell.y == first_y)
            .map(|cell| (cell.x, cell.width, cell.height))
            .collect();
        if columns
            .windows(2)
            .any(|pair| pair[1].0 <= pair[0].0 + pair[0].1)
        {
            return None;
        }
        let mut row_pitch = None;
        for (index, cell) in initial.iter().enumerate() {
            let (x, width, height) = columns[index % columns.len()];
            if cell.x != x || cell.width != width || cell.height != height {
                return None;
            }
            if index >= columns.len() {
                let above = &initial[index - columns.len()];
                let pitch = cell.y - above.y;
                match row_pitch {
                    None if pitch > 1.0 => row_pitch = Some(pitch),
                    Some(known) if (pitch - known).abs() <= ROW_PITCH_TOLERANCE => {}
                    _ => return None,
                }
            } else if cell.y != first_y {
                return None;
            }
        }
        if initial.len() < wanted
            && (!initial.len().is_multiple_of(columns.len()) || row_pitch.is_none())
        {
            return None;
        }
        Some(Self {
            cells: initial,
            columns,
            row_pitch,
            wanted,
        })
    }

    pub(super) fn columns(&self) -> usize {
        self.columns.len()
    }

    pub(super) fn known(&self) -> usize {
        self.cells.len()
    }

    /// The row pitch, for a scroll of exactly one row.
    pub(super) fn row_pitch(&self) -> Option<f64> {
        self.row_pitch
    }

    /// Where cell `absolute` must be, in the first snapshot's frame, if it has not been seen:
    /// the cell one row above it, one pitch further down. `learned` holds the cells this
    /// snapshot has already admitted, so a snapshot may reveal more than one new row.
    fn expected_unseen(
        &self,
        absolute: usize,
        learned: &[ElementBox],
    ) -> Option<(f64, f64, f64, f64)> {
        if absolute != self.cells.len() + learned.len() || absolute >= self.wanted {
            return None;
        }
        let (x, width, height) = self.columns[absolute % self.columns.len()];
        let above_index = absolute.checked_sub(self.columns.len())?;
        let above = self
            .cells
            .get(above_index)
            .or_else(|| learned.get(above_index.checked_sub(self.cells.len())?))?;
        Some((x, above.y + self.row_pitch?, width, height))
    }
}

/// Match the visible contiguous slice back to the isolated album's grid.
/// TikTok 45.7.3, 09/09/2026: selecting image 10 in an 11-photo album hides
/// ordinals 1..3 and shifts the remaining controls uniformly by -181 px. A scroll
/// may translate y, but it must not reorder, resize, skip an interior cell or
/// replace the ordinal immediately before the next blank selection.
///
/// Cells past what the grid has seen are admitted only as the next row of the same grid, and
/// only on success are they written into `grid` — a snapshot that fails any check leaves the
/// reference exactly as it was. A visible cell past `wanted` is an album holding more photos
/// than the bundle, and is refused for the same reason the old shape refused an oversize
/// first screen.
fn visible_selection(
    rows: Vec<ElementBox>,
    next: &[ElementBox],
    screen: Screen,
    grid: &mut KnownGrid,
    count: usize,
) -> Option<(Vec<ElementBox>, ElementBox)> {
    let [next] = next else {
        return None;
    };
    if !on_screen(next, screen)
        || !next.enabled
        || (count > 0
            && (!next.clickable || explicit_count(std::slice::from_ref(next)) != Some(count)))
    {
        return None;
    }
    let rows = ordered_controls(rows, screen, next.y)?;
    let offset = if count == 0 {
        0
    } else {
        rows.first()?
            .description
            .as_deref()?
            .trim()
            .parse::<usize>()
            .ok()?
            .checked_sub(1)?
    };
    if offset > count
        || offset + rows.len() > grid.wanted
        || (count == 0 && rows.len() != grid.known())
        || count > offset + rows.len()
    {
        return None;
    }
    let shift = rows.first()?.y - grid.cells.get(offset)?.y;
    let mut learned = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let absolute = offset + index;
        let text = row.description.as_deref().unwrap_or("").trim();
        if (absolute < count && text.parse::<usize>() != Ok(absolute + 1))
            || (absolute >= count && !text.is_empty())
        {
            return None;
        }
        if let Some(reference) = grid.cells.get(absolute) {
            if row.x != reference.x
                || row.width != reference.width
                || row.height != reference.height
                || row.y - reference.y != shift
            {
                return None;
            }
        } else {
            let (x, y, width, height) = grid.expected_unseen(absolute, &learned)?;
            if row.x != x
                || row.width != width
                || row.height != height
                || (row.y - shift - y).abs() > ROW_PITCH_TOLERANCE
            {
                return None;
            }
            learned.push(ElementBox {
                y: row.y - shift,
                description: None,
                ..row.clone()
            });
        }
    }
    grid.cells.extend(learned);
    Some((rows, next.clone()))
}

impl<P: TapPlanner> Composer<'_, P> {
    /// One selection algorithm for 1..=35 photos — TikTok's own carousel ceiling — including
    /// TikTok's automatic scrolling and our own, one measured row at a time, when the album
    /// is taller than the screen. Every successful tap yields the next validated snapshot
    /// directly; see [`KnownGrid`] for how cells past the first screen are admitted.
    pub(super) async fn select_verified(
        &mut self,
        controls: PickerControls,
        screen: Screen,
        wanted: usize,
        album: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<Selection> {
        if !(1..=crate::publish::MAX_CAROUSEL_IMAGES).contains(&wanted) {
            return Ok(Selection::NotEnoughSelected);
        }
        if stop.load(Ordering::Relaxed) {
            return Ok(Selection::Stopped);
        }
        let session = self.session;
        let album_query = self.plan.album_menu;
        let read = || async {
            let snapshot = session.hierarchy_source_snapshot().await?;
            let parsed = picker_snapshot(&snapshot.xml, controls)?;
            Ok::<_, anyhow::Error>(
                snapshot_has_album(&snapshot.xml, controls, album_query, album)?.then_some(parsed),
            )
        };
        let Some((initial, next)) = read().await? else {
            return Ok(Selection::NotEnoughSelected);
        };
        let [next_button] = next.as_slice() else {
            return Ok(Selection::NotEnoughSelected);
        };
        let Some(initial) = ordered_controls(initial, screen, next_button.y) else {
            return Ok(Selection::NotEnoughSelected);
        };
        if !selected_prefix(&initial, 0) || explicit_count(&next).is_some_and(|n| n != 0) {
            return Ok(Selection::NotEnoughSelected);
        }
        let Some(mut grid) = KnownGrid::from_initial(initial.clone(), wanted) else {
            return Ok(Selection::NotEnoughSelected);
        };
        let Some(mut current) = visible_selection(initial, &next, screen, &mut grid, 0) else {
            return Ok(Selection::NotEnoughSelected);
        };
        // One scroll reveals one row, so an album of `wanted` cells needs at most one fewer
        // scroll than it has rows; one more absorbs TikTok's own scroll on the first tap.
        let scroll_budget = wanted.div_ceil(grid.columns());
        let mut count = 0;
        let mut scrolls = 0;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(Selection::Stopped);
            }
            let (rows, next) = current;
            if count == wanted {
                return Ok(Selection::Armed {
                    next,
                    counted: Some(wanted),
                });
            }
            if let Some(row) = rows
                .iter()
                .find(|row| row.description.as_deref().unwrap_or("").trim().is_empty())
            {
                self.tap_inside(row).await?;
                let expected = count + 1;
                let deadline = Instant::now() + ARM_WINDOW;
                current = loop {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(Selection::Stopped);
                    }
                    let Some((rows, next)) = read().await? else {
                        return Ok(Selection::NotEnoughSelected);
                    };
                    if let Some(observed) =
                        visible_selection(rows, &next, screen, &mut grid, expected)
                    {
                        break observed;
                    }
                    if Instant::now() >= deadline {
                        return Ok(Selection::NotEnoughSelected);
                    }
                    sleep(POLL, stop).await;
                };
                count = expected;
            } else {
                // The selected tail is visible but the next cell is below the viewport.
                // Scroll only one measured row so overlapping ordinals retain identity.
                if scrolls >= scroll_budget
                    || rows
                        .last()
                        .and_then(|row| row.description.as_deref())
                        .and_then(|text| text.trim().parse::<usize>().ok())
                        != Some(count)
                {
                    return Ok(Selection::NotEnoughSelected);
                }
                let row_height = rows
                    .windows(2)
                    .map(|rows| rows[1].y - rows[0].y)
                    .find(|distance| *distance > 1.0)
                    .or_else(|| grid.row_pitch())
                    .context("picker rows missing")?;
                let from = rows.last().unwrap().y;
                self.session
                    .swipe(crate::SwipeGesture {
                        from: crate::TapPoint {
                            x: screen.width() * 0.5,
                            y: from,
                        },
                        to: crate::TapPoint {
                            x: screen.width() * 0.5,
                            y: from - row_height,
                        },
                        duration_ms: 450,
                    })
                    .await?;
                scrolls += 1;
                sleep(Duration::from_millis(450), stop).await;
                let Some((rows, next)) = read().await? else {
                    return Ok(Selection::NotEnoughSelected);
                };
                let Some(observed) = visible_selection(rows, &next, screen, &mut grid, count)
                else {
                    return Ok(Selection::NotEnoughSelected);
                };
                current = observed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn measured_global_videos_have_one_corner_ordinal_and_one_next_count() {
        for (version, xml) in [
            (
                "45.4.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-45.4.3-en/video-selected.xml"
                ),
            ),
            (
                "45.7.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-45.7.3-en/video-selected.xml"
                ),
            ),
            (
                "46.0.41",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.0.41-en/video-selected.xml"
                ),
            ),
            (
                "46.1.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.1.3-en/video-selected.xml"
                ),
            ),
            (
                "46.2.1",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.2.1-en/video-selected.xml"
                ),
            ),
            (
                "46.4.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.4.3-en/video-selected.xml"
                ),
            ),
        ] {
            let labels =
                crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", version)
                    .unwrap();
            let controls = PickerControls::for_labels(&labels).unwrap();
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert_eq!(rows.len(), 1, "{version}");
            assert!(selected_prefix(&rows, 1));
            assert_eq!(explicit_count(&next), Some(1));
        }
    }

    use super::*;
    #[test]
    fn new_fleet_photo_ordinals_and_next_count_match_each_measured_version() {
        for (version, initial, one, two) in [
            (
                "45.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/two.xml"),
            ),
            (
                "46.1.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/two.xml"),
            ),
            (
                "46.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/two.xml"),
            ),
        ] {
            let labels =
                crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en-US", version)
                    .unwrap();
            let controls = PickerControls::for_labels(&labels).unwrap();
            for (count, xml) in [initial, one, two].into_iter().enumerate() {
                let (rows, next) = picker_snapshot(xml, controls).unwrap();
                assert_eq!(rows.len(), 2);
                assert!(selected_prefix(&rows, count));
                assert_eq!(explicit_count(&next), (count > 0).then_some(count));
            }
            assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        }
    }
    use crate::TapPoint;
    use parking_lot::Mutex;
    const MOCK_ROW_PITCH: f64 = 362.0;
    /// A picker whose album may be taller than its screen.
    ///
    /// `visible_rows` is how many rows fit with nothing selected; `tray_hides_row` takes one
    /// away once something is (the selected-photos tray measured on 46.0.41, §9.197); each
    /// swipe scrolls exactly one row. `None` shows the whole album, as the small albums did.
    struct Picker {
        selected: Mutex<usize>,
        taps: Mutex<Vec<TapPoint>>,
        drop_at: Option<usize>,
        wrong_album: bool,
        total: usize,
        autoscroll: bool,
        reads: std::sync::atomic::AtomicUsize,
        corrupt_after_ten: Option<&'static str>,
        visible_rows: Option<usize>,
        tray_hides_row: bool,
        /// A row the album grows after this many swipes, off the grid: `"column"` shifts the
        /// new row's x, `"pitch"` its y, `"extra"` appends photos the bundle does not have.
        corrupt_scrolled_row: Option<&'static str>,
        swipes: Mutex<usize>,
    }
    impl Picker {
        fn new(drop_at: Option<usize>) -> Self {
            Self {
                selected: Mutex::new(0),
                taps: Mutex::new(Vec::new()),
                drop_at,
                wrong_album: false,
                total: 6,
                autoscroll: false,
                reads: std::sync::atomic::AtomicUsize::new(0),
                corrupt_after_ten: None,
                visible_rows: None,
                tray_hides_row: false,
                corrupt_scrolled_row: None,
                swipes: Mutex::new(0),
            }
        }
        /// A thirteen-photo album on the measured phone: five rows fit, four once the tray
        /// shows, so the thirteenth is reached by one scroll.
        fn with_tray(total: usize) -> Self {
            let mut session = Self::new(None);
            session.total = total;
            session.visible_rows = Some(5);
            session.tray_hides_row = true;
            session
        }
        /// First visible cell index and how many cells the viewport shows right now.
        fn viewport(&self) -> (usize, usize) {
            let selected = *self.selected.lock();
            let scrolled_rows = *self.swipes.lock();
            let Some(mut rows) = self.visible_rows else {
                return (0, self.total);
            };
            if self.tray_hides_row && selected > 0 {
                rows -= 1;
            }
            let start = scrolled_rows * 3;
            let end = ((scrolled_rows + rows) * 3).min(self.album_size());
            (start, end.saturating_sub(start))
        }
        fn album_size(&self) -> usize {
            if self.corrupt_scrolled_row == Some("extra") && *self.swipes.lock() > 0 {
                self.total + 3
            } else {
                self.total
            }
        }
        fn rows(&self) -> Vec<ElementBox> {
            let selected = *self.selected.lock();
            if self.autoscroll {
                let labels =
                    crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3")
                        .unwrap();
                let controls = PickerControls::for_labels(&labels).unwrap();
                let xml = if selected >= 10 {
                    include_str!("../../fixtures/tiktok-publish/picker-11-autoscroll-musically-45.7.3-en.xml")
                } else {
                    include_str!(
                        "../../fixtures/tiktok-publish/picker-11-initial-musically-45.7.3-en.xml"
                    )
                };
                let mut rows = picker_snapshot(xml, controls).unwrap().0;
                let offset = if selected >= 10 { 3 } else { 0 };
                for (index, row) in rows.iter_mut().enumerate() {
                    row.description = Some(if index + offset < selected {
                        (index + offset + 1).to_string()
                    } else {
                        String::new()
                    });
                }
                if selected >= 10 {
                    match self.corrupt_after_ten {
                        Some("gap") => {
                            rows.remove(2);
                        }
                        Some("misordered") => {
                            rows[1].description = Some("8".into());
                        }
                        Some("missing_tail") => {
                            rows.remove(rows.len() - 2);
                        }
                        _ => {}
                    }
                }
                return rows;
            }
            let (offset, shown) = self.viewport();
            let first_screen = self.visible_rows.map_or(self.total, |rows| rows * 3);
            (offset..offset + shown)
                .map(|index| {
                    // Corruptions apply to cells the first screen never showed — the ones the
                    // grid has to admit by extrapolation.
                    let revealed = index >= first_screen;
                    let skew_x = if revealed && self.corrupt_scrolled_row == Some("column") {
                        7.0
                    } else {
                        0.0
                    };
                    let skew_y = if revealed && self.corrupt_scrolled_row == Some("pitch") {
                        9.0
                    } else {
                        0.0
                    };
                    ElementBox {
                        x: 268.0 + (index % 3) as f64 * 358.0 + skew_x,
                        y: 375.0 + ((index - offset) / 3) as f64 * MOCK_ROW_PITCH + skew_y,
                        width: 72.0,
                        height: 72.0,
                        description: Some(if index < selected {
                            (index + 1).to_string()
                        } else {
                            String::new()
                        }),
                        clickable: true,
                        enabled: true,
                    }
                })
                .collect()
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Picker {
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::driver::HierarchySourceSnapshot> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let rows = self.rows();
            let count = *self.selected.lock();
            let album =
                if self.wrong_album || (self.corrupt_after_ten == Some("album") && count >= 10) {
                    "Other"
                } else {
                    "album"
                };
            let cells=rows.iter().map(|r|format!(r#"<node package="fixture" class="android.widget.Button" resource-id="fixture:id/h4b" text="{}" bounds="[{},{}][{},{}]" displayed="true" enabled="true" clickable="true"/>"#,r.description.as_deref().unwrap_or(""),r.x,r.y,r.x+r.width,r.y+r.height)).collect::<String>();
            Ok(crate::driver::HierarchySourceSnapshot {
                generation: 1,
                xml: format!(
                    r#"<hierarchy><node package="fixture" class="android.widget.TextView" resource-id="fixture:fixture-album-menu" text="{album}" displayed="true"/>{cells}<node package="fixture" class="android.widget.Button" resource-id="fixture:id/q4g" text="Next ({count})" bounds="[552,1936][1044,2028]" displayed="true" enabled="true" clickable="{}"/></hierarchy>"#,
                    count > 0
                ),
            })
        }
        async fn tap(&self, p: TapPoint) -> anyhow::Result<()> {
            let index = self
                .rows()
                .iter()
                .position(|r| {
                    p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height
                })
                .expect("only corner buttons may be tapped");
            let index = if self.autoscroll && *self.selected.lock() >= 10 {
                index + 3
            } else {
                index + self.viewport().0
            };
            self.taps.lock().push(p);
            if self.drop_at != Some(index) {
                *self.selected.lock() += 1;
            }
            Ok(())
        }
        async fn swipe(&self, gesture: crate::SwipeGesture) -> anyhow::Result<()> {
            assert!(
                self.visible_rows.is_some(),
                "scroll only when the selected tail hides the next image"
            );
            // Exactly one row, anchored on the last visible row — an overshoot would put a
            // second unseen row on screen with no ordinal to anchor it.
            let last = self.rows().last().unwrap().y;
            assert_eq!(
                gesture.from.y, last,
                "scroll starts on the last visible row"
            );
            assert_eq!(
                gesture.from.y - gesture.to.y,
                MOCK_ROW_PITCH,
                "scroll moves one measured row"
            );
            *self.swipes.lock() += 1;
            Ok(())
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("selection must not type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("no arbitrary controls")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate(&self, q: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.locate_all_described(q).await?.into_iter().next())
        }
        async fn locate_all_described(
            &self,
            q: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let key = match q {
                ElementQuery::Description { value, .. }
                | ElementQuery::Text { value, .. }
                | ElementQuery::ResourceIdSuffix(value)
                | ElementQuery::Semantic(value)
                | ElementQuery::ClassName(value) => value,
            };
            if key == ":id/h4b" {
                return Ok(self.rows());
            }
            let count = *self.selected.lock();
            Ok(match key {
                "fixture-picker-next" => vec![ElementBox {
                    x: 552.0,
                    y: 1936.0,
                    width: 492.0,
                    height: 92.0,
                    description: Some(if count == 0 {
                        "Next".into()
                    } else {
                        format!("Next ({count})")
                    }),
                    clickable: count > 0,
                    enabled: true,
                }],
                "fixture-album-menu" => vec![ElementBox {
                    x: 150.0,
                    y: 90.0,
                    width: 400.0,
                    height: 80.0,
                    description: Some(
                        if self.wrong_album
                            || (self.corrupt_after_ten == Some("album") && count >= 10)
                        {
                            "Other"
                        } else {
                            "album"
                        }
                        .into(),
                    ),
                    enabled: true,
                    clickable: true,
                }],
                _ => vec![],
            })
        }
    }
    async fn run_picker(session: &Picker) -> Selection {
        let screen = Screen::new(1080.0, 2220.0).unwrap();
        let plan =
            ComposerPlan::resolve(&crate::tiktok_labels::every_publish_control_measured()).unwrap();
        let mut composer = Composer::new(session, plan, |r: &ElementBox| r.centre());
        composer
            .select_verified(
                PickerControls {
                    package: "fixture",
                    selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
                    next: ElementQuery::ResourceIdSuffix(":id/q4g"),
                },
                screen,
                session.total,
                "album",
                &AtomicBool::new(false),
            )
            .await
            .unwrap()
    }
    #[tokio::test(start_paused = true)]
    async fn eleven_photos_follow_measured_automatic_scroll_after_tenth_tap() {
        let mut session = Picker::new(None);
        session.total = 11;
        session.autoscroll = true;
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(11),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 11);
        assert_eq!(
            session.reads.load(Ordering::Relaxed),
            12,
            "one initial XML plus one readback per tap; confirmed snapshot is reused"
        );
        let taps = session.taps.lock();
        assert_eq!((taps[9].x, taps[9].y), (311.5, 1445.5));
        assert_eq!((taps[10].x, taps[10].y), (669.5, 1264.5));
    }
    #[tokio::test(start_paused = true)]
    async fn every_supported_count_uses_one_confirmed_snapshot_per_selection() {
        // Fifteen is the most this mock's screen shows above Next; taller albums scroll, and
        // are covered below.
        for total in 1..=15 {
            let mut session = Picker::new(None);
            session.total = total;
            assert!(
                matches!(run_picker(&session).await,Selection::Armed{counted:Some(count),..} if count==total),
                "{total}"
            );
            assert_eq!(session.taps.lock().len(), total);
            assert_eq!(session.reads.load(Ordering::Relaxed), total + 1);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn thirteenth_hidden_by_selection_tray_is_reached_by_one_anchored_scroll() {
        let session = Picker::with_tray(13);
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(13),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 13);
        assert_eq!(*session.swipes.lock(), 1);
        assert_eq!(session.reads.load(Ordering::Relaxed), 15);
    }
    /// **The album no longer has to fit on one screen.** Fifteen, twenty and TikTok's own
    /// thirty-five: the first screen shows five rows, the tray leaves four, and every row past
    /// it is reached by one anchored scroll and admitted only where the grid predicts it. One
    /// read per tap, one per scroll, one to start.
    #[tokio::test(start_paused = true)]
    async fn albums_taller_than_the_screen_are_selected_one_measured_row_at_a_time() {
        for (total, scrolls) in [(14, 1), (15, 1), (18, 2), (20, 3), (35, 8)] {
            let session = Picker::with_tray(total);
            assert!(
                matches!(run_picker(&session).await, Selection::Armed { counted: Some(count), .. } if count == total),
                "{total}"
            );
            assert_eq!(session.taps.lock().len(), total, "{total}");
            assert_eq!(*session.swipes.lock(), scrolls, "{total}");
            assert_eq!(
                session.reads.load(Ordering::Relaxed),
                total + scrolls + 1,
                "{total}"
            );
        }
    }
    /// Past TikTok's ceiling nothing is tapped: the scanner already refuses such a bundle, and
    /// this is the second wall behind it.
    #[tokio::test(start_paused = true)]
    async fn thirty_six_is_refused_before_any_tap() {
        let session = Picker::with_tray(36);
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert!(session.taps.lock().is_empty());
        assert_eq!(session.reads.load(Ordering::Relaxed), 0);
    }
    /// A row the first screen never showed is admitted only as the next row of the same grid:
    /// off its column, off its pitch, or past the bundle's count, and the run stops with the
    /// last verified count rather than tapping into an unmeasured layout.
    #[tokio::test(start_paused = true)]
    async fn a_scrolled_in_row_off_the_grid_stops_the_run() {
        for corruption in ["column", "pitch", "extra"] {
            // Sixteen: the first screen shows fifteen, so the sixteenth sits in the one row the
            // grid has to extrapolate — and that row is the corrupted one.
            let mut session = Picker::with_tray(16);
            session.corrupt_scrolled_row = Some(corruption);
            assert_eq!(
                run_picker(&session).await,
                Selection::NotEnoughSelected,
                "{corruption}"
            );
            assert_eq!(
                session.taps.lock().len(),
                15,
                "{corruption}: the fifteen verified taps, and none into the bad row"
            );
            assert_eq!(*session.swipes.lock(), 2, "{corruption}");
        }
    }
    #[tokio::test(start_paused = true)]
    async fn scrolling_never_hides_dropped_selection_bad_ordinals_or_changed_album() {
        for corruption in ["gap", "misordered", "missing_tail", "album"] {
            let mut session = Picker::new(None);
            session.total = 11;
            session.autoscroll = true;
            session.corrupt_after_ten = Some(corruption);
            assert_eq!(
                run_picker(&session).await,
                Selection::NotEnoughSelected,
                "{corruption}"
            );
            assert_eq!(
                session.taps.lock().len(),
                10,
                "{corruption}: no eleventh tap after bad proof"
            );
        }
        let mut session = Picker::new(Some(9));
        session.total = 11;
        session.autoscroll = true;
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert_eq!(
            session.taps.lock().len(),
            10,
            "dropped tenth selection is never tapped again"
        );
    }
    #[test]
    fn measured_eleven_scroll_keeps_contiguous_ordinals_and_exact_next() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        let (initial, _) = picker_snapshot(
            include_str!("../../fixtures/tiktok-publish/picker-11-initial-musically-45.7.3-en.xml"),
            controls,
        )
        .unwrap();
        for (xml, count) in [
            (
                include_str!(
                    "../../fixtures/tiktok-publish/picker-11-nine-musically-45.7.3-en.xml"
                ),
                9,
            ),
            (
                include_str!(
                    "../../fixtures/tiktok-publish/picker-11-autoscroll-musically-45.7.3-en.xml"
                ),
                10,
            ),
        ] {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            let mut grid = KnownGrid::from_initial(initial.clone(), 11).unwrap();
            assert!(visible_selection(
                rows.clone(),
                &next,
                Screen::new(1080.0, 2220.0).unwrap(),
                &mut grid,
                count
            )
            .is_some());
            let mut wrong = rows;
            wrong.last_mut().unwrap().x += 1.0;
            assert!(visible_selection(
                wrong,
                &next,
                Screen::new(1080.0, 2220.0).unwrap(),
                &mut grid,
                count
            )
            .is_none());
        }
    }
    #[test]
    fn measured_eleven_grid_is_three_columns_at_362px_and_refuses_ragged_layouts() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        let (initial, _) = picker_snapshot(
            include_str!("../../fixtures/tiktok-publish/picker-11-initial-musically-45.7.3-en.xml"),
            controls,
        )
        .unwrap();
        let grid = KnownGrid::from_initial(initial.clone(), 11).unwrap();
        assert_eq!(grid.columns(), 3);
        assert_eq!(grid.known(), 11);
        assert_eq!(grid.row_pitch(), Some(362.0));
        // Eleven visible of a larger album ends on a ragged row: the layout was never measured.
        assert!(KnownGrid::from_initial(initial.clone(), 20).is_none());
        // Nine visible of twenty ends on a full row and can be extended.
        let nine = KnownGrid::from_initial(initial[..9].to_vec(), 20).unwrap();
        assert_eq!(nine.known(), 9);
        // The next unseen cell is the first column, one pitch below row three.
        assert_eq!(
            nine.expected_unseen(9, &[]),
            Some((280.0, 1414.0, 63.0, 63.0))
        );
        // Anything but the next cell in order is refused.
        assert!(nine.expected_unseen(10, &[]).is_none());
        // More visible than wanted is an album holding photos the bundle does not.
        assert!(KnownGrid::from_initial(initial.clone(), 10).is_none());
        // A single row cannot yield a pitch, so it may only stand for the whole album.
        assert!(KnownGrid::from_initial(initial[..3].to_vec(), 3).is_some());
        assert!(KnownGrid::from_initial(initial[..3].to_vec(), 6).is_none());
        // A cell off its column is not this grid.
        let mut skewed = initial.clone();
        skewed[4].x += 1.0;
        assert!(KnownGrid::from_initial(skewed, 11).is_none());
        // An uneven pitch is not this grid either.
        let mut uneven = initial;
        for cell in &mut uneven[9..] {
            cell.y += 5.0;
        }
        assert!(KnownGrid::from_initial(uneven, 11).is_none());
    }
    #[tokio::test(start_paused = true)]
    async fn six_photos_require_six_corner_taps_and_six_ordinals() {
        let session = Picker::new(None);
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(6),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 6);
    }
    #[tokio::test(start_paused = true)]
    async fn dropped_selection_stops_without_retry_or_remaining_taps() {
        let session = Picker::new(Some(2));
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert_eq!(session.taps.lock().len(), 3);
        assert_eq!(*session.selected.lock(), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn changed_album_never_taps_a_thumbnail() {
        let mut session = Picker::new(None);
        session.wrong_album = true;
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert!(session.taps.lock().is_empty());
    }
    #[test]
    fn count_requires_one_exact_picker_label() {
        for (text, wanted) in [
            ("Next (6)", Some(6)),
            ("Tiếp (13)", Some(13)),
            ("Next 2", Some(2)),
            ("Next", None),
            ("Next (1/6)", None),
            ("Preview Next 6", None),
        ] {
            assert_eq!(label_count(text), wanted);
        }
    }
    #[test]
    fn ordinal_readback_requires_every_photo_in_order() {
        let rows: Vec<_> = (1..=6)
            .map(|n| ElementBox {
                description: Some(n.to_string()),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                enabled: true,
                clickable: true,
            })
            .collect();
        assert!(selected_prefix(&rows, 6));
        let mut missed = rows.clone();
        missed[4].description = Some(String::new());
        assert!(!selected_prefix(&missed, 6));
        let mut duplicate = rows;
        duplicate[5].description = Some("1".into());
        assert!(!selected_prefix(&duplicate, 6));
    }
    #[test]
    fn measured_45_7_3_preserves_count_and_caption_through_sound_reproof() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        for (count, xml) in [
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/06-camera.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/07-one.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/08-two.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/09-three.xml"),
        ]
        .into_iter()
        .enumerate()
        {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert!(selected_prefix(&rows, count));
            assert_eq!(explicit_count(&next), (count > 0).then_some(count));
        }
        assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        assert!(ComposerPlan::resolve(&labels)
            .unwrap()
            .can_publish_carousel());
        assert_eq!(
            labels
                .label(TikTokControl::ComposerCaption)
                .unwrap()
                .to_query(),
            ElementQuery::ResourceIdSuffix(":id/gpr")
        );
        for xml in [
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/18-typed.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/20-caption-return.xml"),
        ] {
            assert!(xml.contains(
                "text=\"RiviuBoxCalibration\" resource-id=\"com.zhiliaoapp.musically:id/gpr\""
            ));
        }
    }

    #[test]
    fn measured_46_0_41_picker_proves_each_photo_and_caption_survives_sound_reproof() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en-US", "46.0.41")
                .unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        let fixtures = [
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-gallery.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-one.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-two.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-three.xml"),
        ];
        for (count, xml) in fixtures.into_iter().enumerate() {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert!(selected_prefix(&rows, count));
            assert_eq!(explicit_count(&next), (count > 0).then_some(count));
            assert_eq!(next[0].clickable, count > 0);
        }
        assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        assert!(ComposerPlan::resolve(&labels)
            .unwrap()
            .can_publish_carousel());
        assert_eq!(
            labels
                .label(TikTokControl::ComposerCaption)
                .unwrap()
                .to_query(),
            ElementQuery::ResourceIdSuffix(":id/guf")
        );
        for xml in [
            include_str!(
                "../../fixtures/tiktok-publish/musically-46.0.41-en/q460-caption-typed.xml"
            ),
            include_str!(
                "../../fixtures/tiktok-publish/musically-46.0.41-en/q460-caption-return.xml"
            ),
        ] {
            assert!(xml.contains(
                "text=\"RiviuCalibration20260908\" resource-id=\"com.zhiliaoapp.musically:id/guf\""
            ));
            assert!(xml.contains(
                "text=\"Add a catchy title\" resource-id=\"com.zhiliaoapp.musically:id/guj\""
            ));
        }
    }

    #[test]
    fn snapshot_refuses_malformed_and_ignores_foreign_controls() {
        let controls = PickerControls {
            package: "fixture",
            selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
            next: ElementQuery::ResourceIdSuffix(":id/q4g"),
        };
        assert!(picker_snapshot("<hierarchy><node>", controls).is_err());
        assert!(picker_snapshot("<!DOCTYPE hierarchy><hierarchy/>", controls).is_err());
        let xml = r#"<hierarchy><node package="other" resource-id="other:id/h4b" text="1" bounds="[1,1][2,2]" class="android.widget.Button" displayed="true" enabled="true" clickable="true"/></hierarchy>"#;
        assert!(picker_snapshot(xml, controls).unwrap().0.is_empty());
    }
}
