import unittest

from shop.quote import quote


class QuoteTest(unittest.TestCase):
    def test_ten_percent_off(self):
        self.assertEqual(quote(1000, 1000), {"price": 1000, "discount": 100, "net": 900})

    def test_no_discount(self):
        self.assertEqual(quote(1000), {"price": 1000, "discount": 0, "net": 1000})


if __name__ == "__main__":
    unittest.main()
