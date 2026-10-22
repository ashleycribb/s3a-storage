"""`s3a read <file>` / `s3a merkle < case.json`: the conformance runner protocol."""
import json
import sys

from . import merkle_valid, read_report


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) >= 2 and argv[0] == "read":
        print(json.dumps(read_report(argv[1]), sort_keys=True))
        return 0
    if argv and argv[0] == "merkle":
        print(json.dumps({"valid": merkle_valid(json.load(sys.stdin))}))
        return 0
    sys.stderr.write("usage: s3a read <file.s3a> | s3a merkle < case.json\n")
    return 2


if __name__ == "__main__":
    sys.exit(main())
