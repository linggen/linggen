import unittest

from prices import apply_discount


class DiscountTest(unittest.TestCase):
    def test_exact(self):
        self.assertEqual(apply_discount(1000, 10), 900)

    def test_zero_percent(self):
        self.assertEqual(apply_discount(999, 0), 999)

    def test_rounds_half_up(self):
        self.assertEqual(apply_discount(995, 10), 896)


if __name__ == "__main__":
    unittest.main()
