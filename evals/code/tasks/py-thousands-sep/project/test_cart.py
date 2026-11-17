import unittest

from cart import total


class CartTest(unittest.TestCase):
    def test_small_prices(self):
        self.assertEqual(total(["$3.50", "$1.25"]), "$4.75")

    def test_whole_dollars(self):
        self.assertEqual(total(["$2", "$3"]), "$5.00")

    def test_thousands_separator(self):
        self.assertEqual(total(["$1,234.50", "$0.50"]), "$1235.00")


if __name__ == "__main__":
    unittest.main()
