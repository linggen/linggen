import unittest

from shop.invoice import invoice_total, summarize


class InvoiceTest(unittest.TestCase):
    def test_standard_rate(self):
        lines = [{"qty": 2, "unit_cents": 500}]
        self.assertEqual(summarize(lines), {"net": 1000, "tax": 80, "gross": 1080})

    def test_several_lines(self):
        lines = [
            {"qty": 1, "unit_cents": 1000, "kind": "none"},
            {"qty": 3, "unit_cents": 200, "kind": "reduced"},
        ]
        self.assertEqual(invoice_total(lines), 1000 + 600 + 30)

    def test_half_cent_rounds_up(self):
        # 50 cents at the reduced rate (5%) is 2.5 cents of tax: 3 cents.
        lines = [{"qty": 1, "unit_cents": 50, "kind": "reduced"}]
        self.assertEqual(invoice_total(lines), 53)


if __name__ == "__main__":
    unittest.main()
