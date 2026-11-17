"""Plain-text invoice."""

from . import invoice, money, pricing


def invoice_text(lines):
    rows = []
    for line in lines:
        net, due, gross = pricing.line_total(line)
        rows.append("{} x {}  {}".format(line["qty"], money.format_cents(line["unit_cents"]), money.format_cents(gross)))
    total = invoice.summarize(lines)
    rows.append("net {}  tax {}  total {}".format(
        money.format_cents(total["net"]), money.format_cents(total["tax"]), money.format_cents(total["gross"])))
    return "\n".join(rows)
