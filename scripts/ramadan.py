#!/usr/bin/env python3
"""Builds crates/rechenkern/src/data/ramadan.txt: the first day of each Ramadan and its length.

Usage: scripts/ramadan.py > crates/rechenkern/src/data/ramadan.txt

- hijridate (https://github.com/dralshehri/hijridate, MIT): the Umm al-Qura calendar of
  Saudi Arabia, 1343 to 1500 AH (1924 to 2077). In nixpkgs as python3Packages.hijridate.

The Islamic calendar follows the moon, so its months come from a table, as Chinese New
Year does. The length tells where Ramadan ends, the day before 1 Shawwal.
"""

from hijridate import Hijri
from hijridate.ummalqura import HIJRI_RANGE

first, last = HIJRI_RANGE[0][0], HIJRI_RANGE[1][0]
print("# The first day of Ramadan and its length in days, from the Umm al-Qura calendar.")
print("# Built by scripts/ramadan.py with hijridate (MIT).")
for year in range(first, last + 1):
    start = Hijri(year, 9, 1).to_gregorian()
    end = Hijri(year, 10, 1).to_gregorian()
    # A month has 29 or 30 days; the table's first year doesn't hold together.
    if (end - start).days not in (29, 30):
        continue
    print(f"{start.isoformat()} {(end - start).days}")
