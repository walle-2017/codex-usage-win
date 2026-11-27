"""Separate PNG re-encoding, marker geometry, and chroma on the Opaque baseline."""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageOps
from search_hybrid_probe import NAMES, mark, inspect

VARIANTS = ("Reencode", "GrayMark", "Tint")


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
    im = base(path)
    save(im, path)
    baseline = pixel_info(path)
    if variant == "GrayMark":
        # Use the proven hybrid marker geometry and text; remove chroma only
        # from the exact marker rectangle, leaving the remaining pixels intact.
        mark(path, index)
        with Image.open(path) as marked:
            rgba = marked.convert("RGBA")
        n = rgba.width
        box = (round(n * .18), round(n * .10),
               round(n * .82), round(n * .34))
        patch = rgba.crop(box)
        gray = ImageOps.grayscale(patch)
        rgba.paste(Image.merge("RGBA", (gray, gray, gray, patch.getchannel("A"))), box)
        save(rgba.convert(im.mode), path)
    elif variant == "Tint":
        # No geometry change; black remains black, white becomes #E6F2FF.
        rgba = im.convert("RGBA")
        r, g, b, a = rgba.split()
        result = Image.merge("RGBA", (
            r.point([round(i * 230 / 255) for i in range(256)]),
            g.point([round(i * 242 / 255) for i in range(256)]), b, a))
        save(result.convert(im.mode), path)
    result = pixel_info(path)
    if result["size"] != baseline["size"] or result["alpha_pixels"] != baseline["alpha_pixels"]:
        raise ValueError(f"Unexpected size or alpha change: {path}")
    if variant == "Reencode" and result["pixels"] != baseline["pixels"]:
        raise ValueError(f"Unexpected pixel change: {path}")
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
