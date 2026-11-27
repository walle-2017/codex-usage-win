"""Search detail A/B probe: keep all artwork; flatten only 80px variants in B."""
import argparse
import hashlib
import json
from pathlib import Path
from PIL import Image

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
    after = {p.name: inspect(p) for p in files}
    changed = {name for name in before if before[name] != after[name]}
    if changed != (NAMES if args.variant == "Opaque" else set()):
        raise ValueError(f"Unexpected diagnostic asset changes: {changed}")
    args.report.write_text(json.dumps({"variant": args.variant, "before": before,
        "after": after}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
