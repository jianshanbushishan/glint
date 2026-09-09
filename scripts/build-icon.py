"""Package the generated PNG as a Windows multi-resolution ICO (requires Pillow)."""
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
for source_name, output_name in [("glint-icon.png", "glint.ico"), ("glint-paused.png", "glint-paused.ico")]:
    with Image.open(root / "assets" / source_name) as source:
        source.convert("RGBA").save(
            root / "assets" / output_name, sizes=[(size, size) for size in sizes]
        )
