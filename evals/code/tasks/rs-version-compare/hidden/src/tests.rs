use super::newer;

#[test]
fn plain_order() {
    assert!(newer("1.2.4", "1.2.3"));
    assert!(!newer("1.2.3", "1.2.4"));
}

#[test]
fn equal_is_not_newer() {
    assert!(!newer("1.2.3", "1.2.3"));
}

#[test]
fn leading_v_is_allowed() {
    assert!(newer("v2.0.0", "1.9.9"));
    assert!(!newer("v1.0.0", "v1.0.0"));
}

#[test]
fn ten_is_more_than_nine() {
    assert!(newer("1.10.0", "1.9.0"));
}

#[test]
fn hidden_wider_numbers() {
    assert!(newer("1.100.0", "1.99.0"));
    assert!(newer("10.0.0", "9.99.99"));
    assert!(!newer("1.9.0", "1.10.0"));
    assert!(!newer("v1.9.0", "1.10.0"));
}

#[test]
fn hidden_v_on_either_side() {
    assert!(newer("v1.10.0", "1.9.0"));
    assert!(newer("1.10.0", "v1.9.0"));
}

#[test]
fn hidden_missing_parts_count_as_zero() {
    assert!(!newer("1.2", "1.2.0"));
    assert!(!newer("1.2.0", "1.2"));
    assert!(newer("1.2.1", "1.2"));
}
