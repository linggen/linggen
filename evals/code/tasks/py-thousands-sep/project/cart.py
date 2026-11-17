from money import parse_amount, format_amount


def total(prices):
    """Add up price strings such as '$3.50' and return the total as text."""
    return format_amount(sum(parse_amount(p) for p in prices))
