import json
import sys

from .reader import merkle_valid, read_report


def main(argv):
    if len(argv) >= 2 and argv[0] == "read":
        print(json.dumps(read_report(argv[1]), sort_keys=True))
        return 0
    if argv and argv[0] == "merkle":
        print(json.dumps({"valid": merkle_valid(json.load(sys.stdin))}))
        return 0
    sys.stderr.write("usage: python3 -m s3a_reader read <file.s3a> | merkle < case.json\n")
    return 2


sys.exit(main(sys.argv[1:]))
