#!/usr/bin/env bash
# Installs the Funnel version that the integration suites use (linux/amd64)
# into the directory given as $1, after checking its checksum. CI calls it
# from the integration job and the CLI image workflow.
set -euo pipefail

version=v0.11.14
sha256=007e85ee808f131034854f6868efc9b11d3205f2fc1c778f339b484e9271996b
dest=${1:?usage: tests/install-funnel.sh DIRECTORY}

archive=funnel-linux-amd64-$version.tar.gz
mkdir -p "$dest"
curl --fail --location --silent --show-error --output "$dest/$archive" \
    "https://github.com/calypr/funnel/releases/download/$version/$archive"
echo "$sha256  $dest/$archive" | sha256sum --check
tar -xzf "$dest/$archive" -C "$dest" funnel
rm "$dest/$archive"
