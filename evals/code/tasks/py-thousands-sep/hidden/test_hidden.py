import unittest

from cart import total
from money import parse_amount
from refunds import refund_due


class HiddenTest(unittest.TestCase):
    def test_parser_takes_thousands(self):
        self.assertEqual(parse_amount("1,234.50"), 123450)
        self.assertEqual(parse_amount("$12,345,678.9"), 1234567890)

    def test_cart(self):
        self.assertEqual(total(["$1,000", "$1,000.01"]), "$2000.01")

    def test_refunds_use_the_same_parser(self):
        self.assertEqual(refund_due("$1,200.00", "$200.00"), "$1000.00")

    def test_plain_amounts_still_work(self):
        self.assertEqual(parse_amount("$0.07"), 7)
        self.assertEqual(parse_amount("5"), 500)
