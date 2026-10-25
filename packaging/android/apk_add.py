"""Append file entries to an existing APK/zip in place.

Usage: apk_add.py APK PATH NAME [PATH NAME …]

Each NAME is the entry's archive path. .so entries are stored uncompressed
(STORED) so the loader can map them directly; everything else deflates.
"""

import sys
import zipfile

def main() -> None:
    apk = sys.argv[1]
    pairs = sys.argv[2:]
    if len(pairs) % 2 != 0:
        raise SystemExit("usage: apk_add.py APK PATH NAME [PATH NAME …]")
    # Rebuild into a temp zip: Python cannot append STORED entries cleanly to a
    # compressed archive, so rewrite every entry once.
    with zipfile.ZipFile(apk) as src:
        entries = [(i.filename, src.read(i.filename), i.compress_type) for i in src.infolist()]
    with zipfile.ZipFile(apk + ".tmp", "w") as dst:
        for name, data, method in entries:
            dst.writestr(name, data, method)
        for path, name in zip(pairs[0::2], pairs[1::2]):
            store = name.endswith(".so")
            with open(path, "rb") as f:
                dst.writestr(
                    name,
                    f.read(),
                    zipfile.ZIP_STORED if store else zipfile.ZIP_DEFLATED,
                )
    import os
    os.replace(apk + ".tmp", apk)

if __name__ == "__main__":
    main()
