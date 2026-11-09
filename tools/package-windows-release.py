"""Build the portable Windows release from prebuilt, verified FFmpeg DLLs."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
from tempfile import NamedTemporaryFile
from zipfile import ZIP_DEFLATED, ZipFile

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "vendor" / "ffmpeg-runtime"
INFO = json.loads((RUNTIME / "build-info.json").read_text(encoding="utf-8"))
SOURCE_ENTRY = f"codecs/source/FFmpeg-{INFO['ffmpeg_commit']}-source.tar.gz"


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


def add_codecs(output, source, open_dll, source_archive):
    for name, expected in INFO["files"].items():
        with open_dll(source, name) as src, output.open(f"codecs/{name}", "w") as dst:
            copy_dll(src, dst, expected, name)
    with source_archive(source) as src, output.open(SOURCE_ENTRY, "w") as dst:
        shutil.copyfileobj(src, dst, length=1024 * 1024)


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
                    lambda archive: archive.open(SOURCE_ENTRY),
                )
        else:
            add_codecs(
                output, args.codec_dir,
                lambda folder, name: (folder / name).open("rb"),
                lambda _: args.ffmpeg_source.open("rb"),
            )
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
    parser.add_argument("--ffmpeg-source", type=Path)
    args = parser.parse_args()
    if bool(args.codec_dir) != bool(args.ffmpeg_source):
        parser.error("--codec-dir and --ffmpeg-source must be supplied together")
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
