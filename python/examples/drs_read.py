"""Read one DRS object without logging its access URLs or headers.

Run from ``python/`` after installing the bindings::

    python examples/drs_read.py https://your-drs.example/ga4gh/drs/v1 object-id
"""

import argparse

from GA4GH import Configuration, DRS


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("api_root", help="DRS API root, including /ga4gh/drs/v1")
    parser.add_argument("object_id", help="DRS object identifier")
    args = parser.parse_args()

    drs = DRS(Configuration(args.api_root))
    obj = drs.get_object_raw(args.object_id)
    print("Object ID:", obj["id"])
    print("Size:", obj.get("size"))


if __name__ == "__main__":
    main()
