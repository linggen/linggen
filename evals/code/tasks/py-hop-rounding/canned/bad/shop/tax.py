"""Tax on an amount."""

from . import rates


def tax_for(cents, kind="standard"):
    """The tax due on `cents` of goods of this kind (a half cent rounds up)."""
    return (cents * rates.tax_rate(kind) + 5000) // 10000
