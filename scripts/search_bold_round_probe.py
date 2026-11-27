"""Combine the exact successful Bold40 transformation with original transparent 80px resources."""
import argparse
import json
from pathlib import Path

from PIL import Image
from search_small_probe import U40, NAMES, pixel_info
from search_u40_probe import transform as bold_transform

VARIANTS = ("BoldRound",)


def transform(path, variant, index):
    if variant != "BoldRound":
        raise ValueError(variant)
    if path.name in NAMES:
        # Preserve the original transparent pixels, including the alpha mask.
        with Image.open(path) as image:
            im = image.convert("RGBA")
        im.save(path, optimize=True)
        info = pixel_info(path)
        return info, info
    # Invoke the exact round-thirteen implementation for the other 63 assets.
    return bold_transform(path, "Bold40", index)


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
            expected = ("RGBA", [0, 255]) if args.variant == "BoldRound" else ("RGB", [255, 255])
            if (actual[name]["mode"], actual[name]["alpha"]) != expected or actual[name]["size"] != [80, 80]:
                raise ValueError(f"Invalid 80px resource: {name}")
        print("PASS: all 66 packaged images verified")
        return
    before = {p.name: pixel_info(p) for p in files}
    baseline, after = {}, {}
    for i, p in enumerate(files):
        baseline[p.name], after[p.name] = transform(p, args.variant, i)
        if p.name not in NAMES or args.variant == "BoldRound":
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
