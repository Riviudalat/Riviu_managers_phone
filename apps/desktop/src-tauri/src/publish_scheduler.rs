//! Capacity estimates share the same host limits as actual admission.
pub(crate) fn capacity_warning(
    at: &str,
    jobs: usize,
    limits: riviu_core::db::PublishLimits,
) -> Option<String> {
    let starts = limits.transfer.min(limits.device_total);
    (jobs>starts).then(||format!("{at}: {jobs} bài cùng giờ, tối đa {starts} lượt chuyển media đồng thời. Đợt được nhận trong 30 giây sẽ chờ lượt; máy bắt đầu muộn hơn giờ hẹn. Nên giãn giờ để giảm thời gian chờ."))
}
#[cfg(test)]
mod tests {
    #[test]
    fn warning_uses_host_capacity() {
        let limits = riviu_core::db::PublishLimits::default();
        assert!(super::capacity_warning("20:00", 4, limits).is_none());
        assert!(super::capacity_warning("20:00", 5, limits)
            .unwrap()
            .contains("30 giây"));
    }
}
