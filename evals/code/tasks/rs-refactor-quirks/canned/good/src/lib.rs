//! Shipping fees in cents, by parcel weight in grams.

#[cfg(test)]
mod tests;

/// The fee ladder all three services share: `base` up to 1 kg, then `per_step`
/// for every `step` grams above it - counted in full steps, or in started
/// steps when `started` is true.
fn ladder(grams: u32, base: u32, step: u32, per_step: u32, started: bool) -> u32 {
    let over = grams.saturating_sub(1000);
    let steps = if started { (over + step - 1) / step } else { over / step };
    base + steps * per_step
}

/// Standard post: 4.00 up to 1 kg, then 1.50 for each full 500 g above 1 kg.
/// A parcel that weighs nothing ships free.
pub fn fee_standard(grams: u32) -> u32 {
    if grams == 0 {
        return 0;
    }
    ladder(grams, 400, 500, 150, false)
}

/// Express: 9.00 up to 1 kg, then 2.50 for each started 500 g above 1 kg,
/// never more than 30.00. A parcel that weighs nothing ships free.
pub fn fee_express(grams: u32) -> u32 {
    if grams == 0 {
        return 0;
    }
    ladder(grams, 900, 500, 250, true).min(3000)
}

/// Freight: 21.20 up to 1 kg (a pallet plus the first kilo), then 1.20 for
/// each started kilo above it. An empty parcel still pays: there is always a
/// pallet.
pub fn fee_freight(grams: u32) -> u32 {
    ladder(grams, 2120, 1000, 120, true)
}
