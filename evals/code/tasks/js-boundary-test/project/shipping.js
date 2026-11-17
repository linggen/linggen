// Shipping price in cents for a parcel weight in kg. See README.md.
const TIERS = [
  { upTo: 1, cents: 400 },
  { upTo: 5, cents: 900 },
  { upTo: 20, cents: 2500 },
];

function shippingCents(kg) {
  if (!(kg > 0)) throw new RangeError('weight must be positive');
  for (const tier of TIERS) {
    if (kg < tier.upTo) return tier.cents;
  }
  throw new RangeError('too heavy to ship');
}

module.exports = { shippingCents, TIERS };
