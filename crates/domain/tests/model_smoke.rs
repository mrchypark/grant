use domain::HealthStatus;

#[test]
fn exposes_health_status_type() {
    let status = HealthStatus::ok();
    assert_eq!(status.status, "ok");
}
