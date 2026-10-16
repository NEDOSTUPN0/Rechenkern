#!/usr/bin/env python3
"""Builds crates/rechenkern/src/data/cpi.txt: the US consumer price index, for inflation.

Usage: scripts/cpi.py cu.data.1.AllItems > crates/rechenkern/src/data/cpi.txt

- https://download.bls.gov/pub/time.series/cu/cu.data.1.AllItems
  (U.S. Bureau of Labor Statistics, public domain; BLS refuses requests without a
  browser-like User-Agent)

Series CUUR0000SA0: CPI for All Urban Consumers (CPI-U), U.S. city average, all items,
not seasonally adjusted, 1982-84 = 100. The file keeps the annual average of every year
and the months of the years that have none yet, so the latest month is the price level now.
"""

import sys

SERIES = "CUUR0000SA0"

annual, monthly = {}, {}
with open(sys.argv[1]) as data:
    for line in data:
        fields = [f.strip() for f in line.split("\t")]
        if len(fields) < 4 or fields[0] != SERIES:
            continue
        year, period, value = int(fields[1]), fields[2], fields[3]
        if period == "M13":
            annual[year] = value
        elif period.startswith("M"):
            monthly[(year, int(period[1:]))] = value

print(f"# US CPI-U ({SERIES}), 1982-84 = 100, from the U.S. Bureau of Labor Statistics (public domain).")
print("# Built by scripts/cpi.py. `year value` is an annual average, `year-month value` a month.")
for year in sorted(annual):
    print(f"{year} {annual[year]}")
for year, month in sorted(monthly):
    if year not in annual:
        print(f"{year}-{month:02} {monthly[(year, month)]}")
