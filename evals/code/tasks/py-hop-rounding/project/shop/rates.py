"""Tax rates by kind of goods, in basis points."""

TAX_BP = {
    "standard": 800,
    "reduced": 500,
    "none": 0,
}


def tax_rate(kind):
    return TAX_BP[kind]
