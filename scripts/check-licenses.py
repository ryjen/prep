#!/usr/bin/env python3
import json
import re
import subprocess
import sys

ALLOWED = {"MIT", "Apache-2.0", "BSD-3-Clause", "Unicode-3.0"}
ALLOWED_EXCEPTIONS: set[str] = set()
TOKEN_RE = re.compile(r"\(|\)|AND|OR|WITH|[A-Za-z0-9][A-Za-z0-9.+-]*")


class LicenseExpressionError(ValueError):
    pass


class Parser:
    def __init__(self, expression: str) -> None:
        self.expression = expression
        self.tokens = TOKEN_RE.findall(expression)
        compact_expression = re.sub(r"\s+", "", expression)
        if "".join(self.tokens) != compact_expression:
            raise LicenseExpressionError(f"unsupported SPDX syntax: {expression}")
        self.position = 0

    def parse(self) -> bool:
        allowed = self.parse_or()
        if self.position != len(self.tokens):
            raise LicenseExpressionError(
                f"unexpected token {self.tokens[self.position]!r} in {self.expression}"
            )
        return allowed

    def parse_or(self) -> bool:
        allowed = self.parse_and()
        while self.peek() == "OR":
            self.take("OR")
            alternative = self.parse_and()
            allowed = allowed or alternative
        return allowed

    def parse_and(self) -> bool:
        allowed = self.parse_with()
        while self.peek() == "AND":
            self.take("AND")
            requirement = self.parse_with()
            allowed = allowed and requirement
        return allowed

    def parse_with(self) -> bool:
        allowed = self.parse_primary()
        if self.peek() == "WITH":
            self.take("WITH")
            exception = self.take_identifier()
            allowed = allowed and exception in ALLOWED_EXCEPTIONS
        return allowed

    def parse_primary(self) -> bool:
        if self.peek() == "(":
            self.take("(")
            allowed = self.parse_or()
            self.take(")")
            return allowed
        return self.take_identifier() in ALLOWED

    def peek(self) -> str | None:
        if self.position >= len(self.tokens):
            return None
        return self.tokens[self.position]

    def take(self, expected: str) -> None:
        actual = self.peek()
        if actual != expected:
            raise LicenseExpressionError(
                f"expected {expected!r}, got {actual!r} in {self.expression}"
            )
        self.position += 1

    def take_identifier(self) -> str:
        token = self.peek()
        if token is None or token in {"(", ")", "AND", "OR", "WITH"}:
            raise LicenseExpressionError(
                f"expected license identifier, got {token!r} in {self.expression}"
            )
        self.position += 1
        return token


def expression_is_allowed(expression: str) -> bool:
    return Parser(expression).parse()


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
            failures.append(
                f"{package['name']} {package['version']}: missing SPDX license expression"
            )
            continue
        try:
            allowed = expression_is_allowed(expression)
        except LicenseExpressionError as error:
            failures.append(f"{package['name']} {package['version']}: {error}")
            continue
        if not allowed:
            failures.append(
                f"{package['name']} {package['version']}: no policy-approved choice in {expression}"
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
