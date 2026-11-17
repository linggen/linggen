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
