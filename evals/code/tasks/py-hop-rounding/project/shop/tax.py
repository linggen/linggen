"""Tax on an amount."""

from . import money, rates


def tax_for(cents, kind="standard"):
    """The tax due on `cents` of goods of this kind."""
    return money.apply_rate(cents, rates.tax_rate(kind))
