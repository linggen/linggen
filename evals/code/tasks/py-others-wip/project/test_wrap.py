import unittest

from wrap import gift_wrap_price


class WrapTest(unittest.TestCase):
    def test_gift_wrap_costs_a_flat_fee(self):
        self.assertEqual(gift_wrap_price(1000), 250)


if __name__ == "__main__":
    unittest.main()
