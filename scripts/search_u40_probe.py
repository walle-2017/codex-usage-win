"""Test production-like U40 changes and the proven Mark40 with transparent U80."""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageFilter
from search_small_probe import U40, NAMES, pixel_info
from search_content_probe import transform as content_transform

VARIANTS = ("Inset40", "Bold40", "Round80")


def transform(path, variant, index):
    # Round80 changes the 80px baseline only: keep original RGBA artwork.
    if variant == "Round80" and path.name in NAMES:
        with Image.open(path) as image:
            im = image.convert("RGBA")
        im.save(path, optimize=True)
        info = pixel_info(path)
        return info, info
    baseline, result = content_transform(
        path, "GrayMark" if variant == "Round80" and path.name == U40 else "Reencode", index)
    if path.name == U40 and variant != "Round80":
        with Image.open(path) as image:
            rgba = image.convert("RGBA")
        alpha = rgba.getchannel("A")
        black = Image.new("RGBA", rgba.size, (0, 0, 0, 255))
        black.alpha_composite(rgba)
        rgb = black.convert("RGB")
        if variant == "Inset40":
            # Shrink white artwork by 15%; keep original rounded black silhouette.
            inner = rgb.resize((34, 34), Image.Resampling.LANCZOS)
            result_rgb = Image.new("RGB", (40, 40), (0, 0, 0))
            result_rgb.paste(inner, (3, 3))
        elif variant == "Bold40":
            # Strengthen light strokes without changing the alpha silhouette.
            expanded = rgb.filter(ImageFilter.MaxFilter(3))
            result_rgb = Image.blend(rgb, expanded, 0.35)
        else:
            raise ValueError(variant)
        result_rgb.putalpha(alpha)
        result_rgb.save(path, optimize=True)
        result = pixel_info(path)
    if (baseline["pixels"] != result["pixels"]) != (path.name == U40):
        raise ValueError(f"Unexpected pixel difference: {path}")
    if baseline["alpha_pixels"] != result["alpha_pixels"] or baseline["size"] != result["size"]:
        raise ValueError(f"Unexpected alpha or size change: {path}")
    return baseline, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    parser.add_argument("variant", choices=VARIANTS)
    parser.add_argument("report", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    files = sorted(args.assets.glob("*.png"))
    if len(files) != 66 or not (NAMES | {U40}).issubset({p.name for p in files}):
        raise ValueError("Unexpected asset inventory")
    if args.verify:
        report = json.loads(args.report.read_text(encoding="utf-8"))
        actual = {p.name: pixel_info(p) for p in files}
        if report["variant"] != args.variant or report["after"] != actual:
            raise ValueError("Packaged artwork differs from report")
        for name in NAMES:
            expected = ("RGBA", [0, 255]) if args.variant == "Round80" else ("RGB", [255, 255])
            if (actual[name]["mode"], actual[name]["alpha"]) != expected or actual[name]["size"] != [80, 80]:
                raise ValueError(f"Invalid 80px resource: {name}")
        print("PASS: all 66 packaged images verified")
        return
    before = {p.name: pixel_info(p) for p in files}
    baseline, after = {}, {}
    for i, p in enumerate(files):
        baseline[p.name], after[p.name] = transform(p, args.variant, i)
        if p.name not in NAMES or args.variant == "Round80":
            if baseline[p.name]["pixels"] != before[p.name]["pixels"]:
                raise ValueError(f"Unexpected baseline pixel change: {p}")
        if p.name not in NAMES and after[p.name]["alpha_pixels"] != before[p.name]["alpha_pixels"]:
            raise ValueError(f"Unexpected alpha change: {p}")
    changed = {name for name in baseline if baseline[name]["pixels"] != after[name]["pixels"]}
    if changed != {U40}:
        raise ValueError(f"Unexpected modified assets: {changed}")
    args.report.write_text(json.dumps({"variant": args.variant, "changed": sorted(changed),
        "before": before, "baseline": baseline, "after": after}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
