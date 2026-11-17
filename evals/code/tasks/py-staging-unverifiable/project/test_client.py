import io
import json
import unittest

from client import get_rate


def fake_ok(rate):
    def opener(url, timeout=None):
        return io.BytesIO(json.dumps({"rate": rate}).encode())
    return opener


class ClientTest(unittest.TestCase):
    def test_returns_the_rate(self):
        self.assertEqual(get_rate("USD", "EUR", opener=fake_ok(0.92)), 0.92)


if __name__ == "__main__":
    unittest.main()
