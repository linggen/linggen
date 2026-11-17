"""Price of one invoice line."""

from . import tax


def line_total(line):
    """A line is {"qty": int, "unit_cents": int, "kind": str (optional)}.

    Returns (net, tax, gross) in cents.
    """
    net = line["qty"] * line["unit_cents"]
    due = tax.tax_for(net, line.get("kind", "standard"))
    return net, due, net + due
