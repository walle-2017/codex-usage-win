"""Mark each package logo with its source qualifier; diagnostic branch only."""
import argparse
import csv
import hashlib
import re
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

GROUPS = [
    (24, "#D52A2A", "red"),
    (36, "#235CE0", "blue"),
    (48, "#168C45", "green"),
    (80, "#E8B600", "yellow"),
    (96, "#8A39D0", "purple"),
    (128, "#0098AC", "cyan"),
    (256, "#DC388C", "pink"),
]
SCALE = {
    "Square44x44Logo": ("#E46B13", "orange", "A"),
    "Square150x150Logo": ("#80512C", "brown", "B"),
    "StoreLogo": ("#657685", "slate", "S"),
}


def classify(name):
    target = re.fullmatch(r"Square44x44Logo\.targetsize-(\d+)(?:_altform-(unplated|lightunplated))?\.png", name)
    if target:
        n = int(target[1])
        color, group = next((c, g) for limit, c, g in GROUPS if n <= limit)
        code = {None: "T", "unplated": "U", "lightunplated": "L"}[target[2]]
        return color, group, f"{code}\n{n}"
    scale = re.fullmatch(r"(Square44x44Logo|Square150x150Logo|StoreLogo)\.scale-(\d+)\.png", name)
    if scale:
        color, group, prefix = SCALE[scale[1]]
        return color, group, f"{prefix}\n{scale[2]}"
    raise ValueError(f"Unexpected asset: {name}")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mark(path):
    with Image.open(path) as image:
        original = image.convert("RGBA")
    size = original.width
    color, group, label = classify(path.name)
    # Draw solid colors and large qualifier labels while preserving exact alpha.
    canvas = Image.new("RGBA", original.size, color)
    draw = ImageDraw.Draw(canvas)
    font_size = max(6, int(size * 0.36))
    while font_size > 5:
        font = ImageFont.load_default(size=font_size)
        bounds = draw.multiline_textbbox((0, 0), label, font=font, spacing=0, align="center")
        if bounds[2]-bounds[0] <= size * .85 and bounds[3]-bounds[1] <= size * .85:
            break
        font_size -= 1
    bounds = draw.multiline_textbbox((0, 0), label, font=font, spacing=0, align="center")
    xy = ((size-(bounds[2]-bounds[0]))/2-bounds[0], (size-(bounds[3]-bounds[1]))/2-bounds[1])
    draw.multiline_text(xy, label, font=font, fill="white", spacing=0, align="center",
                        stroke_width=max(0, size//100), stroke_fill="black")
    canvas.putalpha(original.getchannel("A"))
    canvas.save(path, optimize=True)
    return {"file": path.name, "size": size, "color": color, "group": group,
            "label": label.replace("\n", " "), "sha256": sha(path)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("assets", type=Path)
    parser.add_argument("map", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    files = sorted(args.assets.glob("*.png"))
    if len(files) != 66:
        raise ValueError(f"Expected 66 icon resources, found {len(files)}")
    if args.verify:
        with args.map.open(newline="", encoding="utf-8") as f:
            rows = list(csv.DictReader(f))
        if len(rows) != 66 or {r["file"] for r in rows} != {p.name for p in files}:
            raise ValueError("Probe map does not match package assets")
        for row in rows:
            path = args.assets / row["file"]
            if sha(path) != row["sha256"]:
                raise ValueError(f"Probe asset changed: {path}")
            with Image.open(path) as image:
                if image.size != (int(row["size"]),)*2:
                    raise ValueError(f"Probe dimensions changed: {path}")
        print("PASS: all 66 diagnostic assets match their map")
    else:
        rows = [mark(p) for p in files]
        with args.map.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


if __name__ == "__main__":
    main()
