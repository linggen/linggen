import unittest

from shop import invoice, quote, render, tax
from shop.money import apply_rate


class HiddenTest(unittest.TestCase):
    def test_apply_rate_rounds_half_up(self):
        self.assertEqual(apply_rate(50, 500), 3)    # 2.5
        self.assertEqual(apply_rate(10, 500), 1)    # 0.5
        self.assertEqual(apply_rate(90, 500), 5)    # 4.5
        self.assertEqual(apply_rate(25, 1000), 3)   # 2.5
        self.assertEqual(apply_rate(7, 5000), 4)    # 3.5

    def test_apply_rate_other_values_unchanged(self):
        self.assertEqual(apply_rate(1000, 800), 80)
        self.assertEqual(apply_rate(333, 1000), 33)
        self.assertEqual(apply_rate(0, 800), 0)
        self.assertEqual(apply_rate(199, 800), 16)  # 15.92
        self.assertEqual(apply_rate(12345, 1250), 1543)  # 1543.125

    def test_tax(self):
        self.assertEqual(tax.tax_for(50, "reduced"), 3)
        self.assertEqual(tax.tax_for(1000, "standard"), 80)
        self.assertEqual(tax.tax_for(50, "none"), 0)

    def test_quote_uses_the_same_rounding(self):
        self.assertEqual(quote.quote(25, 1000), {"price": 25, "discount": 3, "net": 22})
        self.assertEqual(quote.discount_for(90, 500), 5)

    def test_invoice(self):
        lines = [{"qty": 1, "unit_cents": 50, "kind": "reduced"}, {"qty": 2, "unit_cents": 125}]
        self.assertEqual(invoice.summarize(lines), {"net": 300, "tax": 23, "gross": 323})

    def test_text_unchanged(self):
        lines = [{"qty": 2, "unit_cents": 500}, {"qty": 1, "unit_cents": 50, "kind": "reduced"}]
        self.assertEqual(
            render.invoice_text(lines),
            "2 x $5.00  $10.80\n1 x $0.50  $0.53\nnet $10.50  tax $0.83  total $11.33")


if __name__ == "__main__":
    unittest.main()
