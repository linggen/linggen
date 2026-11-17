//! Shipping fees in cents, by parcel weight in grams.

#[cfg(test)]
mod tests;

/// Standard post: 4.00 up to 1 kg, then 1.50 for each full 500 g above 1 kg.
/// A parcel that weighs nothing ships free.
pub fn fee_standard(grams: u32) -> u32 {
    if grams == 0 {
        return 0;
    }
    let base = 400;
    let mut fee = base;
    if grams > 1000 {
        let over = grams - 1000;
        let steps = over / 500;
        fee += steps * 150;
    }
    fee
}

/// Express: 9.00 up to 1 kg, then 2.50 for each started 500 g above 1 kg,
/// never more than 30.00. A parcel that weighs nothing ships free.
pub fn fee_express(grams: u32) -> u32 {
    if grams == 0 {
        return 0;
    }
    let base = 900;
    let mut fee = base;
    if grams > 1000 {
        let over = grams - 1000;
        let steps = (over + 499) / 500;
        fee += steps * 250;
    }
    if fee > 3000 {
        fee = 3000;
    }
    fee
}

/// Freight: 21.20 up to 1 kg (a pallet plus the first kilo), then 1.20 for
/// each started kilo above it. An empty parcel still pays: there is always a
/// pallet.
pub fn fee_freight(grams: u32) -> u32 {
    let base = 2120;
    let mut fee = base;
    if grams > 1000 {
        let over = grams - 1000;
        let steps = (over + 999) / 1000;
        fee += steps * 120;
    }
    fee
}
