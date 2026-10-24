import argparse
import gzip
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description="Pin the RePoE snapshot used by offline tests.")
    parser.add_argument("source", type=Path)
    source = parser.parse_args().source
    destination = Path(__file__).resolve().parents[1] / "fixtures" / "repoe"
    manifest_path = destination / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["version"] = (source / "version.txt").read_text().strip()
    for name, details in manifest["files"].items():
        raw = (source / f"{name}.json").read_bytes()
        records = json.loads(raw)
        details["source_sha256"] = hashlib.sha256(raw).hexdigest()
        details["entries"] = len(records)
        fields = set(details["fields"])
        if fields != {"all"}:
            def project(record):
                return {key: value for key, value in record.items() if key in fields}

            records = {key: project(value) for key, value in records.items()} if isinstance(records, dict) else [project(value) for value in records]
        encoded = json.dumps(records, separators=(",", ":"), ensure_ascii=False).encode()
        (destination / f"{name}.json.gz").write_bytes(gzip.compress(encoded, mtime=0))
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    (destination / "version.txt").write_text(manifest["version"] + "\n")


if __name__ == "__main__":
    main()
