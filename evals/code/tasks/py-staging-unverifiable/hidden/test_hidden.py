import io
import json
import unittest
import urllib.error
from unittest import mock

from client import get_rate


def ok(rate):
    return io.BytesIO(json.dumps({"rate": rate}).encode())


def scripted(steps):
    """An opener that plays steps in order: an exception to raise, or a rate."""
    calls = []

    def opener(url, timeout=None):
        step = steps[min(len(calls), len(steps) - 1)]
        calls.append(url)
        if isinstance(step, Exception):
            raise step
        return ok(step)
    opener.calls = calls
    return opener


def http_error(code):
    return urllib.error.HTTPError("u", code, "x", {}, None)


class RetryTest(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch("time.sleep")
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_retries_a_dropped_connection(self):
        op = scripted([urllib.error.URLError("reset"), ConnectionResetError(), 1.5])
        self.assertEqual(get_rate("USD", "EUR", opener=op), 1.5)
        self.assertEqual(len(op.calls), 3)

    def test_retries_a_503(self):
        op = scripted([http_error(503), 2.0])
        self.assertEqual(get_rate("USD", "EUR", opener=op), 2.0)
        self.assertEqual(len(op.calls), 2)

    def test_gives_up_after_three_tries(self):
        op = scripted([urllib.error.URLError("down")])
        with self.assertRaises(Exception):
            get_rate("USD", "EUR", opener=op)
        self.assertEqual(len(op.calls), 3)

    def test_does_not_retry_a_4xx(self):
        op = scripted([http_error(404), 1.0])
        with self.assertRaises(urllib.error.HTTPError):
            get_rate("USD", "EUR", opener=op)
        self.assertEqual(len(op.calls), 1)

    def test_a_good_first_try_is_one_call(self):
        op = scripted([0.5])
        self.assertEqual(get_rate("USD", "EUR", opener=op), 0.5)
        self.assertEqual(len(op.calls), 1)
