#!/usr/bin/env python3
"""Report worldData entries whose `cell` differs between two data.json files."""
import json
import sys


def worlds(path):
    with open(path) as f:
        return {w["id"]: w for w in json.load(f)["worldData"]}


def main(old_path, new_path):
    old, new = worlds(old_path), worlds(new_path)

    for wid in sorted(old.keys() - new.keys()):
        print(f"-  {wid:4} {old[wid]['title']} (cell {old[wid]['cell']})")
    for wid in sorted(new.keys() - old.keys()):
        print(f"+  {wid:4} {new[wid]['title']} (cell {new[wid]['cell']})")

    changed = 0
    for wid in sorted(old.keys() & new.keys()):
        a, b = old[wid]["cell"], new[wid]["cell"]
        if a != b:
            changed += 1
            print(f"~  {wid:4} {new[wid]['title']}: {a} -> {b}")

    print(f"\n{changed} changed, {len(new.keys() - old.keys())} added, "
          f"{len(old.keys() - new.keys())} removed", file=sys.stderr)
    return 1 if changed or old.keys() != new.keys() else 0


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: cell_diff.py OLD.json NEW.json")
    sys.exit(main(sys.argv[1], sys.argv[2]))
