"""Packs Yume 2kki's window skins, as forest-orb cuts them from each system graphic, into one atlas.

Usage: python3 tools/windowskins.py ~/dev/forest-orb
Writes assets/windowskins.png and assets/windowskins.txt, one skin name per line in atlas order.
Keep CELL and the per-cell layout in sync with `src/ui/skin.rs`.
"""

import re
import sys
from pathlib import Path

from PIL import Image

CELL = (64, 48)
COLUMNS = 11
# Where each of forest-orb's files lands inside a skin's cell.
LAYOUT = {
    "containerbg": (0, 0),
    "border": (32, 0),
    "font1": (0, 32),
    "fontshadow": (16, 32),
    "font2": (48, 32),
    "arrowup": (32, 32),
    "arrowdown": (32, 40),
}


def skin_names(forest_orb: Path) -> list[str]:
    script = (forest_orb / "system.js").read_text()
    listing = re.search(r"'2kki': \[(.*?)\]", script, re.S)
    return re.findall(r"'([^']+)'", listing.group(1))


def main() -> None:
    forest_orb = Path(sys.argv[1]).expanduser()
    names = skin_names(forest_orb)
    rows = -(-len(names) // COLUMNS)
    atlas = Image.new("RGBA", (COLUMNS * CELL[0], rows * CELL[1]))
    for index, name in enumerate(names):
        cell = (index % COLUMNS * CELL[0], index // COLUMNS * CELL[1])
        for part, (x, y) in LAYOUT.items():
            piece = Image.open(forest_orb / "images/ui/2kki" / name / f"{part}.png").convert("RGBA")
            atlas.paste(piece, (cell[0] + x, cell[1] + y))
    assets = Path(__file__).resolve().parent.parent / "assets"
    atlas.save(assets / "windowskins.png", optimize=True)
    (assets / "windowskins.txt").write_text("\n".join(names) + "\n")


if __name__ == "__main__":
    main()
