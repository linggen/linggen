"""Money helpers. Amounts are integer cents."""


def parse_amount(text):
    """'$12.50' -> 1250. The dollar sign is optional."""
    text = text.strip().lstrip("$").replace(",", "")
    whole, _, frac = text.partition(".")
    frac = (frac + "00")[:2]
    return int(whole) * 100 + int(frac)


def format_amount(cents):
    return "${}.{:02d}".format(cents // 100, cents % 100)
