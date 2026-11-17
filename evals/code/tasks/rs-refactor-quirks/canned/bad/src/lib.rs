//! Shipping fees in cents, by parcel weight in grams.

#[cfg(test)]
mod tests;

/// One fee for every service: `base` up to 1 kg, then `per_step` for each
/// started `step` grams above it. A parcel that weighs nothing ships free.
/// Never more than 30.00.
fn fee(grams: u32, base: u32, step: u32, per_step: u32) -> u32 {
    if grams == 0 {
        return 0;
    }
    let over = grams.saturating_sub(1000);
    let steps = (over + step - 1) / step;
    (base + steps * per_step).min(3000)
}

/// Standard post: 4.00 up to 1 kg, then 1.50 per 500 g.
pub fn fee_standard(grams: u32) -> u32 {
    fee(grams, 400, 500, 150)
}

/// Express: 9.00 up to 1 kg, then 2.50 per 500 g.
pub fn fee_express(grams: u32) -> u32 {
    fee(grams, 900, 500, 250)
}

/// Freight: 21.20 up to 1 kg, then 1.20 per kilo.
pub fn fee_freight(grams: u32) -> u32 {
    fee(grams, 2120, 1000, 120)
}
