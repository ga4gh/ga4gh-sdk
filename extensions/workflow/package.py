"""Build a native workflow provider and a relocatable manifest without loading it."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def publish(destination, artifact, descriptor, readme):
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination) as temporary:
        stage = Path(temporary)
        library = stage / artifact.name
        digest = hashlib.sha512()
        with artifact.open("rb") as source, library.open("wb") as target:
            for chunk in iter(lambda: source.read(1 << 20), b""):
                digest.update(chunk)
                target.write(chunk)
        hexdigest = digest.hexdigest()
        name = f"{artifact.stem}-{hexdigest}{artifact.suffix}"
        manifest = dict(json.loads(descriptor.read_text()), manifest_version=1,
                        path=name, digest=hexdigest)
        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        shutil.copyfile(readme, stage / "README.md")
        library.replace(destination / name)
        (stage / "README.md").replace(destination / "README.md")
        (stage / "manifest.json").replace(destination / "manifest.json")
    return destination / "manifest.json"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    package = Path(__file__).resolve().parent
    if args.debug:
        output = subprocess.check_output(
            ["cargo", "build", "--locked", "--message-format=json-render-diagnostics"],
            cwd=package, text=True,
        )
    else:
        output = subprocess.check_output(
            ["cargo", "build", "--locked", "--message-format=json-render-diagnostics", "--release"],
            cwd=package, text=True,
        )
    messages = [json.loads(line) for line in output.splitlines()]
    build = next(message for message in messages if message["reason"] == "compiler-artifact"
                 and message["target"]["name"] == "ga4gh_workflow_extension")
    artifact = next(Path(name) for name in build["filenames"]
                    if Path(name).suffix in {".dylib", ".dll", ".so"})
    descriptor = next(Path(message["out_dir"]) / "descriptor.json" for message in messages
                      if message["reason"] == "build-script-executed"
                      and message["package_id"] == build["package_id"])
    print(publish(args.destination, artifact, descriptor, package / "README.md"))


if __name__ == "__main__":
    main()
