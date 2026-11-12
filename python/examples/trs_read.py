"""Read a tool version and its primary descriptor from a TRS.

Run from ``python/`` after installing the bindings::

    python examples/trs_read.py https://workflowhub.eu/ga4gh/trs/v2 26 1 CWL
"""

import argparse

from ga4gh_sdk import TRS, Configuration


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("api_root", help="TRS API root, including /ga4gh/trs/v2")
    parser.add_argument("tool_id", help="TRS tool ID")
    parser.add_argument("version_id", help="TRS version ID")
    parser.add_argument("descriptor_type", help="Descriptor type, such as CWL or PLAIN_CWL")
    args = parser.parse_args()

    trs = TRS(Configuration(args.api_root))
    print("TRS version:", trs.api_version)
    version = trs.get_version(args.tool_id, args.version_id)
    print("Version:", version.id, version.name)
    descriptor = trs.get_descriptor(args.tool_id, args.version_id, args.descriptor_type)
    content = descriptor.content or ""
    print("Descriptor:", f"{len(content)} characters", descriptor.url or "")
    for file in trs.list_files(args.tool_id, args.version_id, args.descriptor_type.removeprefix("PLAIN_")):
        print("File:", file.file_type, file.path)


if __name__ == "__main__":
    main()
