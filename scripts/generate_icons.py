"""Generate and verify all application icons from the canonical documentation PNG."""
import argparse
import math
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

ROOT = Path(__file__).resolve().parents[1]
DOC_ICON_DIR = ROOT / "docs" / "assets"
ICON_DIR = ROOT / "src" / "icons"
SOURCE = DOC_ICON_DIR / "icon.png"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
TARGET_SIZES = (16, 20, 24, 30, 32, 36, 40, 44, 48, 60, 64, 72, 80, 96, 256)


def source_image():
    with Image.open(SOURCE) as image:
        result = image.convert("RGBA")
    if result.size != (256, 256) or result.getchannel("A").getextrema()[0] != 0:
        raise ValueError("Canonical icon must be 256x256 with transparency")
    return result


def scaled(source, size):
    return source.resize((size, size), Image.Resampling.LANCZOS)


def store_listing_icon(source, size=300):
    # Store adds its own rounded frame; avoid a second transparent margin.
    background = Image.new("RGBA", (size, size), (0, 0, 0, 255))
    background.alpha_composite(scaled(source, size))
    return background.convert("RGB")


def verify_store_icon(actual, source, size=300, label="Store listing PNG", tolerance=3):
    if actual.size != (size, size):
        raise ValueError(f"{label}: incorrect dimensions")
    rgba = actual.convert("RGBA")
    if rgba.getchannel("A").getextrema() != (255, 255):
        raise ValueError(f"{label}: background must be fully opaque")
    corners = [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)]
    if any(rgba.getpixel(point) != (0, 0, 0, 255) for point in corners):
        raise ValueError(f"{label}: background must extend to all corners")
    means = ImageStat.Stat(ImageChops.difference(actual.convert("RGB"), store_listing_icon(source, size))).mean
    if max(means) > tolerance:
        raise ValueError(f"{label}: artwork differs from canonical icon: {means}")


def verify_image(actual, expected, label, tolerance=3):
    actual = actual.convert("RGBA")
    if actual.size != expected.size:
        raise ValueError(f"{label}: unexpected size {actual.size}")
    if actual.getchannel("A").getextrema()[0] != 0:
        raise ValueError(f"{label}: missing transparent pixels")
    # Compare visible pixels and alpha, ignoring RGB hidden by transparency.
    # Allow small resampling/rounding differences across encoders.
    difference = ImageChops.difference(actual.convert("RGBa"), expected.convert("RGBa"))
    means = ImageStat.Stat(difference).mean
    if max(means) > tolerance:
        raise ValueError(f"{label}: artwork differs from canonical icon: {means}")


def asset_sizes():
    result = {}
    for scale in (100, 125, 150, 200, 250, 300, 400):
        for logo, base in (("Square44x44Logo", 44), ("Square150x150Logo", 150), ("StoreLogo", 50)):
            result[f"{logo}.scale-{scale}.png"] = math.ceil(base * scale / 100)
    for size in TARGET_SIZES:
        for suffix in ("", "_altform-unplated", "_altform-lightunplated"):
            result[f"Square44x44Logo.targetsize-{size}{suffix}.png"] = size
    return result


def verify(source, msix_assets):
    with Image.open(ICON_DIR / "256x256.png") as image:
        verify_image(image, source, "runtime PNG")
    with Image.open(DOC_ICON_DIR / "store-icon.png") as image:
        verify_store_icon(image, source)
    with Image.open(ICON_DIR / "icon.ico") as image:
        if image.ico.sizes() != {(n, n) for n in ICO_SIZES}:
            raise ValueError("ICO frame sizes do not match")
        for size in ICO_SIZES:
            verify_image(image.ico.getimage((size, size)), scaled(source, size), f"ICO {size}")
    if msix_assets:
        # GDI+ uses bicubic rather than Lanczos; use an independent comparison
        # against the same artwork with a modest tolerance for small icons.
        for name, size in asset_sizes().items():
            with Image.open(msix_assets / name) as image:
                if name.startswith("StoreLogo."):
                    verify_store_icon(image, source, size, name, tolerance=12)
                else:
                    verify_image(image, scaled(source, size), name, tolerance=12)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify without modifying files")
    parser.add_argument("--msix-assets", type=Path, help="Also verify extracted MSIX Assets directory")
    args = parser.parse_args()
    source = source_image()
    if not args.check:
        ICON_DIR.mkdir(parents=True, exist_ok=True)
        source.save(ICON_DIR / "256x256.png", optimize=True)
        source.save(ICON_DIR / "icon.ico", format="ICO",
                    sizes=[(n, n) for n in ICO_SIZES])
        store_listing_icon(source).save(DOC_ICON_DIR / "store-icon.png", optimize=True)
    verify(source, args.msix_assets)
    print("PASS: application icons match canonical artwork and preserve transparency")


if __name__ == "__main__":
    main()
