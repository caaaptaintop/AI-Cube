#!/usr/bin/env python3
"""Export Pleat's editable SVG with librsvg; assemble ICO/ICNS without new Python deps."""
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
source = root / "assets/brand/pleat-icon.svg"
renderer = shutil.which("rsvg-convert")
if not renderer:
    sys.exit("rsvg-convert is required (macOS: brew install librsvg).")


def render(size, output):
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([renderer, "--width", str(size), "--height", str(size),
                    "--output", str(output), str(source)], check=True)


render(1024, root / "assets/app-icon.png")
shutil.copyfile(root / "assets/app-icon.png", root / "assets/macos/app-icon.png")
for scale, suffix in [(1, ""), (2, "@2x")]:
    subprocess.run([renderer, "--width", str(821 * scale), "--height", str(479 * scale),
                    "--output", str(root / f"assets/dmg/background{suffix}.png"),
                    str(root / "assets/brand/installer-background.svg")], check=True)
with tempfile.TemporaryDirectory(prefix="pleat-icons-") as directory:
    work = Path(directory)
    sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
    for size in sizes:
        render(size, work / f"{size}.png")
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    entries, payload = [], bytearray()
    offset = 6 + 16 * len(ico_sizes)
    for size in ico_sizes:
        png = (work / f"{size}.png").read_bytes()
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                   1, 32, len(png), offset))
        payload.extend(png)
        offset += len(png)
    (root / "assets/app-icon.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(entries)) + b"".join(entries) + payload)
    if sys.platform == "darwin":
        iconset = work / "Pleat.iconset"
        iconset.mkdir()
        for size in [16, 32, 128, 256, 512]:
            for scale in [1, 2]:
                suffix = "@2x" if scale == 2 else ""
                shutil.copyfile(work / f"{size * scale}.png",
                                iconset / f"icon_{size}x{size}{suffix}.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o",
                        str(root / "assets/macos/app-icon.icns")], check=True)
print("Exported Pleat PNG (1024), ICO (16–256), and macOS ICNS when available.")
