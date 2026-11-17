def apply_discount(cents, percent):
    """Price in cents after `percent` off, rounded to the nearest cent
    (a half cent rounds up)."""
    return (cents * (100 - percent) + 50) // 100
