#!/usr/bin/env python3
"""Recipe-author contract v1: exact per-sequence canonical DNA k-mer histogram.

No network, secrets, randomness, locale-dependent sorting or wall-clock input.
Only uppercase ASCII A/C/G/T is accepted. FASTA headers and record ordering do
not affect the histogram; sequence boundaries do. CPython 3.13 on Linux arm64/x86_64,
the pinned image, script bytes, k, fixed input and output layout define identity.
"""
import collections
import json
import pathlib
import platform
import sys

source, destination, k_text = sys.argv[1:]
k = int(k_text)
if not 1 <= k <= 10:
    raise ValueError("k must be between 1 and 10")
counts = collections.Counter()
sequences = []
for line in pathlib.Path(source).read_text(encoding="ascii").splitlines():
    if line.startswith(">"):
        sequences.append("")
    elif line:
        if not sequences or any(c not in "ACGT" for c in line):
            raise ValueError("expected uppercase synthetic DNA FASTA")
        sequences[-1] += line
for sequence in sequences:
    counts.update(sequence[index:index + k] for index in range(len(sequence) - k + 1))
pathlib.Path(destination).write_text(
    json.dumps(dict(sorted(counts.items())), sort_keys=True, separators=(",", ":")) + "\n",
    encoding="ascii",
)
pathlib.Path("runtime.json").write_text(json.dumps({
    "os": platform.system().lower(),
    "architecture": platform.machine(),
    "runtime": f"{platform.python_implementation()}-{sys.version_info.major}.{sys.version_info.minor}",
}, sort_keys=True))
