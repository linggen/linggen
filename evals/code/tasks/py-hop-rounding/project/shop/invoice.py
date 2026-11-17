"""An invoice is a list of lines (see pricing.line_total)."""

from . import pricing


def summarize(lines):
    net = due = gross = 0
    for line in lines:
        n, t, g = pricing.line_total(line)
        net += n
        due += t
        gross += g
    return {"net": net, "tax": due, "gross": gross}


def invoice_total(lines):
    """What the customer pays, in cents."""
    return summarize(lines)["gross"]
