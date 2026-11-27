"""Local verification-policy tests; generated files are not native releases."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from package_desktop_candidate import TARGETS, write_archive
from verify_desktop_candidate import manifest, source_blob, source_docs, valid_name, verify


def sha(value):
    return hashlib.sha256(value).hexdigest()


class CandidateVerificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.revision = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True
        ).strip()
        cls.docs = {name: source_blob(cls.revision, name)
                    for name in source_docs(cls.revision)}

    def candidate(self, folder, platform, *, changed_doc=False, extra_path=None):
        binary = "intralink.exe" if platform == "windows" else "intralink"
        files = dict(self.docs)
        if changed_doc:
            files["README.md"] += b"\nUnexpected change\n"
        features = ["intralink/jpeg-simd", "intralink/jpeg-parallel",
                    "intralink/resource-monitor"]
        if platform == "windows":
            files[binary] = b"MZ test-only header"
            features.extend(["intralink/wgc-preview", "intralink/input-preview"])
        else:
            files[binary] = b"\x7fELF test-only header"
            features.append("intralink/desktop-preview")
            files["linux-runtime-libraries.txt"] = b"test-only fixture\n"
        files["CANDIDATE.txt"] = (
            f"Source revision: {self.revision}\nTarget: {TARGETS[platform]}\n"
        ).encode()
        files["dependency-inventory.json"] = json.dumps({
            "target": TARGETS[platform], "build_host": TARGETS[platform],
            "packages": [{"name": "test-only", "version": "0"}],
            "features": ",".join(features),
        }).encode()
        files["THIRD-PARTY-NOTICES.txt"] = b"test-only fixture"
        if extra_path:
            files[extra_path] = b"invalid fixture"
        files["SHA256SUMS.txt"] = "".join(
            f"{sha(data)}  {name}\n" for name, data in sorted(files.items())
        ).encode()
        extension = "zip" if platform == "windows" else "tar.gz"
        path = folder / f"intralink-test-{self.revision[:12]}-candidate.{extension}"
        write_archive(path, platform, files, binary)
        path.with_name("SHA256SUMS.txt").write_text(
            f"{sha(path.read_bytes())}  {path.name}\n", encoding="utf-8"
        )
        return path

    def test_paths_and_manifests(self):
        for name in ("", ".", "../escape", "/absolute", "a/../b", "a//b",
                     "C:/escape", "a\\b", "a\nname"):
            self.assertFalse(valid_name(name), name)
        self.assertTrue(valid_name("docs/SECURITY.md"))
        line = f"{'a' * 64}  README.md\n"
        with self.assertRaises(ValueError):
            manifest(line * 2)
        with self.assertRaises(ValueError):
            manifest("bad hash  README.md")

    def test_both_archive_formats_and_source_binding(self):
        for platform in TARGETS:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as folder:
                path = self.candidate(Path(folder), platform)
                result = verify(path, self.revision, platform)
                self.assertEqual(result["source_revision"], self.revision)
                self.assertEqual(result["archive_entries"], result["verified_entries"] + 1)
                with path.open("ab") as stream:
                    stream.write(b"tampered")
                with self.assertRaisesRegex(ValueError, "Outer archive SHA-256"):
                    verify(path, self.revision, platform)

    def test_rehashed_modified_document_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = self.candidate(Path(folder), "linux", changed_doc=True)
            with self.assertRaisesRegex(ValueError, "Source documentation mismatch"):
                verify(path, self.revision, "linux")

    def test_rehashed_unsafe_path_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = self.candidate(Path(folder), "linux", extra_path="../escape")
            with self.assertRaisesRegex(ValueError, "Unsafe or duplicate"):
                verify(path, self.revision, "linux")


if __name__ == "__main__":
    unittest.main()
