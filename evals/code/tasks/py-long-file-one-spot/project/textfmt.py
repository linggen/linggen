"""Text formatting helpers for the status page.

Every function takes plain values and returns a string. Nothing here reads
the clock, the locale or the network, so the output is the same everywhere.
Sections: sizes and counts, times, numbers, names and ids, lists, and the
unit converters at the bottom.
"""

# ---------------------------------------------------------------------------
# Sizes and counts
# ---------------------------------------------------------------------------

_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"]


def format_size(num_bytes):
    """Bytes as text with 1024-based units: 0 -> '0 B', 1536 -> '1.5 KB'.

    Whole numbers show no decimal point ('2 KB', not '2.0 KB'); anything else
    shows one decimal. Negative sizes are shown with a leading minus.
    """
    sign = "-" if num_bytes < 0 else ""
    value = float(abs(num_bytes))
    unit = 0
    while value >= 1024 and unit < len(_SIZE_UNITS) - 1:
        value /= 1024.0
        unit += 1
    if value == int(value):
        text = "{}".format(int(value))
    else:
        text = "{:.1f}".format(value)
    return "{}{} {}".format(sign, text, _SIZE_UNITS[unit])


def format_count(n):
    """Large counts shortened: 999 -> '999', 1200 -> '1.2k', 3400000 -> '3.4M'.

    Exactly 1000 is '1k'. The decimal is dropped when it is zero.
    """
    if n < 1000:
        return str(n)
    for limit, suffix in ((1000000000, "B"), (1000000, "M"), (1000, "k")):
        if n >= limit:
            value = n / float(limit)
            text = "{:.1f}".format(value)
            if text.endswith(".0"):
                text = text[:-2]
            return text + suffix
    return str(n)


def format_thousands(n):
    """12345 -> '12,345'. Negative numbers keep the minus in front."""
    sign = "-" if n < 0 else ""
    digits = str(abs(int(n)))
    groups = []
    while len(digits) > 3:
        groups.insert(0, digits[-3:])
        digits = digits[:-3]
    groups.insert(0, digits)
    return sign + ",".join(groups)


def plural(n, word):
    """1 file, 2 files, 0 files. Words ending in y become -ies ('city' -> 'cities')."""
    if n == 1:
        return "{} {}".format(n, word)
    if word.endswith("y") and word[-2:-1] not in ("a", "e", "i", "o", "u"):
        return "{} {}ies".format(n, word[:-1])
    if word.endswith(("s", "x", "ch", "sh")):
        return "{} {}es".format(n, word)
    return "{} {}s".format(n, word)


# ---------------------------------------------------------------------------
# Times
# ---------------------------------------------------------------------------

def format_duration(seconds):
    """A length of time as text.

    Under a minute: '45s'. Under an hour: minutes and seconds, '1m 30s';
    whole minutes drop the seconds, '5m'. Seconds are rounded down.
    """
    seconds = int(seconds)
    if seconds < 0:
        return "-" + format_duration(-seconds)
    minutes, secs = divmod(seconds, 60)
    if minutes == 0:
        return "{}s".format(secs)
    if secs == 0:
        return "{}m".format(minutes)
    return "{}m {}s".format(minutes, secs)


def format_clock(seconds):
    """Seconds into a day as 'HH:MM', wrapping after 24 hours: 3600 -> '01:00'."""
    seconds = int(seconds) % 86400
    return "{:02d}:{:02d}".format(seconds // 3600, (seconds % 3600) // 60)


def format_ago(seconds):
    """How long ago: 'just now' under 45 s, then the largest whole unit."""
    seconds = int(seconds)
    if seconds < 45:
        return "just now"
    if seconds < 3600:
        return "{}m ago".format(max(1, seconds // 60))
    if seconds < 86400:
        return "{}h ago".format(seconds // 3600)
    days = seconds // 86400
    if days < 14:
        return "{}d ago".format(days)
    return "{}w ago".format(days // 7)


def format_date(year, month, day):
    """(2026, 3, 9) -> '9 Mar 2026'. Months are 1-12."""
    names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    if not 1 <= month <= 12:
        raise ValueError("month out of range: {}".format(month))
    return "{} {} {}".format(day, names[month - 1], year)


def format_range_of_days(first_day, last_day):
    """Day numbers of one month: (3, 3) -> '3', (3, 7) -> '3-7'."""
    if first_day == last_day:
        return str(first_day)
    return "{}-{}".format(first_day, last_day)


# ---------------------------------------------------------------------------
# Numbers
# ---------------------------------------------------------------------------

def format_percent(part, whole):
    """part/whole as a whole-number percentage: (1, 3) -> '33%'. A zero whole is '-'."""
    if whole == 0:
        return "-"
    return "{}%".format(int(round(100.0 * part / whole)))


def format_ratio(part, whole):
    """'3 of 4'. When the whole is zero, just '0'."""
    if whole == 0:
        return "0"
    return "{} of {}".format(part, whole)


def format_ordinal(n):
    """1 -> '1st', 2 -> '2nd', 11 -> '11th', 22 -> '22nd'."""
    if 10 <= n % 100 <= 20:
        suffix = "th"
    else:
        suffix = {1: "st", 2: "nd", 3: "rd"}.get(n % 10, "th")
    return "{}{}".format(n, suffix)


def format_signed(n):
    """Always show the sign: 5 -> '+5', -5 -> '-5', 0 -> '0'."""
    if n > 0:
        return "+{}".format(n)
    return str(n)


def format_fixed(value, places):
    """Fixed decimals without a negative zero: (-0.001, 2) -> '0.00'."""
    text = "{:.{}f}".format(value, places)
    if text.startswith("-") and float(text) == 0:
        text = text[1:]
    return text


def format_money(cents):
    """Cents as dollars: 1234 -> '$12.34', -5 -> '-$0.05'."""
    sign = "-" if cents < 0 else ""
    cents = abs(int(cents))
    return "{}${}.{:02d}".format(sign, cents // 100, cents % 100)


def format_rate(count, seconds):
    """Events per second with one decimal: (30, 10) -> '3.0/s'. Zero seconds is 'n/a'."""
    if seconds == 0:
        return "n/a"
    return "{:.1f}/s".format(count / float(seconds))


def format_roman(n):
    """1-3999 as Roman numerals; anything else is returned as a plain number."""
    if not 1 <= n <= 3999:
        return str(n)
    pairs = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
             (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")]
    out = []
    for value, letters in pairs:
        while n >= value:
            out.append(letters)
            n -= value
    return "".join(out)


# ---------------------------------------------------------------------------
# Names and ids
# ---------------------------------------------------------------------------

def pad_id(n, width=6):
    """7 -> 'ID-000007'. Wider numbers are not cut."""
    return "ID-" + str(n).zfill(width)


def initials(name):
    """'ada king lovelace' -> 'AKL'. Extra spaces are ignored."""
    return "".join(part[0].upper() for part in name.split())


def title_name(name):
    """'mc donald' -> 'Mc Donald'; hyphenated names keep their hyphen: 'anne-marie' -> 'Anne-Marie'."""
    words = []
    for word in name.split():
        words.append("-".join(piece[:1].upper() + piece[1:].lower() for piece in word.split("-")))
    return " ".join(words)


def truncate_middle(text, width):
    """Shorten to `width` characters with '...' in the middle; short text is unchanged.

    The left side gets the extra character when the room is odd.
    """
    if len(text) <= width:
        return text
    if width <= 3:
        return text[:width]
    room = width - 3
    left = (room + 1) // 2
    right = room // 2
    return text[:left] + "..." + (text[len(text) - right:] if right else "")


def mask_email(address):
    """'ann@example.com' -> 'a**@example.com'. One-letter names are fully shown."""
    name, _, domain = address.partition("@")
    if len(name) <= 1:
        return address
    return name[0] + "*" * (len(name) - 1) + "@" + domain


def mask_card(number):
    """Digits only, last four shown: '4111 1111 1111 1111' -> '************1111'."""
    digits = "".join(ch for ch in number if ch.isdigit())
    if len(digits) <= 4:
        return digits
    return "*" * (len(digits) - 4) + digits[-4:]


def format_phone(digits):
    """10 digits as '(555) 123-4567'; 11 digits starting with 1 drop the 1; other lengths unchanged."""
    only = "".join(ch for ch in digits if ch.isdigit())
    if len(only) == 11 and only.startswith("1"):
        only = only[1:]
    if len(only) != 10:
        return digits
    return "({}) {}-{}".format(only[:3], only[3:6], only[6:])


def format_version(major, minor=0, patch=0, pre=None):
    """(1, 2) -> 'v1.2.0'; with pre='rc1' -> 'v1.2.0-rc1'."""
    text = "v{}.{}.{}".format(major, minor, patch)
    if pre:
        text += "-" + pre
    return text


# ---------------------------------------------------------------------------
# Lists
# ---------------------------------------------------------------------------

def format_list(items):
    """['a'] -> 'a'; ['a', 'b'] -> 'a and b'; ['a', 'b', 'c'] -> 'a, b, and c'."""
    items = list(items)
    if not items:
        return ""
    if len(items) == 1:
        return items[0]
    if len(items) == 2:
        return "{} and {}".format(items[0], items[1])
    return "{}, and {}".format(", ".join(items[:-1]), items[-1])


def format_top(items, limit):
    """The first `limit` items, then 'and N more': (['a','b','c','d'], 2) -> 'a, b and 2 more'."""
    items = list(items)
    if len(items) <= limit:
        return ", ".join(items)
    rest = len(items) - limit
    return "{} and {} more".format(", ".join(items[:limit]), rest)


def format_bullets(items, marker="-"):
    """One '- item' line per item; an empty list is the empty string."""
    return "\n".join("{} {}".format(marker, item) for item in items)


def format_table_row(cells, widths):
    """Cells padded on the right to their widths and joined by ' | '; long cells are cut."""
    out = []
    for cell, width in zip(cells, widths):
        text = str(cell)
        out.append(text[:width].ljust(width))
    return " | ".join(out)


def format_bool(value):
    """True -> 'yes', False -> 'no', None -> 'unknown'."""
    if value is None:
        return "unknown"
    return "yes" if value else "no"


def format_status(code):
    """HTTP-ish status codes grouped by their first digit; unknown codes are 'unknown'."""
    groups = {2: "ok", 3: "moved", 4: "client error", 5: "server error"}
    return groups.get(int(code) // 100, "unknown")

# ---------------------------------------------------------------------------
# Status lines and small charts
# ---------------------------------------------------------------------------

def format_uptime(seconds):
    """'up ' and the duration: 5400 -> 'up 1h 30m'. Negative uptime is 'up 0s'."""
    if seconds < 0:
        return "up 0s"
    return "up " + format_duration(seconds)


def format_eta(seconds):
    """'less than a minute' under 60 s, otherwise 'about ' and the duration."""
    if seconds < 60:
        return "less than a minute"
    return "about " + format_duration(seconds)


def format_bar(fraction, width=10):
    """A text progress bar: (0.5, 10) -> '[#####.....]'. The fraction is clamped to 0..1."""
    fraction = min(1.0, max(0.0, fraction))
    filled = int(fraction * width)
    return "[" + "#" * filled + "." * (width - filled) + "]"


def format_delta(old, new):
    """Change between two numbers: (10, 15) -> '+5 (+50%)', (10, 5) -> '-5 (-50%)'.

    When old is zero there is no percentage: (0, 5) -> '+5'. No change is '0'.
    """
    diff = new - old
    if diff == 0:
        return "0"
    sign = "+" if diff > 0 else "-"
    if old == 0:
        return "{}{}".format(sign, abs(diff))
    pct = int(round(abs(diff) * 100.0 / abs(old)))
    return "{}{} ({}{}%)".format(sign, abs(diff), sign, pct)


def format_histogram_row(label, count, scale=1):
    """'label  ####  4' with one '#' per `scale` events, rounded up; zero events draw no bar."""
    bars = 0 if count <= 0 else -(-count // scale)
    return "{:<8}{:<12}{}".format(label, "#" * bars, count)


def format_hex(n, width=2):
    """255 -> '0xff'. Padded with zeros to `width` digits; negatives are not supported."""
    if n < 0:
        raise ValueError("negative number")
    return "0x" + format(n, "x").zfill(width)


def format_bits(n, width=8):
    """5 -> '00000101'. Wider numbers are not cut."""
    if n < 0:
        raise ValueError("negative number")
    return format(n, "b").zfill(width)


def format_weekday(index):
    """0 -> 'Mon' ... 6 -> 'Sun'. The index wraps: 7 -> 'Mon', -1 -> 'Sun'."""
    names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    return names[index % 7]


def format_quarter(month):
    """Month 1-12 -> 'Q1'..'Q4'."""
    if not 1 <= month <= 12:
        raise ValueError("month out of range: {}".format(month))
    return "Q{}".format((month - 1) // 3 + 1)


def format_coords(lat, lon):
    """(51.5, -0.12) -> '51.50N 0.12W'. Two decimals; the hemisphere letter replaces the sign."""
    ns = "N" if lat >= 0 else "S"
    ew = "E" if lon >= 0 else "W"
    return "{:.2f}{} {:.2f}{}".format(abs(lat), ns, abs(lon), ew)


def format_range(low, high):
    """(1, 5) -> '1-5'. Equal ends give one number; reversed ends are swapped."""
    if low > high:
        low, high = high, low
    if low == high:
        return str(low)
    return "{}-{}".format(low, high)


def format_headline(text, width=40):
    """Upper-cased, cut at a word boundary to `width` characters, '...' when cut."""
    text = " ".join(text.split()).upper()
    if len(text) <= width:
        return text
    cut = text[: width - 3]
    if " " in cut:
        cut = cut[: cut.rindex(" ")]
    return cut.rstrip() + "..."


def format_key_value(pairs):
    """[('a', 1), ('bb', 22)] -> 'a   1\nbb  22': keys padded to the longest, values right as given."""
    pairs = list(pairs)
    if not pairs:
        return ""
    width = max(len(str(k)) for k, _ in pairs)
    return "\n".join("{}  {}".format(str(k).ljust(width), v) for k, v in pairs)


def format_json_like(mapping):
    """{'b': 1, 'a': 'x'} -> '{a: x, b: 1}': keys sorted, no quotes. Empty is '{}'."""
    if not mapping:
        return "{}"
    return "{" + ", ".join("{}: {}".format(k, mapping[k]) for k in sorted(mapping)) + "}"


# ---------------------------------------------------------------------------
# Unit converters
#
# Each takes a number and returns text with the unit, rounded to the places
# its docstring names. A converted value that rounds to zero is shown as
# plain 0 (no negative zero). Not for calculation: use the numbers, not this text.
# ---------------------------------------------------------------------------

def _shown(value, places, unit):
    text = "{:.{}f}".format(value, places)
    if text.startswith("-") and float(text) == 0:
        text = text[1:]
    return "{} {}".format(text, unit)

def km_to_miles(value):
    """Kilometres to miles, 1 decimal place: '12.3 mi'."""
    return _shown(value * 0.621371, 1, "mi")

def miles_to_km(value):
    """Miles to kilometres, 1 decimal place: '12.3 km'."""
    return _shown(value * 1.609344, 1, "km")

def m_to_feet(value):
    """Metres to feet, 0 decimal places: '12 ft'."""
    return _shown(value * 3.28084, 0, "ft")

def feet_to_m(value):
    """Feet to metres, 1 decimal place: '12.3 m'."""
    return _shown(value * 0.3048, 1, "m")

def cm_to_inches(value):
    """Centimetres to inches, 1 decimal place: '12.3 in'."""
    return _shown(value * 0.393701, 1, "in")

def inches_to_cm(value):
    """Inches to centimetres, 1 decimal place: '12.3 cm'."""
    return _shown(value * 2.54, 1, "cm")

def kg_to_lb(value):
    """Kilograms to pounds, 1 decimal place: '12.3 lb'."""
    return _shown(value * 2.204623, 1, "lb")

def lb_to_kg(value):
    """Pounds to kilograms, 1 decimal place: '12.3 kg'."""
    return _shown(value * 0.453592, 1, "kg")

def g_to_oz(value):
    """Grams to ounces, 1 decimal place: '12.3 oz'."""
    return _shown(value * 0.035274, 1, "oz")

def oz_to_g(value):
    """Ounces to grams, 0 decimal places: '12 g'."""
    return _shown(value * 28.349523, 0, "g")

def l_to_gal(value):
    """Litres to US gallons, 2 decimal places: '12.34 gal'."""
    return _shown(value * 0.264172, 2, "gal")

def gal_to_l(value):
    """US gallons to litres, 2 decimal places: '12.34 L'."""
    return _shown(value * 3.785412, 2, "L")

def ml_to_floz(value):
    """Millilitres to US fluid ounces, 1 decimal place: '12.3 fl oz'."""
    return _shown(value * 0.033814, 1, "fl oz")

def c_to_f(value):
    """Celsius to Fahrenheit, 0 decimal places: '12 F'."""
    return _shown(value * 1.8 + 32, 0, "F")

def f_to_c(value):
    """Fahrenheit to Celsius, 0 decimal places: '12 C'."""
    return _shown((value - 32) * 5.0 / 9.0, 0, "C")

def kmh_to_mph(value):
    """Kilometres per hour to miles per hour, 0 decimal places: '12 mph'."""
    return _shown(value * 0.621371, 0, "mph")

def mph_to_kmh(value):
    """Miles per hour to kilometres per hour, 0 decimal places: '12 km/h'."""
    return _shown(value * 1.609344, 0, "km/h")

def ms_to_kmh(value):
    """Metres per second to kilometres per hour, 1 decimal place: '12.3 km/h'."""
    return _shown(value * 3.6, 1, "km/h")

def hp_to_kw(value):
    """Horsepower to kilowatts, 1 decimal place: '12.3 kW'."""
    return _shown(value * 0.7457, 1, "kW")

def kw_to_hp(value):
    """Kilowatts to horsepower, 1 decimal place: '12.3 hp'."""
    return _shown(value * 1.341022, 1, "hp")

def psi_to_kpa(value):
    """Pounds per square inch to kilopascals, 1 decimal place: '12.3 kPa'."""
    return _shown(value * 6.894757, 1, "kPa")

def kpa_to_psi(value):
    """Kilopascals to pounds per square inch, 1 decimal place: '12.3 psi'."""
    return _shown(value * 0.145038, 1, "psi")

def acres_to_ha(value):
    """Acres to hectares, 2 decimal places: '12.34 ha'."""
    return _shown(value * 0.404686, 2, "ha")

def ha_to_acres(value):
    """Hectares to acres, 2 decimal places: '12.34 ac'."""
    return _shown(value * 2.471054, 2, "ac")


def sqm_to_sqft(value):
    """Square metres to square feet, 1 decimal place: '12.3 sq ft'."""
    return _shown(value * 10.76391, 1, "sq ft")


def sqft_to_sqm(value):
    """Square feet to square metres, 1 decimal place: '12.3 sq m'."""
    return _shown(value * 0.092903, 1, "sq m")


def mb_to_gb(value):
    """Megabytes to gigabytes (decimal), 2 decimal places: '12.34 GB'."""
    return _shown(value * 0.001, 2, "GB")


def gb_to_mb(value):
    """Gigabytes to megabytes (decimal), 0 decimal places: '12 MB'."""
    return _shown(value * 1000.0, 0, "MB")


def mbps_to_mbs(value):
    """Megabits per second to megabytes per second, 2 decimal places: '12.34 MB/s'."""
    return _shown(value * 0.125, 2, "MB/s")


def mbs_to_mbps(value):
    """Megabytes per second to megabits per second, 1 decimal place: '12.3 Mb/s'."""
    return _shown(value * 8.0, 1, "Mb/s")


def kcal_to_kj(value):
    """Kilocalories to kilojoules, 0 decimal places: '12 kJ'."""
    return _shown(value * 4.184, 0, "kJ")


def kj_to_kcal(value):
    """Kilojoules to kilocalories, 0 decimal places: '12 kcal'."""
    return _shown(value * 0.239006, 0, "kcal")


def kwh_to_mj(value):
    """Kilowatt-hours to megajoules, 1 decimal place: '12.3 MJ'."""
    return _shown(value * 3.6, 1, "MJ")


def mj_to_kwh(value):
    """Megajoules to kilowatt-hours, 2 decimal places: '12.34 kWh'."""
    return _shown(value * 0.277778, 2, "kWh")


def min_to_hours(value):
    """Minutes to hours, 2 decimal places: '12.34 h'."""
    return _shown(value * 0.016666666666666666, 2, "h")


def hours_to_min(value):
    """Hours to minutes, 0 decimal places: '12 min'."""
    return _shown(value * 60.0, 0, "min")


def days_to_hours(value):
    """Days to hours, 0 decimal places: '12 h'."""
    return _shown(value * 24.0, 0, "h")


def hours_to_days(value):
    """Hours to days, 1 decimal place: '12.3 d'."""
    return _shown(value * 0.041666666666666664, 1, "d")


def deg_to_rad(value):
    """Degrees to radians, 3 decimal places: '12.34 rad'."""
    return _shown(value * 0.0174533, 3, "rad")


def rad_to_deg(value):
    """Radians to degrees, 1 decimal place: '12.3 deg'."""
    return _shown(value * 57.29578, 1, "deg")


def knots_to_kmh(value):
    """Knots to kilometres per hour, 0 decimal places: '12 km/h'."""
    return _shown(value * 1.852, 0, "km/h")


def kmh_to_knots(value):
    """Kilometres per hour to knots, 0 decimal places: '12 kn'."""
    return _shown(value * 0.539957, 0, "kn")


def bar_to_psi(value):
    """Bar to pounds per square inch, 1 decimal place: '12.3 psi'."""
    return _shown(value * 14.50377, 1, "psi")


def psi_to_bar(value):
    """Pounds per square inch to bar, 2 decimal places: '12.34 bar'."""
    return _shown(value * 0.068948, 2, "bar")


def mpg_to_l100(value):
    """Miles per gallon to litres per 100 km (a reciprocal), 1 decimal place: '12.3 L/100km'."""
    if value <= 0:
        return "n/a"
    return _shown(235.215 / value, 1, "L/100km")


def l100_to_mpg(value):
    """Litres per 100 km to miles per gallon (a reciprocal), 1 decimal place: '12.3 mpg'."""
    if value <= 0:
        return "n/a"
    return _shown(235.215 / value, 1, "mpg")
