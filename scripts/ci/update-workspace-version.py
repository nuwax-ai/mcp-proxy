#!/usr/bin/env python3
"""Update workspace.package and internal workspace dependency versions in Cargo.toml."""

import re
import sys

def internal_path_deps(content: str) -> list[str]:
    """Discover workspace.dependencies entries that carry a `path` (internal crates).

    Hardcoding a list breaks whenever a new crate joins the workspace (the
    mcp-proxy-args miss made prerelease versions unresolvable on CI). Entries
    without a `version` key (e.g. voice-toolkit) are skipped — nothing to rewrite.
    """
    deps: list[str] = []
    in_ws_deps = False
    for line in content.splitlines():
        stripped = line.strip()
        if stripped == "[workspace.dependencies]":
            in_ws_deps = True
            continue
        if stripped.startswith("[") and in_ws_deps:
            break
        if not in_ws_deps or "path =" not in stripped:
            continue
        m = re.match(r'^([A-Za-z0-9_-]+)\s*=\s*\{', line)
        if m and "version =" in stripped:
            deps.append(m.group(1))
    return deps


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

    for crate in internal_path_deps(content):
        content = re.sub(
            rf'({re.escape(crate)} = \{{ version = )"[^"]*"',
            rf'\1"{version}"',
            content,
        )

    with open("Cargo.toml", "w", encoding="utf-8") as f:
        f.write(content)

    print(f"==> Updated workspace version to: {version}")
    for crate in internal_path_deps(content):
        match = re.search(rf'{re.escape(crate)} = \{{ version = "([^"]*)"', content)
        if match:
            print(f"    {crate}: {match.group(1)}")


if __name__ == "__main__":
    main()
