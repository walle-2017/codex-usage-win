"""Isolate the 80px group versus other assets, and test marker-free geometry."""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image
from search_hybrid_probe import NAMES, inspect
from search_content_probe import transform as content_transform

VARIANTS = ("Mark80", "MarkOthers", "Fit80")


def pixel_info(path):
    info = inspect(path)
    with Image.open(path) as im:
        rgba = im.convert("RGBA")
        info["pixels"] = hashlib.sha256(rgba.tobytes()).hexdigest()
        info["alpha_pixels"] = hashlib.sha256(rgba.getchannel("A").tobytes()).hexdigest()
        info["mode"] = im.mode
    return info


def base(path):
    with Image.open(path) as im:
        rgba = im.convert("RGBA")
    if path.name in NAMES:
        bg = Image.new("RGBA", rgba.size, (0, 0, 0, 255))
        bg.alpha_composite(rgba)
        return bg.convert("RGB")
    return rgba


def save(image, path):
    image.save(path, optimize=True)


def transform(path, variant, index):
    is_80 = path.name in NAMES
    mark_this = (variant == "Mark80" and is_80) or (
        variant == "MarkOthers" and not is_80)
    # Reuse the exact round-ten GrayMark and Reencode implementations.
    baseline, result = content_transform(
        path, "GrayMark" if mark_this else "Reencode", index)
    if variant == "Fit80" and is_80:
        with Image.open(path) as im:
            original = im.convert("RGB")
        # Enlarge the same artwork 12.5%, retaining the 80px black canvas.
        enlarged = original.resize((90, 90), Image.Resampling.LANCZOS)
        save(enlarged.crop((5, 5, 85, 85)), path)
        result = pixel_info(path)
    should_change = mark_this or (variant == "Fit80" and is_80)
    if (result["pixels"] != baseline["pixels"]) != should_change:
        raise ValueError(f"Unexpected pixel difference: {path}")
    if result["size"] != baseline["size"] or result["alpha_pixels"] != baseline["alpha_pixels"]:
        raise ValueError(f"Unexpected size or alpha change: {path}")
    return baseline, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    parser.add_argument("variant", choices=VARIANTS)
    parser.add_argument("report", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    files = sorted(args.assets.glob("*.png"))
    if len(files) != 66 or not NAMES.issubset({p.name for p in files}):
        raise ValueError("Unexpected asset inventory")
    if args.verify:
        expected = json.loads(args.report.read_text(encoding="utf-8"))
        actual = {p.name: pixel_info(p) for p in files}
        if expected["variant"] != args.variant or expected["after"] != actual:
            raise ValueError("Packaged artwork differs from report")
        for name in NAMES:
            if actual[name]["size"] != [80, 80] or actual[name]["alpha"] != [255, 255] or actual[name]["mode"] != "RGB":
                raise ValueError(f"Invalid 80px opaque variant: {name}")
        print("PASS: all 66 packaged images verified")
        return
    before = {p.name: pixel_info(p) for p in files}
    baseline, after = {}, {}
    for i, p in enumerate(files):
        baseline[p.name], after[p.name] = transform(p, args.variant, i)
        if p.name not in NAMES and baseline[p.name]["pixels"] != before[p.name]["pixels"]:
            raise ValueError(f"Unexpected baseline pixel change: {p}")
        if p.name not in NAMES and after[p.name]["alpha_pixels"] != before[p.name]["alpha_pixels"]:
            raise ValueError(f"Unexpected alpha change: {p}")
    args.report.write_text(json.dumps({"variant": args.variant, "before": before,
        "baseline": baseline, "after": after}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
