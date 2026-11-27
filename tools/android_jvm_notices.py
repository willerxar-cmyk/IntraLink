"""Preserve exact Android JVM runtime artifacts' license provenance."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET
import zipfile

KOTLIN_VERSION = "2.2.10"
KOTLIN_NOTICE_BLOBS = {
    "COPYRIGHT.txt": "8dc8226f64f07dc5134d3674371cbfe109465cd9",
    "COPYRIGHT_HEADER.txt": "3e86cd52df236ddc8c8bbdf2814d2a79d885d4d5",
    "LICENSE.txt": "d645695673349e3947e8e5ae42332d0ac3164cd7",
    "NOTICE.txt": "80dbbefa483f605b7e69a1458f9f66b277b61988",
}
NAMESPACE = {"m": "http://maven.apache.org/POM/4.0.0"}
NOTICE = re.compile(r"(?:^|/)(?:licen[cs]e|notice|copyright|copying|authors)[^/]*$", re.I)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def upstream_license():
    result = {}
    for name, sha in KOTLIN_NOTICE_BLOBS.items():
        blob = json.loads(subprocess.check_output(
            ["gh", "api", f"repos/JetBrains/kotlin/git/blobs/{sha}"], encoding="utf-8"
        ))
        data = base64.b64decode(blob["content"])
        if hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() != sha:
            raise ValueError("Invalid upstream notice blob")
        result[name] = data
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime_graph", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    rows = json.loads(args.runtime_graph.read_text(encoding="utf-8"))
    actual = {(r["group"], r["name"], r["version"]) for r in rows}
    reviewed = {
        ("org.jetbrains.kotlin", "kotlin-stdlib", KOTLIN_VERSION),
        ("org.jetbrains", "annotations", "13.0"),
    }
    if actual != reviewed or len(rows) != len(reviewed):
        raise ValueError("Review the changed Maven runtime graph before packaging")
    notices = upstream_license()
    contents = {}
    summary = []
    for row in rows:
        coordinate = ":".join(row[k] for k in ("group", "name", "version"))
        folder = "maven/" + coordinate.replace(":", "/") + "/"
        pom = Path(row["pom"]).read_bytes()
        artifact = Path(row["artifact"]).read_bytes()
        element = ET.fromstring(pom)
        declarations = [
            {"name": node.findtext("m:name", namespaces=NAMESPACE),
             "url": node.findtext("m:url", namespaces=NAMESPACE)}
            for node in element.findall("m:licenses/m:license", NAMESPACE)
        ]
        if not any("Apache" in (x["name"] or "") and "2.0" in (x["name"] or "") for x in declarations):
            raise ValueError("Unexpected Maven license declaration")
        contents[folder + "original.pom"] = pom
        with zipfile.ZipFile(Path(row["artifact"])) as jar:
            if jar.testzip() is not None:
                raise ValueError("Damaged Maven runtime artifact")
            for name in jar.namelist():
                if NOTICE.search(name):
                    contents[folder + "embedded/" + name] = jar.read(name)
        review_required = row["name"] == "annotations"
        if row["name"] == "kotlin-stdlib":
            for name, data in notices.items():
                contents[folder + "upstream/" + name] = data
            source = "Kotlin v2.2.10 repository license directory, exact Git blobs"
        else:
            # The original annotations JAR has no separate license file.
            # Preserve its exact POM and the declared Apache-2.0 terms. Do not
            # present Kotlin's copyright or NOTICE as annotations attribution.
            contents[folder + "APACHE-2.0.txt"] = notices["LICENSE.txt"]
            contents[folder + "PROVENANCE.txt"] = (
                "The Maven POM declares Apache-2.0. This published JAR has no "
                "standalone license/notice; the standard Apache-2.0 terms are "
                "provided without inventing an original copyright statement. "
                "Exact original notice provenance requires independent review.\n"
            ).encode()
            source = "Original Maven POM plus declared Apache-2.0 terms"
        summary.append({
            "coordinate": coordinate, "sha256": digest(artifact),
            "pom_sha256": digest(pom), "licenses": declarations,
            "notice_source": source, "review_required": review_required,
        })
    contents["jvm-runtime-inventory.json"] = (json.dumps(summary, indent=2) + "\n").encode()
    contents["THIRD-PARTY-NOTICES.txt"] = (
        "IntraLink Android JVM runtime inventory. Native Cargo graphs are separate.\n"
        + "\n".join(f"{r['coordinate']}: {r['notice_source']}" for r in summary)
        + "\nNot an independent license audit.\n"
    ).encode()
    with zipfile.ZipFile(args.output, "x", zipfile.ZIP_DEFLATED) as z:
        for name, content in sorted(contents.items()):
            z.writestr(name, content)
    with zipfile.ZipFile(args.output) as z:
        if z.testzip() is not None:
            raise ValueError("Invalid JVM notice archive")
    print(f"JVM runtime inventory: {len(summary)} Maven artifacts; {args.output}")


if __name__ == "__main__":
    main()
