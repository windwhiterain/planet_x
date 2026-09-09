#!/usr/bin/env python3
"""Generate PLANET_X_CONFIG variants by safely patching a RON config.

USAGE:
    python patch_config.py OUT.ron field:value [field:value ...]

Scans the *whole file text* for a line matching `<field>: <number>` (or the
multi-line `field: <number>,` inside a block) and replaces just the number.
We patch by exact string-replace of `field: OLD,` so we never rewrite the
surrounding structure. Fields we patch must already exist in the base config.
"""
import sys
import re


def patch(base_text: str, field: str, value: str) -> str:
    # Match `field:` followed by an optional comma/newline-tolerant number.
    # We want to hit e.g. `    hegemon_power: 0.30,` but NOT `power_city_weight:`.
    # We require the field token to be a standalone word.
    pat = re.compile(rf"(\b{re.escape(field)}\s*:\s*)(-?[0-9]+(?:\.[0-9]+)?)")
    m = pat.search(base_text)
    if not m:
        raise SystemExit(f"field {field!r} not found in base config")
    new = m.group(1) + value
    return base_text[: m.start()] + new + base_text[m.end():]


def main() -> None:
    out = sys.argv[1]
    pairs = []
    for arg in sys.argv[2:]:
        k, v = arg.split(":", 1)
        pairs.append((k, v))
    base = sys.argv[2 + len(pairs) * 0]
    # base config is the second arg's ... no; argv[1]=out, rest=field:value.
    base_text = open("config/game.ron", encoding="utf-8").read()
    for k, v in pairs:
        base_text = patch(base_text, k, v)
    with open(out, "w", encoding="utf-8") as fh:
        fh.write(base_text)
    print(f"wrote {out}: {pairs}")


if __name__ == "__main__":
    main()
