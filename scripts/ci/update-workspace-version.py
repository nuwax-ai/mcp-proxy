#!/usr/bin/env python3
"""Update workspace.package and internal workspace dependency versions in Cargo.toml."""

import re
import sys

# systemd-installer 独立版本，不在此列表
INTERNAL_CRATES = [
    "mcp-proxy",
    "mcp-common",
    "mcp-sse-proxy",
    "mcp-streamable-proxy",
    "oss-client",
    "run_code_rmcp",
]


def main() -> None:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <version>", file=sys.stderr)
        sys.exit(1)

    version = sys.argv[1]
    with open("Cargo.toml", "r", encoding="utf-8") as f:
        content = f.read()

    content = re.sub(
        r'^version = "[^"]*"',
        f'version = "{version}"',
        content,
        count=1,
        flags=re.MULTILINE,
    )

    for crate in INTERNAL_CRATES:
        content = re.sub(
            rf'({re.escape(crate)} = \{{ version = )"[^"]*"',
            rf'\1"{version}"',
            content,
        )

    with open("Cargo.toml", "w", encoding="utf-8") as f:
        f.write(content)

    print(f"==> Updated workspace version to: {version}")
    for crate in INTERNAL_CRATES:
        match = re.search(rf'{re.escape(crate)} = \{{ version = "([^"]*)"', content)
        if match:
            print(f"    {crate}: {match.group(1)}")


if __name__ == "__main__":
    main()
