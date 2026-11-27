"""Preserve real artwork and mark each source qualifier; flatten 80px variants as in Opaque probe."""
import argparse
import hashlib
import colorsys
import json
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
from mark_search_probe import classify

NAMES = {
    f"Square44x44Logo.targetsize-80{suffix}.png"
    for suffix in ("", "_altform-unplated", "_altform-lightunplated")
}


def inspect(path):
    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        return {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "size": list(rgba.size),
            "alpha": list(rgba.getchannel("A").getextrema()),
        }


def mark(path, index):
    with Image.open(path) as image:
        original_mode = image.mode
        canvas = image.convert("RGBA")
    alpha = canvas.getchannel("A")
    size = canvas.width
    _, _, label = classify(path.name)
    code = label.replace("\n", "")
    # Keep the exact source alpha and the original artwork below the top marker.
    special = {"U80": "#F0C000", "U40": "#00BA68", "T32": "#3979FF"}
    hue = (index * 0.61803398875 + 0.13) % 1.0
    rgb = tuple(round(v * 255) for v in colorsys.hsv_to_rgb(hue, .78, .88))
    color = special.get(code, "#%02X%02X%02X" % rgb)
    draw = ImageDraw.Draw(canvas)
    left, top = round(size * .18), round(size * .10)
    right, bottom = round(size * .82)-1, round(size * .34)-1
    draw.rectangle((left, top, right, bottom), fill=color)
    if size >= 32:
        font_size = max(5, round(size * .18))
        while font_size >= 5:
            font = ImageFont.load_default(size=font_size)
            b = draw.textbbox((0, 0), code, font=font)
            if b[2]-b[0] <= right-left-1 and b[3]-b[1] <= bottom-top:
                xy = ((left+right-b[2]+b[0])/2-b[0], (top+bottom-b[3]+b[1])/2-b[1])
                ink = "black" if code == "U80" else "white"
                draw.text(xy, code, font=font, fill=ink)
                break
            font_size -= 1
    canvas.putalpha(alpha)
    if original_mode == "RGB":
        canvas = canvas.convert("RGB")
    canvas.save(path, optimize=True)
    return {"code": code, "color": color}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    parser.add_argument("variant", choices=("Transparent", "Opaque"))
    parser.add_argument("report", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    files = sorted(args.assets.glob("*.png"))
    if len(files) != 66 or not NAMES.issubset({p.name for p in files}):
        raise ValueError("Unexpected asset inventory")
    if args.verify:
        expected = json.loads(args.report.read_text(encoding="utf-8"))
        actual = {p.name: inspect(p) for p in files}
        if expected["variant"] != args.variant or expected["after"] != actual:
            raise ValueError("Packaged asset hashes or properties differ from diagnostic report")
        for name in NAMES:
            if actual[name]["size"] != [80, 80] or actual[name]["alpha"] != (
                [255, 255] if args.variant == "Opaque" else [0, 255]
            ):
                raise ValueError(f"Invalid 80px variant: {name}")
        print("PASS: all diagnostic artwork and alpha properties verified")
        return
    before = {p.name: inspect(p) for p in files}
    if args.variant == "Opaque":
        for name in NAMES:
            path = args.assets / name
            with Image.open(path) as image:
                original = image.convert("RGBA")
            background = Image.new("RGBA", original.size, (0, 0, 0, 255))
            background.alpha_composite(original)
            background.convert("RGB").save(path, optimize=True)
    markers = {p.name: mark(p, i) for i, p in enumerate(files)}
    after = {p.name: inspect(p) for p in files}
    changed = {name for name in before if before[name] != after[name]}
    if changed != set(before):
        raise ValueError(f"Unexpected diagnostic asset changes: {changed}")
    for name in before:
        if before[name]["size"] != after[name]["size"]:
            raise ValueError(f"Changed dimensions: {name}")
        expected_alpha = [255, 255] if name in NAMES and args.variant == "Opaque" else before[name]["alpha"]
        if after[name]["alpha"] != expected_alpha:
            raise ValueError(f"Changed alpha range: {name}")
    args.report.write_text(json.dumps({"variant": args.variant, "before": before,
        "after": after, "markers": markers}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
