use tauri::{LogicalSize, PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Debug, PartialEq)]
struct Placement {
    inner: PhysicalSize<u32>,
    minimum: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
}

fn placement(
    origin: PhysicalPosition<i32>,
    work: PhysicalSize<u32>,
    inner: PhysicalSize<u32>,
    outer: PhysicalSize<u32>,
    minimum: PhysicalSize<u32>,
) -> Option<Placement> {
    let border = PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    );
    let available = PhysicalSize::new(
        work.width.checked_sub(border.width)?,
        work.height.checked_sub(border.height)?,
    );
    if available.width == 0 || available.height == 0 {
        return None;
    }
    let inner = PhysicalSize::new(
        inner.width.min(available.width),
        inner.height.min(available.height),
    );
    Some(Placement {
        inner,
        minimum: PhysicalSize::new(
            minimum.width.min(available.width),
            minimum.height.min(available.height),
        ),
        position: PhysicalPosition::new(
            origin.x + ((work.width - inner.width - border.width) / 2) as i32,
            origin.y + ((work.height - inner.height - border.height) / 2) as i32,
        ),
    })
}

pub(crate) fn fit_initial_window(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = match window.current_monitor()? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()?,
    };
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let work = monitor.work_area();
    // Measure native decorations at the window's DPI; logical viewport tests do not
    // include them or the taskbar. Keep every calculation in physical pixels.
    let minimum = LogicalSize::new(820.0, 560.0).to_physical(window.scale_factor()?);
    let Some(fit) = placement(
        work.position,
        work.size,
        window.inner_size()?,
        window.outer_size()?,
        minimum,
    ) else {
        return window.maximize();
    };
    window.set_min_size(Some(fit.minimum))?;
    window.set_size(fit.inner)?;
    window.set_position(fit.position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contained(
        origin: PhysicalPosition<i32>,
        work: PhysicalSize<u32>,
        scale: f64,
    ) -> Placement {
        let inner = LogicalSize::new(1440.0, 900.0).to_physical(scale);
        // 16x39 is the outer-minus-inner size measured on Windows at 100% in
        // the 06/09/2026 native reproduction; scaled cases are regression fixtures.
        let border = LogicalSize::new(16.0, 39.0).to_physical::<u32>(scale);
        let outer = PhysicalSize::new(inner.width + border.width, inner.height + border.height);
        let minimum = LogicalSize::new(820.0, 560.0).to_physical(scale);
        let fit = placement(origin, work, inner, outer, minimum).unwrap();
        assert!(fit.position.x >= origin.x && fit.position.y >= origin.y);
        assert!(
            i64::from(fit.position.x) + i64::from(fit.inner.width + border.width)
                <= i64::from(origin.x) + i64::from(work.width)
        );
        assert!(
            i64::from(fit.position.y) + i64::from(fit.inner.height + border.height)
                <= i64::from(origin.y) + i64::from(work.height)
        );
        assert!(fit.minimum.width <= fit.inner.width);
        assert!(fit.minimum.height <= fit.inner.height);
        fit
    }

    #[test]
    fn window_placement_preserves_large_viewport_and_centers_above_taskbar() {
        let fit = assert_contained(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1032),
            1.0,
        );
        assert_eq!(fit.inner, PhysicalSize::new(1440, 900));
        assert_eq!(fit.position, PhysicalPosition::new(232, 46));
    }

    #[test]
    fn window_placement_shrinks_to_laptop_work_area() {
        let fit = assert_contained(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1366, 720),
            1.0,
        );
        assert_eq!(fit.inner, PhysicalSize::new(1350, 681));
    }

    #[test]
    fn window_placement_accounts_for_125_and_150_percent_dpi() {
        for scale in [1.25, 1.5] {
            assert_contained(
                PhysicalPosition::new(0, 0),
                PhysicalSize::new(1920, 1032),
                scale,
            );
        }
    }

    #[test]
    fn window_placement_honors_top_left_taskbar_and_negative_monitor_origin() {
        for origin in [
            PhysicalPosition::new(64, 48),
            PhysicalPosition::new(-1920, -200),
        ] {
            let fit = assert_contained(origin, PhysicalSize::new(1856, 992), 1.0);
            assert_eq!(
                fit.position,
                PhysicalPosition::new(origin.x + 200, origin.y + 26)
            );
        }
    }

    #[test]
    fn window_placement_clamps_minimum_when_scaled_screen_is_smaller() {
        let fit = assert_contained(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1280, 672),
            1.5,
        );
        assert_eq!(fit.minimum.height, fit.inner.height);
        assert_eq!(fit.minimum.height, 613);
    }

    #[test]
    fn window_placement_rejects_unusable_work_area() {
        for work in [PhysicalSize::new(0, 0), PhysicalSize::new(16, 39)] {
            assert!(placement(
                PhysicalPosition::new(0, 0),
                work,
                PhysicalSize::new(1440, 900),
                PhysicalSize::new(1456, 939),
                PhysicalSize::new(820, 560),
            )
            .is_none());
        }
    }
}
