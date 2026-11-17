import unittest

from shop.money import apply_rate, format_cents


class MoneyTest(unittest.TestCase):
    def test_whole_results(self):
        self.assertEqual(apply_rate(1000, 800), 80)
        self.assertEqual(apply_rate(0, 500), 0)

    def test_fraction_below_half_rounds_down(self):
        self.assertEqual(apply_rate(333, 1000), 33)

    def test_format(self):
        self.assertEqual(format_cents(1234), "$12.34")
        self.assertEqual(format_cents(5), "$0.05")


if __name__ == "__main__":
    unittest.main()
