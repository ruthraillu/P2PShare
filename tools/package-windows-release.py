"""Build the portable Windows release from prebuilt, verified FFmpeg DLLs."""
import argparse
import hashlib
import json
import re
from pathlib import Path
from tempfile import NamedTemporaryFile
from zipfile import ZIP_DEFLATED, ZipFile

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "vendor" / "ffmpeg-runtime"
INFO = json.loads((RUNTIME / "build-info.json").read_text(encoding="utf-8"))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def copy_dll(source, target, expected, name):
    count = 0
    sha = hashlib.sha256()
    while chunk := source.read(1024 * 1024):
        count += len(chunk)
        if count > expected["bytes"]:
            raise ValueError(f"{name} exceeds its pinned size")
        target.write(chunk)
        sha.update(chunk)
    if count != expected["bytes"] or sha.hexdigest() != expected["sha256"]:
        raise ValueError(f"{name} does not match its pinned size and SHA-256")


def add_codecs(output, source, open_dll):
    for name, expected in INFO["files"].items():
        with open_dll(source, name) as src, output.open(f"codecs/{name}", "w") as dst:
            copy_dll(src, dst, expected, name)


def package(args):
    with ZipFile(args.temp, "w", compression=ZIP_DEFLATED, compresslevel=5) as output:
        output.write(args.exe, "P2PShare.exe")
        output.write(ROOT / "config.example.toml", "config.example.toml")
        for name in ("README.txt", "build-info.json", "ffmpeg-build-configuration.txt"):
            output.write(RUNTIME / name, f"codecs/{name}")
        output.write(ROOT / "vendor/ffmpeg/COPYING.GPLv3", "codecs/FFmpeg-COPYING.GPLv3.txt")
        for name in ("LICENSE-MIT", "LICENSE-APACHE", "P2PSHARE-CHANGES.md"):
            output.write(ROOT / "vendor/egui-directx11" / name, f"licenses/egui-directx11/{name}")

        if args.seed_zip:
            expected = json.loads((RUNTIME / "codec-seed.json").read_text(encoding="utf-8"))
            if digest(args.seed_zip) != expected["sha256"]:
                raise ValueError("The pinned codec seed ZIP checksum does not match")
            with ZipFile(args.seed_zip) as seed:
                add_codecs(
                    output, seed,
                    lambda archive, name: archive.open(f"codecs/{name}"),
                )
        else:
            add_codecs(
                output, args.codec_dir,
                lambda folder, name: (folder / name).open("rb"),
            )
        assets = ROOT / "assets/windows-update"
        for path in sorted(assets.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"Linked update asset: {path}")
            if path.is_file():
                parts = path.relative_to(assets).parts
                if any(not re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]*", part)
                       or part.endswith(".") or re.fullmatch(r"(CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])", part.split(".")[0], re.I)
                       for part in parts):
                    raise ValueError(f"Invalid update asset path: {path}")
                output.write(path, "data/assets/" + "/".join(parts))
    # Reopen to read the on-disk names, which ZIP normalizes to forward slashes.
    with ZipFile(args.temp, "a", compression=ZIP_DEFLATED, compresslevel=5) as output:
        names = output.namelist()
        if len(names) > 256 or len({name.lower() for name in names}) != len(names):
            raise ValueError("Too many update files or conflicting filenames")
        if any(len(name) > 180 for name in names) or sum(entry.file_size for entry in output.infolist()) > 512 * 1024 * 1024 - 256 * 1024:
            raise ValueError("Update file paths or expanded archive exceeded limits")
        inventory = {}
        for name in names:
            with output.open(name) as file:
                inventory[name] = hashlib.file_digest(file, "sha256").hexdigest()
        output.writestr("data/updates/managed.json", json.dumps(inventory, sort_keys=True))
    if args.temp.stat().st_size > 256 * 1024 * 1024:
        raise ValueError("Compressed update exceeds the updater's size limit")
    with ZipFile(args.temp) as output:
        if bad := output.testzip():
            raise ValueError(f"Portable ZIP checksum failed for {bad}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--codec-dir", type=Path)
    source.add_argument("--seed-zip", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(prefix="portable-", suffix=".zip", dir=args.output.parent, delete=False) as temp:
        args.temp = Path(temp.name)
    try:
        package(args)
        args.temp.rename(args.output)
    finally:
        args.temp.unlink(missing_ok=True)
    print(f"Created {args.output} ({digest(args.output)})")


if __name__ == "__main__":
    main()
