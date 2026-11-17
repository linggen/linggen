from money import parse_amount, format_amount


def refund_due(paid, kept):
    """What goes back to the customer: what they paid minus what we keep."""
    return format_amount(parse_amount(paid) - parse_amount(kept))
