#!/usr/bin/env python3
"""
ddddocr captcha helper.

Reads raw PNG bytes from stdin, runs ddddocr in arithmetic mode
(`set_ranges(7)`) which returns the integer answer directly (e.g. "11"
for `5+6=?`), and prints it to stdout. Any failure → empty output.

Usage:
    cat captcha.png | python ddddocr_helper.py
"""
import os
import sys

# Suppress noisy ddddocr init
os.environ.setdefault("DDDDOCR_DISABLE_LOG", "1")

try:
    import ddddocr  # type: ignore
except ImportError as e:
    sys.stderr.write(f"import error: {e}\n")
    sys.stdout.write("")
    sys.exit(1)


def main() -> int:
    raw = sys.stdin.buffer.read()
    if not raw:
        return 0

    try:
        ocr = ddddocr.DdddOcr(show_ad=False)
        ocr.set_ranges(7)  # 7 = arithmetic / math captcha preset
        result = ocr.classification(raw)
        # ddddocr returns the integer answer string for arithmetic mode.
        # Be defensive: strip whitespace, drop non-digit leading sign.
        result = (result or "").strip()
        # Keep only digits and optional leading minus — never anything else.
        cleaned = "".join(
            ch for ch in result
            if ch.isdigit() or (ch == "-" and not result[: result.index(ch) + 1].count("-"))
        )
        sys.stdout.write(cleaned)
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"ocr error: {e}\n")
        sys.stdout.write("")

    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())