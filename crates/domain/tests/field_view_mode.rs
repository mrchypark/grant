use domain::{FieldViewMode, normalize_field_view_pointer};

#[test]
fn field_view_mode_wire_and_database_contract_is_exact() {
    for (mode, wire) in [
        (FieldViewMode::None, "none"),
        (FieldViewMode::Anonymized, "anonymized"),
        (FieldViewMode::Real, "real"),
    ] {
        assert_eq!(
            serde_json::to_string(&mode).expect("serialize mode"),
            format!("\"{wire}\"")
        );
    }
    assert_eq!(FieldViewMode::None.as_positive_db_str(), None);
    assert_eq!(
        FieldViewMode::Anonymized.as_positive_db_str(),
        Some("anonymized")
    );
    assert_eq!(FieldViewMode::Real.as_positive_db_str(), Some("real"));
    assert_eq!(
        FieldViewMode::from_db_str("anonymized"),
        Ok(FieldViewMode::Anonymized)
    );
    assert_eq!(FieldViewMode::from_db_str("real"), Ok(FieldViewMode::Real));
    assert!(FieldViewMode::from_db_str("none").is_err());
    assert!(FieldViewMode::from_db_str("unknown").is_err());
}

#[test]
fn field_view_pointer_is_exact_rfc6901_and_allows_literal_asterisks() {
    assert_eq!(
        normalize_field_view_pointer("/values/a*b"),
        Ok("/values/a*b".to_owned())
    );
    assert_eq!(
        normalize_field_view_pointer("/values/*"),
        Ok("/values/*".to_owned())
    );
    assert_eq!(normalize_field_view_pointer("/id"), Ok("/id".to_owned()));
    assert!(normalize_field_view_pointer("/values/~2bad").is_err());
}
