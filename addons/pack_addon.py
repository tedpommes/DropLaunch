"""Packs an add-on folder into a .dladdon file (a zip with addon.json at the top), next to the folder.

    py pack_addon.py my-addon          ->  my-addon-0.1.0.dladdon

Hidden files (.git, .vscode, ...) and older .dladdon files are left out. MIT licence.
"""
import json
import pathlib
import sys
import zipfile


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    folder = pathlib.Path(sys.argv[1]).resolve()
    manifest_file = folder / "addon.json"
    if not manifest_file.is_file():
        sys.exit(f"No addon.json in {folder}")
    manifest = json.loads(manifest_file.read_text(encoding="utf-8-sig"))
    if folder.name != manifest.get("id"):
        print(f"Note: the folder is called {folder.name}, the id is {manifest.get('id')}. That's fine in a .dladdon.")
    out = folder.parent / f"{manifest['id']}-{manifest['version']}.dladdon"
    files = [
        f
        for f in sorted(folder.rglob("*"))
        if f.is_file() and not any(p.startswith(".") for p in f.relative_to(folder).parts) and f.suffix.lower() != ".dladdon"
    ]
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for f in files:
            z.write(f, f.relative_to(folder).as_posix())
    print(f"{out} ({len(files)} files, {out.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
