#!/usr/bin/env python3
"""Builds crates/rechenkern/src/data/rates.txt: exchange rates for the first run without network.

Usage: scripts/rates.py usd.min.json > crates/rechenkern/src/data/rates.txt

- https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.min.json
  (currency API by Fawaz Ahmed, CC0)

Keeps the currencies of crates/rechenkern/src/currency.rs, as units per US dollar.
"""

import json
import re
import sys
from pathlib import Path

table = Path(__file__).resolve().parent.parent / "crates/rechenkern/src/currency.rs"
codes = re.findall(r'(?:fiat|crypto)\("([A-Z0-9]+)"', table.read_text())

with open(sys.argv[1]) as data:
    api = json.load(data)

print(f"# Exchange rates on {api['date']}, units per US dollar, from the currency API by Fawaz Ahmed (CC0).")
print("# Built by scripts/rates.py; used until fresh rates are known.")
print(f"date {api['date']}")
for code in codes:
    rate = api["usd"].get(code.lower())
    if code != "USD" and rate:
        print(f"{code} {json.dumps(rate)}")
