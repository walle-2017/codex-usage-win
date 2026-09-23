from __future__ import annotations

import json
import pathlib
import urllib.request

from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "fonts"
LICENSES = OUT / "licenses"
TMP = ROOT / "target" / "font-assets"

URLS = {
    "inter": "https://raw.githubusercontent.com/rsms/inter/master/docs/font-files/InterVariable.ttf",
    "jetbrains": "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/fonts/variable/JetBrainsMono%5Bwght%5D.ttf",
    "noto": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
    "inter_license": "https://raw.githubusercontent.com/rsms/inter/master/LICENSE.txt",
    "jetbrains_license": "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/OFL.txt",
    "noto_license": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/OFL.txt",
}

TEXT_SUFFIXES = {".rs", ".md", ".toml", ".ps1", ".yml", ".yaml", ".json"}


def download(url: str, path: pathlib.Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    urllib.request.urlretrieve(url, path)


def collect_ui_glyphs() -> str:
    chars: set[str] = set()
    for base in [ROOT / "src", ROOT]:
        for path in base.rglob("*"):
            if not path.is_file() or path.suffix.lower() not in TEXT_SUFFIXES:
                continue
            if any(part in {"target", ".git", "assets"} for part in path.parts):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for ch in text:
                if ord(ch) >= 0x80:
                    chars.add(ch)

    # Explicit UI symbols which may be generated dynamically or used by popup widgets.
    chars.update("✓×→›▲▼–—•…：，。、（）【】“”")
    return "".join(sorted(chars))


def subset_noto(source: pathlib.Path, output: pathlib.Path, glyphs: str) -> None:
    variable = TTFont(source)
    static = instantiateVariableFont(variable, {"wght": 400}, inplace=False)

    options = Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_legacy = True
    options.name_languages = ["*"]
    options.retain_gids = False
    options.notdef_glyph = True
    options.notdef_outline = True
    options.recalc_average_width = True
    options.recalc_bounds = True

    subsetter = Subsetter(options=options)
    subsetter.populate(text=glyphs)
    subsetter.subset(static)
    output.parent.mkdir(parents=True, exist_ok=True)
    static.save(output)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    LICENSES.mkdir(parents=True, exist_ok=True)
    TMP.mkdir(parents=True, exist_ok=True)

    inter = OUT / "InterVariable.ttf"
    jetbrains = OUT / "JetBrainsMono.ttf"
    noto_full = TMP / "NotoSansSC-full.ttf"
    noto_subset = OUT / "NotoSansSC-UI.ttf"

    download(URLS["inter"], inter)
    download(URLS["jetbrains"], jetbrains)
    download(URLS["noto"], noto_full)

    glyphs = collect_ui_glyphs()
    (TMP / "noto-ui-glyphs.txt").write_text(glyphs, encoding="utf-8")
    subset_noto(noto_full, noto_subset, glyphs)

    download(URLS["inter_license"], LICENSES / "Inter-LICENSE.txt")
    download(URLS["jetbrains_license"], LICENSES / "JetBrainsMono-OFL.txt")
    download(URLS["noto_license"], LICENSES / "NotoSansSC-OFL.txt")

    metadata = {
        "sources": URLS,
        "glyph_count": len(glyphs),
        "files": {
            path.name: path.stat().st_size
            for path in [inter, jetbrains, noto_subset]
        },
    }
    (OUT / "metadata.json").write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    print(json.dumps(metadata, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
