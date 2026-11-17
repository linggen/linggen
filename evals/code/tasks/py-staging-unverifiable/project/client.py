"""Tiny client for the rates service."""
import json
import urllib.request

BASE_URL = "https://rates.staging.example.invalid/v2"


def get_rate(base, quote, opener=urllib.request.urlopen):
    """Return the exchange rate for base -> quote as a float."""
    url = "{}/rate?base={}&quote={}".format(BASE_URL, base, quote)
    with opener(url, timeout=5) as response:
        return json.load(response)["rate"]
