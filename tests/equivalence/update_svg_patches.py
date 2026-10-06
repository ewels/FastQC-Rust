# /// script
# requires-python = ">=3.10"
# dependencies = ["pyyaml", "pillow", "jinja2", "numpy"]
# ///
"""
Regenerate tests/equivalence/patches/*_svg.patch from the current Rust output.

SVG text positions differ slightly from Java (font metrics), so each SVG has
a patch of known differences. Run this after regenerating reference data,
then review the patches: they should only touch text/legend x positions.

Usage:
    cargo build --release
    uv run tests/equivalence/update_svg_patches.py
"""

import difflib
import importlib.util
import subprocess
import tempfile
import zipfile
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
EQ = ROOT / "tests/equivalence"

spec = importlib.util.spec_from_file_location("compare", EQ / "compare.py")
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)

patches = EQ / "patches"
for old in patches.glob("*_svg.patch"):
    old.unlink()

written = 0
for case in yaml.safe_load((EQ / "test_cases.yaml").read_text()):
    with tempfile.TemporaryDirectory() as tmp:
        args = [str(a) for a in case.get("args", [])]
        subprocess.run(
            [str(ROOT / "target/release/fastqc"), "-q", "-o", tmp, *args, str(ROOT / "tests/data" / case["file"])],
            check=True,
            capture_output=True,
        )
        zf = zipfile.ZipFile(next(Path(tmp).glob("*_fastqc.zip")))
        for ref in sorted((EQ / "reference" / case["name"] / "Images").glob("*.svg")):
            member = next((m for m in zf.namelist() if m.endswith("/Images/" + ref.name)), None)
            if member is None:
                continue
            java = compare._normalize_svg(ref.read_text()).splitlines(keepends=True)
            rust = compare._normalize_svg(zf.read(member).decode()).splitlines(keepends=True)
            if java == rust:
                continue
            diff = difflib.unified_diff(java, rust, f"java/Images/{ref.name}", f"rust/Images/{ref.name}")
            (patches / f"{case['name']}_{ref.stem}_svg.patch").write_text("".join(diff))
            written += 1

print(f"Wrote {written} SVG patches")
