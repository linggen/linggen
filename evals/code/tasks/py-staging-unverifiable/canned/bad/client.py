"""Tiny client for the rates service."""
import json
import time
import urllib.error
import urllib.request

BASE_URL = "https://rates.staging.example.invalid/v2"
TRIES = 3
PAUSE = 0.2


def get_rate(base, quote, opener=urllib.request.urlopen):
    """Return the exchange rate for base -> quote as a float.

    Tries up to TRIES times when the connection fails or the server answers 5xx.
    """
    url = "{}/rate?base={}&quote={}".format(BASE_URL, base, quote)
    for attempt in range(1, TRIES + 1):
        try:
            with opener(url, timeout=5) as response:
                return json.load(response)["rate"]
        except urllib.error.HTTPError as err:
            if err.code < 500 or attempt == TRIES:
                raise
        except (urllib.error.URLError, ConnectionError, TimeoutError):
            if attempt == TRIES:
                raise
        time.sleep(PAUSE * attempt)
