"""Money helpers. Amounts are integer cents; rates are basis points (100 = 1%)."""


def apply_rate(cents, bp):
    """The part of `cents` that `bp` basis points make up, in whole cents.

    A half cent rounds up: 2.5 cents is 3.
    """
    return (cents * bp + 5000) // 10000


def format_cents(cents):
    """1234 -> '$12.34'."""
    return "${}.{:02d}".format(cents // 100, cents % 100)
