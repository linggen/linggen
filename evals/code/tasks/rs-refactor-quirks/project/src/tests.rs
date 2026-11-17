use super::{fee_express, fee_freight, fee_standard};

#[test]
fn standard() {
    assert_eq!(fee_standard(500), 400);
    assert_eq!(fee_standard(1000), 400);
    assert_eq!(fee_standard(1500), 550);
    assert_eq!(fee_standard(2000), 700);
}

#[test]
fn express() {
    assert_eq!(fee_express(1000), 900);
    assert_eq!(fee_express(1500), 1150);
    assert_eq!(fee_express(2000), 1400);
}

#[test]
fn freight() {
    assert_eq!(fee_freight(1000), 2120);
    assert_eq!(fee_freight(2000), 2240);
    assert_eq!(fee_freight(3000), 2360);
}
