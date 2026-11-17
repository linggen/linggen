"""Quotes: a price with a percentage discount taken off."""

from . import money


def discount_for(price_cents, discount_bp):
    return money.apply_rate(price_cents, discount_bp)


def quote(price_cents, discount_bp=0):
    discount = discount_for(price_cents, discount_bp)
    return {"price": price_cents, "discount": discount, "net": price_cents - discount}
