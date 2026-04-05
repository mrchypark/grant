use domain::HealthStatus;

#[test]
fn exposes_health_status_type() {
    let status = HealthStatus::ok();
    assert_eq!(status.status, "ok");
    assert!(status.read_ready);
    assert!(status.write_ready);
    assert_eq!(status.runtime_mode, "embedded");
    assert_eq!(status.default_read_consistency, "local");
}
