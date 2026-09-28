from __future__ import annotations

import json
import pathlib
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "fonts"
LICENSES = OUT / "licenses"

URLS = {
    "jetbrains": "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/fonts/variable/JetBrainsMono%5Bwght%5D.ttf",
    "jetbrains_license": "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/OFL.txt",
}


def download(url: str, path: pathlib.Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    urllib.request.urlretrieve(url, path)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    LICENSES.mkdir(parents=True, exist_ok=True)

    jetbrains = OUT / "JetBrainsMono.ttf"
    download(URLS["jetbrains"], jetbrains)
    download(URLS["jetbrains_license"], LICENSES / "JetBrainsMono-OFL.txt")

    metadata = {
        "sources": URLS,
        "files": {
            jetbrains.name: jetbrains.stat().st_size,
        },
    }
    (OUT / "metadata.json").write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    print(json.dumps(metadata, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
