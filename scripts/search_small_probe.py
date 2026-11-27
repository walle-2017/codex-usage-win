"""Bisect the successful MarkOthers probe using the two small assets observed in Procmon."""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image
from search_hybrid_probe import NAMES, inspect
from search_content_probe import transform as content_transform

VARIANTS = ("Mark40", "Mark32", "MarkBoth", "MarkRest")
U40 = "Square44x44Logo.targetsize-40_altform-unplated.png"
T32 = "Square44x44Logo.targetsize-32.png"
OBSERVED = {U40, T32}


def pixel_info(path):
    info = inspect(path)
    with Image.open(path) as im:
        rgba = im.convert("RGBA")
        info["pixels"] = hashlib.sha256(rgba.tobytes()).hexdigest()
        info["alpha_pixels"] = hashlib.sha256(rgba.getchannel("A").tobytes()).hexdigest()
        info["mode"] = im.mode
    return info


def selected(name, variant):
    if variant == "Mark40":
        return name == U40
    if variant == "Mark32":
        return name == T32
    if variant == "MarkBoth":
        return name in OBSERVED
    if variant == "MarkRest":
        return name not in NAMES and name not in OBSERVED
    raise ValueError(f"Unknown variant: {variant}")


def transform(path, variant, index):
    mark_this = selected(path.name, variant)
    baseline, result = content_transform(
        path, "GrayMark" if mark_this else "Reencode", index)
    if (result["pixels"] != baseline["pixels"]) != mark_this:
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
    if len(files) != 66 or not (NAMES | OBSERVED).issubset({p.name for p in files}):
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
    changed = {name for name in baseline if baseline[name]["pixels"] != after[name]["pixels"]}
    expected = {name for name in baseline if selected(name, args.variant)}
    expected_count = {"Mark40": 1, "Mark32": 1, "MarkBoth": 2, "MarkRest": 61}[args.variant]
    if changed != expected or len(changed) != expected_count:
        raise ValueError(f"Unexpected modified assets: {changed}")
    args.report.write_text(json.dumps({"variant": args.variant, "changed": sorted(changed), "before": before,
        "baseline": baseline, "after": after}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
