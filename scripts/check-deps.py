#!/usr/bin/env python3
"""Check the dependency one-way law across the repo's Cargo face.

Usage:
  python3 scripts/check-deps.py [repo-root]   # default: the repo above scripts/

Exit code: 0 = clean, 1 = at least one violation, 2 = a Cargo.toml that
could not be parsed (an unparsed manifest is not a clean one).

Background: the open/closed partition (mcd 3.design/arch/220-open-closed-partition.md,
section 4.1 construction face) mechanizes two judgments for every L1 (open)
repository. This script is repo-agnostic: each L1 repo carries a copy and runs
it in CI and the local gate.

Judgment A - dependency one-way: the Cargo dependency face (workspace members,
path dependencies, crate names) must not point at an L2/L3 repository. A path
dependency whose target resolves into a closed repo directory, or a dependency
named after one, turns the open repo inside-out: the open/closed boundary is
defined by the dependency graph, so any backwards edge is a violation no
matter how innocuous the use.

Judgment B - no second channel: network database driver crates are banned in
the gate domain (postgres family and mysql family). The gateway (mcsrv) is the
only credential holder; a driver crate in an L1 repo is the seed of a direct
connection that would bypass metering and entitlement. No driver means no
direct connection is possible - prevention is cheaper than retrofit rerouting.

Outside the gate: local embedded own-format storage is not a database driver
(e.g. mcc's src/db/cmie file format); mcsrv itself is exempt (the exit gateway
body, legitimately the sole sqlx holder); mcide is L3 and rides the P2 batch,
not this gate.
"""
import re
import sys
import tomllib
from pathlib import Path

# L2/L3 repositories (220 section 2 layer registry). A dependency whose
# resolved path crosses one of these directory names, or whose crate name is
# one of these, is a backwards edge out of the open face.
CLOSED_REPOS = {"mchub", "mce", "mcm", "mcs", "mct", "mcsrv", "mcide"}

# Network database driver crates (judgment B). Local embedded engines
# (rusqlite and friends) are deliberately absent: the law bans channels to
# the asset stores, not local storage.
DB_DRIVERS = {
    # postgres family
    "sqlx",
    "postgres",
    "tokio-postgres",
    "postgres-types",
    "postgres-protocol",
    "postgres-derive",
    "deadpool-postgres",
    "diesel",
    # mysql family
    "mysql",
    "mysql_async",
    "mysql_common",
}

SKIP_DIRS = {".git", "target", "build", "node_modules", "_obsolete"}

# Dependency tables live at the top level or under target."cfg(...)"; any
# table whose last segment reads "dependencies" is in scope.
DEP_TABLE_SUFFIX = "dependencies"


def manifest_files(root: Path):
    for dirpath, dirnames, filenames in sorted(Path(root).walk()):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        if "Cargo.toml" in filenames:
            yield dirpath / "Cargo.toml"


def dep_tables(table, prefix=()):
    for name, value in table.items():
        path = prefix + (name,)
        if isinstance(value, dict):
            if name == DEP_TABLE_SUFFIX:
                yield path, value
            else:
                yield from dep_tables(value, path)


def find_line(text, needle):
    """Best-effort line number for a finding: the first line mentioning it."""
    for lineno, line in enumerate(text.splitlines(), 1):
        if re.search(rf"^\s*{re.escape(needle)}\s*=", line, re.MULTILINE) or needle in line:
            return lineno
    return 0


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    findings = []
    parsed = 0
    for manifest in manifest_files(root):
        rel = manifest.relative_to(root)
        text = manifest.read_text(encoding="utf-8")
        try:
            data = tomllib.loads(text)
        except tomllib.TOMLDecodeError as exc:
            print(f"{rel}: unparsable Cargo.toml ({exc})")
            return 2
        parsed += 1

        # workspace members ride the same judgment A (a member path is a
        # dependency edge by another spelling)
        members = data.get("workspace", {}).get("members", []) or []
        for member in members if isinstance(members, list) else []:
            if any(part in CLOSED_REPOS for part in Path(member).parts):
                findings.append(f"{rel}: workspace member '{member}' points into a closed repo (judgment A)")

        for table_path, deps in dep_tables(data):
            scope = ".".join(table_path)
            for crate, spec in deps.items():
                if crate in DB_DRIVERS:
                    findings.append(
                        f"{rel}: network DB driver crate '{crate}' ({scope}) (judgment B)"
                    )
                if crate in CLOSED_REPOS:
                    findings.append(
                        f"{rel}: dependency '{crate}' is named after a closed repo ({scope}) (judgment A)"
                    )
                if isinstance(spec, dict) and "path" in spec:
                    target = (manifest.parent / spec["path"]).resolve()
                    closed = [part for part in target.parts if part in CLOSED_REPOS]
                    if closed:
                        findings.append(
                            f"{rel}: path dependency '{crate}' -> {spec['path']} enters closed repo '{closed[0]}' ({scope}) (judgment A)"
                        )

    if parsed == 0:
        print("check-deps: no Cargo.toml found - nothing to gate")
        return 0
    for finding in findings:
        print(finding)
    print(f"check-deps: {parsed} manifest(s) scanned, {len(findings)} violation(s)")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
