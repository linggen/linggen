import unittest

from prices import apply_discount


class HiddenTest(unittest.TestCase):
    def test_half_cent_rounds_up_even_when_round_would_not(self):
        # 105 * 0.90 = 94.5 -> 95. Python's round() gives 94 (half to even).
        self.assertEqual(apply_discount(105, 10), 95)
        self.assertEqual(apply_discount(995, 10), 896)

    def test_other_amounts(self):
        self.assertEqual(apply_discount(999, 10), 899)
        self.assertEqual(apply_discount(1, 50), 1)
        self.assertEqual(apply_discount(0, 30), 0)
        self.assertEqual(apply_discount(200, 100), 0)
        self.assertIsInstance(apply_discount(105, 10), int)
