"""Build a native workflow provider and a relocatable manifest without loading it."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    package = Path(__file__).resolve().parent
    command = [
        os.environ.get("CARGO", "cargo"), "build", "--locked",
        "--manifest-path", str(package / "Cargo.toml"), "--message-format=json-render-diagnostics",
    ]
    if not args.debug:
        command.append("--release")
    messages = [json.loads(line) for line in subprocess.check_output(command, text=True).splitlines()]
    build = next(message for message in messages if message["reason"] == "compiler-artifact"
                 and message["target"]["name"] == "ga4gh_workflow_extension")
    artifact = next(Path(name) for name in build["filenames"]
                    if Path(name).suffix in {".dylib", ".dll", ".so"})
    descriptor = next(Path(message["out_dir"]) / "descriptor.json" for message in messages
                      if message["reason"] == "build-script-executed"
                      and message["package_id"] == build["package_id"])
    args.destination.mkdir(parents=True, exist_ok=True)
    library = args.destination / artifact.name
    shutil.copyfile(artifact, library)
    with library.open("rb") as content:
        digest = hashlib.sha512()
        for chunk in iter(lambda: content.read(1 << 20), b""):
            digest.update(chunk)
    manifest = dict(json.loads(descriptor.read_text()), manifest_version=1,
                    path=library.name, digest=digest.hexdigest())
    path = args.destination / "manifest.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(path)


if __name__ == "__main__":
    main()
