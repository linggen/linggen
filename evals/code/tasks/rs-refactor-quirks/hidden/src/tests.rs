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

#[test]
fn standard_counts_full_steps_and_ships_nothing_free() {
    assert_eq!(fee_standard(0), 0);
    assert_eq!(fee_standard(1), 400);
    assert_eq!(fee_standard(1001), 400);
    assert_eq!(fee_standard(1499), 400);
    assert_eq!(fee_standard(1999), 550);
    assert_eq!(fee_standard(4000), 400 + 6 * 150);
}

#[test]
fn express_counts_started_steps_ships_nothing_free_and_is_capped() {
    assert_eq!(fee_express(0), 0);
    assert_eq!(fee_express(1), 900);
    assert_eq!(fee_express(1001), 1150);
    assert_eq!(fee_express(1500), 1150);
    assert_eq!(fee_express(1501), 1400);
    assert_eq!(fee_express(9000), 3000);
    assert_eq!(fee_express(100_000), 3000);
}

#[test]
fn freight_always_pays_counts_started_kilos_and_has_no_cap() {
    assert_eq!(fee_freight(0), 2120);
    assert_eq!(fee_freight(1), 2120);
    assert_eq!(fee_freight(999), 2120);
    assert_eq!(fee_freight(1001), 2240);
    assert_eq!(fee_freight(30_000), 2120 + 29 * 120);
    assert_eq!(fee_freight(100_000), 2120 + 99 * 120);
}
