"""Package a Linux Rust game binary and its validated content for staging."""

import argparse
import gzip
from hashlib import file_digest
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]
SERVICE = ROOT / "rust/deploy/goa2-rust.service"


def checksum(path):
    with path.open("rb") as source:
        return file_digest(source, "sha256").hexdigest()


def package(binary, content, output):
    if output.exists():
        raise FileExistsError(f"Output already exists: {output}")

    # Validate the exact copies destined for the archive. Package publication
    # is atomic and refuses to replace an existing release.
    with TemporaryDirectory(prefix=".goa2-package-", dir=output.parent) as directory:
        stage = Path(directory)
        executable = stage / "goa2-game-server"
        pack = stage / "live-game-content.json"
        shutil.copyfile(binary, executable)
        executable.chmod(0o755)
        shutil.copyfile(content, pack)
        identity = json.loads(subprocess.run(
            [executable, "--check-content", pack], check=True, capture_output=True,
            text=True, timeout=30,
        ).stdout)
        if identity["engine"] != "rust" or identity["target_os"] != "linux":
            raise ValueError("Use a Linux Rust game server binary")
        if identity["binary_sha256"] != checksum(executable) or identity["content_sha256"] != checksum(pack):
            raise ValueError("The staged files do not match the reported service identity")
        shutil.copyfile(SERVICE, stage / SERVICE.name)

        manifest = stage / "manifest.json"
        manifest.write_text(json.dumps({"format_version": 1, "service": identity}, indent=2) + "\n",
                            encoding="utf-8")
        files = [executable, pack, stage / SERVICE.name, manifest]
        sums = stage / "SHA256SUMS"
        sums.write_text("".join(f"{checksum(path)}  {path.name}\n" for path in files), encoding="ascii")

        archive = stage / "release.tar.gz"
        with archive.open("wb") as output_file, gzip.GzipFile(
            filename="", mode="wb", fileobj=output_file, mtime=0,
        ) as compressed, tarfile.open(fileobj=compressed, mode="w") as tar:
            for path in [*files, sums]:
                entry = tarfile.TarInfo(path.name)
                entry.mode = 0o755 if path == executable else 0o644
                entry.size = path.stat().st_size
                with path.open("rb") as source:
                    tar.addfile(entry, source)
        os.link(archive, output)
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("content", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        identity = package(args.binary, args.content, args.output)
    except subprocess.CalledProcessError as error:
        parser.exit(1, f"Cannot package the binary and content: {error.stderr.strip()}\n")
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"Cannot package the binary and content: {error}\n")
    print(f"Packaged {identity['target_arch']} Linux, rules {identity['rules_version']}: {args.output}")


if __name__ == "__main__":
    main()
