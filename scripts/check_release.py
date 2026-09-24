#!/usr/bin/env python3
"""Validate release version metadata before tagging or publishing."""

from __future__ import annotations

import argparse
import json
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def load_versions() -> dict[str, str]:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    app = json.loads((ROOT / "webos-app/appinfo.json").read_text(encoding="utf-8"))
    package = next(
        item for item in lock["package"] if item["name"] == cargo["package"]["name"]
    )
    return {
        "Cargo.toml": cargo["package"]["version"],
        "Cargo.lock": package["version"],
        "webos-app/appinfo.json": app["version"],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="Expected annotated tag, for example v0.4.1")
    args = parser.parse_args()

    versions = load_versions()
    unique = set(versions.values())
    if len(unique) != 1:
        raise SystemExit(
            "release versions differ: "
            + ", ".join(f"{path}={version}" for path, version in versions.items())
        )

    version = unique.pop()
    if not SEMVER.fullmatch(version):
        raise SystemExit(f"invalid semantic version: {version}")

    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    if f"## [{version}]" not in changelog:
        raise SystemExit(f"CHANGELOG.md has no release heading for {version}")

    if args.tag and args.tag != f"v{version}":
        raise SystemExit(f"tag {args.tag} does not match release version v{version}")

    print(f"release metadata agrees on {version}")


if __name__ == "__main__":
    main()
