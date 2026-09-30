#!/usr/bin/env python3
"""Reject font-dependent icon glyphs in handwritten UI and Rust string literals."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
STRINGS = re.compile(r'"(?:\\.|[^"\\])*"')
RANGES = ((0x2190, 0x2BFF), (0xE000, 0xF8FF), (0x1F000, 0x1FAFF),
          (0xF0000, 0xFFFFD), (0x100000, 0x10FFFD))
EXTRA = set('×÷«»‹›⌃⌄')


def violations(source, slint=False):
    for match in STRINGS.finditer(source):
        value = match.group()[1:-1]
        decoded = re.sub(r'\\u(?:\{([0-9a-fA-F]{1,6})\}|([0-9a-fA-F]{4}))',
                         lambda m: chr(int(m.group(1) or m.group(2), 16)), value)
        bad = [c for c in decoded if c in EXTRA or any(lo <= ord(c) <= hi for lo, hi in RANGES)]
        # Catch ASCII substitutes in literal labels, without rejecting keyboard handling.
        prefix = source[max(0, match.start()-40):match.start()]
        ascii_icon = slint and re.search(r'\btext\s*:\s*$', prefix) and value.strip() in {
            '+', '-', 'x', 'X', '<', '>', '^', 'v', '|', '*', '/', '\\\\'}
        if bad or ascii_icon:
            yield source.count('\n', 0, match.start())+1, value


def main():
    paths = sorted((ROOT/'ui').rglob('*.slint')) + sorted((ROOT/'src').rglob('*.rs'))
    failures = []
    for path in paths:
        for line, value in violations(path.read_text(), path.suffix == '.slint'):
            failures.append(f'{path.relative_to(ROOT)}:{line}: use an SVG icon, not {value!r}')
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print('UI icon check passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
