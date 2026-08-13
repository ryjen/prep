#!/usr/bin/env python3
import json
import re
import subprocess
import sys

ALLOWED = {"MIT", "Apache-2.0", "BSD-3-Clause", "Unicode-3.0"}
OPERATORS = {"AND", "OR", "WITH"}


def license_ids(expression: str) -> set[str]:
    tokens = re.findall(r"[A-Za-z0-9.+-]+", expression)
    return {token for token in tokens if token not in OPERATORS}


def main() -> int:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    workspace_members = set(metadata["workspace_members"])
    failures: list[str] = []

    for package in metadata["packages"]:
        if package["id"] in workspace_members:
            continue
        expression = package.get("license")
        if not expression:
            failures.append(f"{package['name']} {package['version']}: missing SPDX license expression")
            continue
        disallowed = license_ids(expression) - ALLOWED
        if disallowed:
            failures.append(
                f"{package['name']} {package['version']}: {expression} contains "
                f"disallowed/unknown licenses {sorted(disallowed)}"
            )

    if failures:
        print("Dependency license policy failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("Dependency licenses satisfy the allowlist policy.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
