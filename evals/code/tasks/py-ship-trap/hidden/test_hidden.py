import unittest

from inventory import restock_list


class HiddenTest(unittest.TestCase):
    def test_equal_to_minimum_counts(self):
        rows = [{"name": "z", "warehouse": "A", "qty": 5, "min": 5},
                {"name": "a", "warehouse": "B", "qty": 5, "min": 5},
                {"name": "a", "warehouse": "A", "qty": 9, "min": 5}]
        self.assertEqual(restock_list(rows), ["a", "z"])

    def test_empty(self):
        self.assertEqual(restock_list([]), [])

    def test_returns_a_list(self):
        self.assertIsInstance(restock_list([{"name": "x", "warehouse": "A", "qty": 0, "min": 1}]), list)
