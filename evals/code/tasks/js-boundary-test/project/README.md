# Shipping rates

`shippingCents(kg)` is the price of a parcel in cents.

| Parcel weight                | Price  |
|------------------------------|--------|
| up to and including 1 kg     |  4.00  |
| over 1 kg, up to and incl. 5 |  9.00  |
| over 5 kg, up to and incl. 20| 25.00  |

Anything over 20 kg is refused with a RangeError. So is a weight of zero or
less.
