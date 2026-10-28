#!/usr/bin/env python3
"""Generate big Markdown benchmark files (same structure as the WordCraft baseline).

Each section is: a heading, a prose paragraph with bold / italic / inline code / a
link, a 2-item list, a Rust code block and a 2-column table with 2 rows (~300 bytes, so 1 MB is about
3 350 sections and 10 MB about 33 000), matching docs/architecture.md section 5.0.

Usage:  python3 -I scripts/gen-big.py OUT_DIR [--sizes 1m,10m]
Writes OUT_DIR/big1m.md and OUT_DIR/big10m.md. Output is deterministic.
"""

import argparse
import os
import sys

SECTION = """## Section {n}

This is paragraph {n} with **bold text**, *italic text*, `inline_code({n})` and a [link](https://example.com/{n}) to read.

- First item {n}
- Second item {n}

```rust
fn section_{n}() -> u32 {{
    {n} * 2
}}
```

| Key | Value |
|-----|-------|
| a{n} | {n} |
| b{n} | {n}0 |

"""

SIZES = {"1m": 1_000_000, "10m": 10_000_000}


def generate(path: str, target_bytes: int) -> int:
    sections = 0
    written = 0
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        header = "# Benchmark document\n\n"
        f.write(header)
        written += len(header)
        while written < target_bytes:
            sections += 1
            chunk = SECTION.format(n=sections)
            f.write(chunk)
            written += len(chunk.encode("utf-8"))
    return sections


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("out_dir")
    ap.add_argument("--sizes", default="1m,10m", help="comma list of: " + ",".join(SIZES))
    args = ap.parse_args()

    os.makedirs(args.out_dir, exist_ok=True)
    for key in args.sizes.split(","):
        key = key.strip().lower()
        if key not in SIZES:
            print(f"unknown size {key!r}; choose from {', '.join(SIZES)}", file=sys.stderr)
            return 2
        path = os.path.join(args.out_dir, f"big{key}.md")
        sections = generate(path, SIZES[key])
        print(f"{path}: {os.path.getsize(path)} bytes, {sections} sections")
    return 0


if __name__ == "__main__":
    sys.exit(main())
