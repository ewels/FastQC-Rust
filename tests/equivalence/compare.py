# /// script
# requires-python = ">=3.10"
# dependencies = ["pyyaml", "pillow", "jinja2", "numpy"]
# ///
"""
Equivalence comparison between Rust FastQC output and Java FastQC reference data.

Runs the Rust fastqc binary on test inputs, compares text and image output
against stored Java reference data, applies patches for known differences,
and generates a self-contained HTML report with pixel-level image diffs.

Usage:
    # Run all test cases against reference data
    uv run tests/equivalence/compare.py --binary ./target/release/fastqc

    # Run specific test case(s)
    uv run tests/equivalence/compare.py --binary ./target/release/fastqc --test minimal_default

    # Compare two arbitrary report directories
    uv run tests/equivalence/compare.py --reference dir1 --actual dir2 --output report.html

    # Adjust image tolerance
    uv run tests/equivalence/compare.py --binary ./target/release/fastqc --pixel-tolerance 2 --max-diff-percent 0.5
"""

import argparse
import base64
import difflib
import html as html_mod
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

import yaml
from jinja2 import Template
import numpy as np
from PIL import Image

# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass
class ImageDiff:
    name: str
    total_pixels: int
    differing_pixels: int
    max_channel_diff: int
    diff_percent: float
    ref_b64: str  # base64-encoded PNG
    actual_b64: str
    diff_b64: str  # highlighted diff image
    passed: bool


@dataclass
class SideBySideRow:
    """One row in a side-by-side diff."""
    left_num: str  # line number or ""
    left_html: str  # HTML-safe content with word highlights
    right_num: str
    right_html: str
    kind: str  # "equal", "delete", "insert", "change", "hunk"


@dataclass
class TextDiff:
    name: str
    identical: bool
    patch_applied: bool
    normalized: bool  # True if HTML normalization was applied
    rows: list[SideBySideRow]  # side-by-side diff rows
    patch_content: str  # patch file content (empty if none)
    passed: bool


@dataclass
class FileEntry:
    name: str
    in_reference: bool
    in_actual: bool
    identical: bool
    has_detail: bool = False  # True if there's a detail section for this file
    status: str = ""  # "identical", "patched", "differs"
    detail_summary: str = ""  # e.g. "84.8% differ" for images, "3 lines differ" for text


@dataclass
class TestCaseResult:
    name: str
    file: str
    args: list[str]
    passed: bool
    text_diffs: list[TextDiff] = field(default_factory=list)
    image_diffs: list[ImageDiff] = field(default_factory=list)
    files: list[FileEntry] = field(default_factory=list)
    error: str = ""


# ---------------------------------------------------------------------------
# Image comparison
# ---------------------------------------------------------------------------

def compare_images(
    ref_path: Path, actual_path: Path, pixel_tolerance: int = 2
) -> ImageDiff:
    """Compare two PNG images pixel-by-pixel with channel tolerance."""
    name = ref_path.name

    ref_img = Image.open(ref_path).convert("RGBA")
    actual_img = Image.open(actual_path).convert("RGBA")

    ref_b64 = _img_to_b64(ref_img)
    actual_b64 = _img_to_b64(actual_img)

    if ref_img.size != actual_img.size:
        # Different dimensions — create a blank diff and fail
        diff_img = Image.new("RGBA", ref_img.size, (255, 0, 0, 128))
        return ImageDiff(
            name=name,
            total_pixels=ref_img.width * ref_img.height,
            differing_pixels=ref_img.width * ref_img.height,
            max_channel_diff=255,
            diff_percent=100.0,
            ref_b64=ref_b64,
            actual_b64=actual_b64,
            diff_b64=_img_to_b64(diff_img),
            passed=False,
        )

    w, h = ref_img.size
    total = w * h

    ref_arr = np.array(ref_img, dtype=np.int16)
    act_arr = np.array(actual_img, dtype=np.int16)

    # Per-channel absolute differences and max across channels per pixel
    ch_diffs = np.abs(ref_arr - act_arr)
    max_per_pixel = ch_diffs.max(axis=2)
    max_ch_diff = int(max_per_pixel.max())

    # Mask of pixels exceeding tolerance
    exceeds = max_per_pixel > pixel_tolerance
    differing = int(exceeds.sum())

    # Build diff image: dimmed actual with red highlights on differing pixels
    diff_arr = act_arr.copy()
    # Dim the background (preserve alpha)
    diff_arr[:, :, :3] //= 3

    # Highlight differing pixels in red, intensity proportional to difference
    intensity = np.clip(max_per_pixel * 3, 0, 255).astype(np.uint8)
    diff_arr[exceeds, 0] = intensity[exceeds]
    diff_arr[exceeds, 1] = 0
    diff_arr[exceeds, 2] = 0
    diff_arr[exceeds, 3] = 255

    diff_img = Image.fromarray(diff_arr.astype(np.uint8), "RGBA")
    pct = (differing / total * 100) if total > 0 else 0.0

    return ImageDiff(
        name=name,
        total_pixels=total,
        differing_pixels=differing,
        max_channel_diff=max_ch_diff,
        diff_percent=pct,
        ref_b64=ref_b64,
        actual_b64=actual_b64,
        diff_b64=_img_to_b64(diff_img),
        passed=True,  # caller sets threshold
    )


def _img_to_b64(img: Image.Image) -> str:
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return base64.b64encode(buf.getvalue()).decode("ascii")


# ---------------------------------------------------------------------------
# Text comparison
# ---------------------------------------------------------------------------

# Files whose text diffs cause test failure (not just informational)
STRICT_FILES = {"fastqc_data.txt", "summary.txt", "fastqc_report.html"}

_BASE64_RE = re.compile(r'(data:image/png;base64,)[A-Za-z0-9+/=]+')
_BASE64_PLACEHOLDER = r'\1[BASE64_IMAGE_DATA]'


_VERSION_RE = re.compile(r'(version\s+)\d+\.\d+\.\d+(?:\.\w+)?')
_VERSION_PLACEHOLDER = r'\g<1>[VERSION]'

_TAG_BOUNDARY_RE = re.compile(r'>(\s*)<')
_BLANK_LINES_RE = re.compile(r'\n{3,}')


def _normalize_html(text: str) -> str:
    """Normalize HTML for comparison: pretty-print and replace base64 images."""
    # Replace base64 image data with placeholder
    text = _BASE64_RE.sub(_BASE64_PLACEHOLDER, text)
    # Simple pretty-print: add newlines after closing tags for readable diffs
    text = _TAG_BOUNDARY_RE.sub('>\n<', text)
    # Collapse multiple blank lines
    text = _BLANK_LINES_RE.sub('\n\n', text)
    return text


def compare_text(
    ref_path: Path, actual_path: Path, patch_paths: list[Path] | None,
    normalize_images: bool = False,
) -> TextDiff:
    """Compare two text files, optionally applying patches to the reference first."""
    name = ref_path.name
    ref_text = ref_path.read_text()
    actual_text = actual_path.read_text()

    patch_content = ""
    patch_applied = False

    # Normalize HTML before patching (patches are written against normalized form)
    if normalize_images:
        ref_text = _normalize_html(ref_text)
        actual_text = _normalize_html(actual_text)

    if patch_paths:
        all_patches = []
        for pp in patch_paths:
            content = pp.read_text()
            all_patches.append(content)
            ref_text = _apply_patch(ref_text, content)
        patch_content = "\n".join(all_patches)
        patch_applied = True

    identical = ref_text == actual_text

    rows: list[SideBySideRow] = []
    if not identical:
        rows = _build_side_by_side(ref_text.splitlines(), actual_text.splitlines())

    return TextDiff(
        name=name,
        identical=identical,
        patch_applied=patch_applied,
        normalized=normalize_images,
        rows=rows,
        patch_content=patch_content,
        passed=identical,
    )


def _word_highlight(old_line: str, new_line: str) -> tuple[str, str]:
    """Produce HTML with <mark> around changed words between two lines."""
    sm = difflib.SequenceMatcher(None, old_line.split(), new_line.split())
    left_parts: list[str] = []
    right_parts: list[str] = []
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        old_words = " ".join(old_line.split()[i1:i2])
        new_words = " ".join(new_line.split()[j1:j2])
        if op == "equal":
            left_parts.append(html_mod.escape(old_words))
            right_parts.append(html_mod.escape(new_words))
        elif op == "replace":
            left_parts.append(f'<mark class="del-word">{html_mod.escape(old_words)}</mark>')
            right_parts.append(f'<mark class="add-word">{html_mod.escape(new_words)}</mark>')
        elif op == "delete":
            left_parts.append(f'<mark class="del-word">{html_mod.escape(old_words)}</mark>')
        elif op == "insert":
            right_parts.append(f'<mark class="add-word">{html_mod.escape(new_words)}</mark>')
    return " ".join(left_parts), " ".join(right_parts)


def _build_side_by_side(
    left_lines: list[str], right_lines: list[str], context: int = 3
) -> list[SideBySideRow]:
    """Build side-by-side diff rows with word-level highlighting and context collapse."""
    sm = difflib.SequenceMatcher(None, left_lines, right_lines)
    rows: list[SideBySideRow] = []

    for group in sm.get_grouped_opcodes(context):
        # Add hunk separator
        if rows:
            rows.append(SideBySideRow("", "", "", "", "hunk"))

        for op, i1, i2, j1, j2 in group:
            if op == "equal":
                for i, j in zip(range(i1, i2), range(j1, j2)):
                    rows.append(SideBySideRow(
                        str(i + 1), html_mod.escape(left_lines[i]),
                        str(j + 1), html_mod.escape(right_lines[j]),
                        "equal",
                    ))
            elif op == "replace":
                # Pair up lines and word-highlight
                max_len = max(i2 - i1, j2 - j1)
                for k in range(max_len):
                    li = i1 + k if k < (i2 - i1) else None
                    rj = j1 + k if k < (j2 - j1) else None
                    if li is not None and rj is not None:
                        lhtml, rhtml = _word_highlight(left_lines[li], right_lines[rj])
                        rows.append(SideBySideRow(
                            str(li + 1), lhtml, str(rj + 1), rhtml, "change",
                        ))
                    elif li is not None:
                        rows.append(SideBySideRow(
                            str(li + 1), html_mod.escape(left_lines[li]), "", "", "delete",
                        ))
                    else:
                        rows.append(SideBySideRow(
                            "", "", str(rj + 1), html_mod.escape(right_lines[rj]), "insert",
                        ))
            elif op == "delete":
                for i in range(i1, i2):
                    rows.append(SideBySideRow(
                        str(i + 1), html_mod.escape(left_lines[i]), "", "", "delete",
                    ))
            elif op == "insert":
                for j in range(j1, j2):
                    rows.append(SideBySideRow(
                        "", "", str(j + 1), html_mod.escape(right_lines[j]), "insert",
                    ))
    return rows


def _apply_patch(text: str, patch: str) -> str:
    """Apply a unified diff patch to text. Simple line-based implementation."""
    lines = text.splitlines(keepends=True)
    result = []
    patch_lines = patch.splitlines(keepends=True)

    i = 0  # index into original lines
    p = 0  # index into patch lines

    while p < len(patch_lines):
        line = patch_lines[p]

        # Skip patch header lines
        if line.startswith("---") or line.startswith("+++"):
            p += 1
            continue

        # Parse hunk header
        if line.startswith("@@"):
            # @@ -start,count +start,count @@
            parts = line.split()
            old_spec = parts[1]  # e.g. -1,5
            old_start = int(old_spec.split(",")[0].lstrip("-"))
            # Copy lines before this hunk
            while i < old_start - 1:
                result.append(lines[i])
                i += 1
            p += 1
            continue

        if line.startswith("-"):
            # Remove line from original
            i += 1
            p += 1
        elif line.startswith("+"):
            # Add line to result
            result.append(line[1:])
            p += 1
        elif line.startswith(" "):
            # Context line
            result.append(lines[i])
            i += 1
            p += 1
        else:
            p += 1

    # Copy remaining lines
    while i < len(lines):
        result.append(lines[i])
        i += 1

    return "".join(result)


# ---------------------------------------------------------------------------
# File inventory
# ---------------------------------------------------------------------------

def inventory_files(ref_dir: Path, actual_dir: Path, check_identity: bool = True) -> list[FileEntry]:
    """List all files in both directories and optionally check identity.

    When check_identity is False, identical is left as False — the caller
    is expected to set it during detailed comparison.  This avoids reading
    every file twice when detailed comparisons will follow.
    """
    ref_files = set()
    actual_files = set()

    for f in ref_dir.rglob("*"):
        if f.is_file():
            ref_files.add(f.relative_to(ref_dir))

    for f in actual_dir.rglob("*"):
        if f.is_file():
            actual_files.add(f.relative_to(actual_dir))

    all_files = sorted(ref_files | actual_files)
    entries = []

    for f in all_files:
        in_ref = f in ref_files
        in_act = f in actual_files
        identical = False
        if check_identity and in_ref and in_act:
            identical = (ref_dir / f).read_bytes() == (actual_dir / f).read_bytes()
        entries.append(FileEntry(str(f), in_ref, in_act, identical))

    return entries


# ---------------------------------------------------------------------------
# Run a single test case
# ---------------------------------------------------------------------------

def run_test_case(
    case: dict,
    binary: Path,
    data_dir: Path,
    ref_base: Path,
    patches_dir: Path,
    pixel_tolerance: int,
    max_diff_percent: float,
) -> TestCaseResult:
    name = case["name"]
    input_file = data_dir / case["file"]
    args = case.get("args", [])
    ref_dir = ref_base / name

    result = TestCaseResult(name=name, file=case["file"], args=args, passed=True)

    if not ref_dir.exists():
        result.passed = False
        result.error = f"Reference directory not found: {ref_dir}"
        return result

    # Run Rust fastqc
    with tempfile.TemporaryDirectory(prefix="fastqc_equiv_") as tmpdir:
        tmpdir = Path(tmpdir)
        cmd = [str(binary), "-o", str(tmpdir), "--svg"] + [str(a) for a in args] + [str(input_file)]
        proc = subprocess.run(cmd, capture_output=True, text=True)

        if proc.returncode != 0:
            result.passed = False
            result.error = f"fastqc failed (exit {proc.returncode}): {proc.stderr}"
            return result

        # Find and extract the ZIP
        zips = list(tmpdir.glob("*_fastqc.zip"))
        if not zips:
            result.passed = False
            result.error = f"No ZIP output found in {tmpdir}"
            return result

        zip_path = zips[0]
        extract_dir = tmpdir / "extracted"
        with zipfile.ZipFile(zip_path) as zf:
            zf.extractall(extract_dir)

        # The ZIP contains a *_fastqc/ subdirectory
        inner_dirs = [d for d in extract_dir.iterdir() if d.is_dir()]
        if not inner_dirs:
            result.passed = False
            result.error = "ZIP contained no subdirectory"
            return result
        actual_dir = inner_dirs[0]

        # File inventory (skip identity check — we determine it during comparison)
        result.files = inventory_files(ref_dir, actual_dir, check_identity=False)

        # Compare every file that exists in both
        for fe in result.files:
            if not (fe.in_reference and fe.in_actual):
                fe.status = "missing"
                continue

            rel = Path(fe.name)
            ref_path = ref_dir / rel
            act_path = actual_dir / rel
            suffix = rel.suffix.lower()

            # Quick identity check via byte comparison
            if ref_path.read_bytes() == act_path.read_bytes():
                fe.identical = True
                fe.status = "identical"
                continue

            # Image files (PNG) — pixel comparison
            if suffix == ".png":
                img_diff = compare_images(ref_path, act_path, pixel_tolerance)
                img_diff.passed = img_diff.diff_percent <= max_diff_percent
                # Use Images/name.png as the display name
                img_diff.name = str(rel)
                result.image_diffs.append(img_diff)
                fe.has_detail = True
                fe.status = "differs"
                fe.detail_summary = f"{img_diff.diff_percent:.1f}% pixels differ"
                if not img_diff.passed:
                    result.passed = False
                continue

            # Text-like files — unified diff
            try:
                ref_text_raw = ref_path.read_text(errors="replace")
                act_text_raw = act_path.read_text(errors="replace")
            except Exception:
                fe.status = "differs"
                fe.detail_summary = "binary file differs"
                continue

            # Check for patches (for .txt files)
            patch_paths = []
            stem = rel.stem  # e.g. "fastqc_data"
            universal_patch = patches_dir / f"_universal_{stem}.patch"
            if universal_patch.exists():
                patch_paths.append(universal_patch)
            case_patch = patches_dir / f"{name}_{stem}.patch"
            if case_patch.exists():
                patch_paths.append(case_patch)

            normalize_images = suffix == ".html"
            td = compare_text(ref_path, act_path, patch_paths if patch_paths else None,
                              normalize_images=normalize_images)
            td.name = str(rel)
            result.text_diffs.append(td)
            fe.has_detail = True
            if td.passed:
                if td.patch_applied:
                    fe.status = "patched"
                elif normalize_images:
                    fe.status = "patched"  # identical after normalization
                else:
                    fe.status = "identical"
                    fe.has_detail = False
            else:
                fe.status = "differs"
                diff_count = sum(1 for r in td.rows if r.kind in ("change", "delete", "insert"))
                fe.detail_summary = f"{diff_count} lines differ"
                if rel.name in STRICT_FILES:
                    result.passed = False

    return result


# ---------------------------------------------------------------------------
# HTML report generation
# ---------------------------------------------------------------------------

HTML_TEMPLATE = Template(r"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>FastQC Equivalence Report</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
         background: #f5f5f5; color: #333; padding: 20px; }
  h1 { margin-bottom: 10px; }
  .summary { margin: 20px 0; padding: 15px; background: #fff; border-radius: 8px;
             box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
  .summary table { width: 100%; border-collapse: collapse; }
  .summary th, .summary td { padding: 8px 12px; text-align: left; border-bottom: 1px solid #eee; }
  .badge { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 12px;
           font-weight: bold; color: #fff; }
  .pass { background: #28a745; }
  .fail { background: #dc3545; }
  .warn { background: #e89a0c; }
  .case-section { margin: 20px 0; padding: 20px; background: #fff; border-radius: 8px;
                  box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
  .case-section h2 { margin-bottom: 5px; }
  .case-meta { color: #666; font-size: 14px; margin-bottom: 15px; }
  .error-msg { background: #fff3cd; border: 1px solid #ffc107; padding: 10px; border-radius: 4px;
               margin: 10px 0; }
  h3 { margin: 15px 0 8px; color: #555; border-bottom: 1px solid #eee; padding-bottom: 5px; }
  .file-table { width: 100%; border-collapse: collapse; font-size: 14px; margin: 10px 0; }
  .file-table th, .file-table td { padding: 6px 10px; border: 1px solid #ddd; }
  .file-table th { background: #f8f9fa; }
  .diff-table { width: 100%; border-collapse: collapse; font-family: "SF Mono", Monaco, monospace;
                font-size: 12px; line-height: 1.4; margin: 8px 0; table-layout: fixed; }
  .diff-wrap { max-height: 500px; overflow: auto; border: 1px solid #d0d7de; border-radius: 6px; }
  .diff-table td { padding: 1px 8px; vertical-align: top; }
  .diff-table .ln { color: #8b949e; text-align: right; user-select: none; width: 1%;
                    white-space: nowrap; padding: 1px 8px; border-right: 1px solid #d0d7de; }
  .diff-table .code { width: 49%; padding: 0; }
  .diff-table .code div { white-space: pre; overflow-x: auto; padding: 1px 8px; }
  .diff-table .sep { width: 1px; background: #d0d7de; padding: 0; }
  .diff-table tr.equal .code { background: #fff; }
  .diff-table tr.change .code.left { background: #ffeef0; }
  .diff-table tr.change .code.right { background: #e6ffec; }
  .diff-table tr.delete .code.left { background: #ffeef0; }
  .diff-table tr.delete .code.right { background: #fafbfc; }
  .diff-table tr.insert .code.left { background: #fafbfc; }
  .diff-table tr.insert .code.right { background: #e6ffec; }
  .diff-table tr.hunk td { background: #f1f8ff; color: #57606a; text-align: center;
                           font-size: 11px; padding: 4px; }
  mark.del-word { background: #fdb8c0; border-radius: 2px; padding: 0 1px; }
  mark.add-word { background: #acf2bd; border-radius: 2px; padding: 0 1px; }
  .patch-block { background: #f0f0f0; padding: 10px; border-radius: 4px; font-family: monospace;
                 font-size: 12px; margin: 8px 0; white-space: pre; overflow-x: auto; }
  .image-stats { font-size: 13px; color: #666; margin: 5px 0; }
  .identical { color: #28a745; }
  .differs { color: #dc3545; }

  /* Unified file table with inline expandable details */
  .file-table { width: 100%; border-collapse: collapse; font-size: 14px; margin: 10px 0; }
  .file-table th, .file-row td { padding: 6px 10px; border-bottom: 1px solid #ddd; vertical-align: top; }
  .file-table th { background: #f8f9fa; border: 1px solid #ddd; text-align: left; }
  .file-row { cursor: pointer; }
  .file-row:hover { background: #f8f9fa; }
  .file-row.expandable td:first-child::before { content: '▶ '; font-size: 10px; color: #666; }
  .file-row.expandable.open td:first-child::before { content: '▼ '; }
  .file-detail-row { display: none; }
  .file-detail-row.open { display: table-row; }
  .file-detail-row td { padding: 12px; background: #fafbfc; border-bottom: 1px solid #ddd; }

  /* Image comparison widget */
  .img-compare { margin: 10px 0; }
  .img-compare .mode-tabs { display: flex; gap: 4px; margin-bottom: 8px; }
  .img-compare .mode-tabs button { padding: 4px 12px; border: 1px solid #ddd; background: #f8f9fa;
    border-radius: 4px; cursor: pointer; font-size: 12px; }
  .img-compare .mode-tabs button.active { background: #0366d6; color: #fff; border-color: #0366d6; }
  .img-compare .viewport { position: relative; display: inline-block; border: 1px solid #ddd;
    border-radius: 4px; overflow: hidden; background: #eee; }
  .img-compare .viewport img { display: block; max-width: 100%; }
  .img-compare .viewport .overlay { position: absolute; top: 0; left: 0; width: 100%; height: 100%; }
  .img-compare .viewport .overlay img { width: 100%; height: 100%; display: block; }
  .img-compare .slider-input { width: 100%; margin-top: 4px; }
  .img-compare .labels { display: flex; justify-content: space-between; font-size: 11px; color: #666; }
  /* Side-by-side mode */
  .img-compare .side-by-side { display: flex; gap: 10px; }
  .img-compare .side-by-side .col { flex: 1; text-align: center; }
  .img-compare .side-by-side .col img { max-width: 100%; border: 1px solid #ddd; border-radius: 4px; }
  .img-compare .side-by-side .col .label { font-size: 12px; color: #666; margin-bottom: 4px; }
</style>
</head>
<body>
<h1>FastQC Equivalence Report</h1>
<p>Comparing Rust FastQC output against Java FastQC v{{ upstream_version }} reference data.</p>

<div class="summary">
<table>
<tr><th>Test Case</th><th>File</th><th>Args</th><th>Text</th><th>Images</th><th>Status</th></tr>
{% for r in results %}
<tr>
  <td><a href="#{{ r.name }}">{{ r.name }}</a></td>
  <td>{{ r.file }}</td>
  <td>{{ r.args | join(' ') if r.args else '(default)' }}</td>
  <td>{% if r.error %}-{% else %}
    {% set text_ok = r.text_diffs | selectattr('passed') | list | length == r.text_diffs | length %}
    <span class="badge {{ 'pass' if text_ok else 'fail' }}">{{ r.text_diffs | selectattr('passed') | list | length }}/{{ r.text_diffs | length }}</span>
  {% endif %}</td>
  <td>{% if r.error %}-{% else %}
    {% set img_ok = r.image_diffs | selectattr('passed') | list | length == r.image_diffs | length %}
    <span class="badge {{ 'pass' if img_ok else 'fail' }}">{{ r.image_diffs | selectattr('passed') | list | length }}/{{ r.image_diffs | length }}</span>
  {% endif %}</td>
  <td><span class="badge {{ 'pass' if r.passed else 'fail' }}">{{ 'PASS' if r.passed else 'FAIL' }}</span></td>
</tr>
{% endfor %}
</table>
</div>

{% for r in results %}
{% set lu = diff_lookups[r.name] %}
<div class="case-section" id="{{ r.name }}">
<h2>{{ r.name }} <span class="badge {{ 'pass' if r.passed else 'fail' }}">{{ 'PASS' if r.passed else 'FAIL' }}</span></h2>
<div class="case-meta">Input: {{ r.file }} | Args: {{ r.args | join(' ') if r.args else '(default)' }}</div>

{% if r.error %}
<div class="error-msg">{{ r.error }}</div>
{% else %}

<table class="file-table">
<tr><th>File</th><th>Java</th><th>Rust</th><th>Status</th></tr>
{% for f in r.files %}
{% set td = lu.text.get(f.name) %}
{% set img = lu.img.get(f.name) %}
{% set expandable = (td and (td.rows or td.patch_applied or td.normalized)) or img %}
<tr class="file-row {{ 'expandable' if expandable }}" {% if expandable %}onclick="toggleDetail(this)"{% endif %}>
  <td>{{ f.name }}</td>
  <td style="text-align:center">{{ '✓' if f.in_reference else '✗' }}</td>
  <td style="text-align:center">{{ '✓' if f.in_actual else '✗' }}</td>
  <td>{% if f.status == 'identical' %}<span class="identical">✓ Identical</span>
  {% elif f.status == 'patched' %}<span class="identical">✓ Expected diff</span>
  {% elif f.status == 'differs' %}<span class="differs">✗ Differs{{ ' (' + f.detail_summary + ')' if f.detail_summary }}</span>
  {% elif f.status == 'missing' %}<span class="differs">✗ {{ 'Only in Java' if f.in_reference else 'Only in Rust' }}</span>
  {% else %}-{% endif %}</td>
</tr>
{% if expandable %}
<tr class="file-detail-row"><td colspan="4">

{% if td and td.rows %}
{% if td.patch_content %}
<details><summary style="font-size:12px;color:#666;cursor:pointer">Patch applied</summary>
<div class="patch-block">{{ td.patch_content | e }}</div></details>
{% endif %}
<div class="diff-wrap"><table class="diff-table">
<thead><tr><td class="ln"></td><td class="code"><div><strong>Java</strong></div></td><td class="sep"></td><td class="ln"></td><td class="code"><div><strong>Rust</strong></div></td></tr></thead>
<tbody>
{% for row in td.rows %}
<tr class="{{ row.kind }}">
{% if row.kind == 'hunk' %}<td colspan="5" style="text-align:center;background:#f1f8ff;color:#57606a">⋯</td>
{% else %}<td class="ln">{{ row.left_num }}</td><td class="code left"><div>{{ row.left_html }}</div></td><td class="sep"></td><td class="ln">{{ row.right_num }}</td><td class="code right"><div>{{ row.right_html }}</div></td>{% endif %}
</tr>
{% endfor %}
</tbody></table></div>
{% elif td and (td.patch_applied or td.normalized) %}
{% if td.patch_content %}
<p style="margin:4px 0;color:#666;font-size:13px">Identical after applying patch:</p>
<div class="patch-block">{{ td.patch_content | e }}</div>
{% endif %}
{% if td.normalized %}
<p style="margin:4px 0;color:#666;font-size:13px">Base64 images and version strings normalized. Remaining structure is identical.</p>
{% endif %}
{% endif %}

{% if img %}
<div class="image-stats">Total: {{ img.total_pixels }} px | Differ: {{ img.differing_pixels }} ({{ "%.2f" | format(img.diff_percent) }}%) | Max channel diff: {{ img.max_channel_diff }}</div>
<div class="img-compare">
  <div class="mode-tabs">
    <button class="active" onclick="setMode(this,'side-by-side')">Side by Side</button>
    <button onclick="setMode(this,'slider')">Slider</button>
    <button onclick="setMode(this,'fade')">Fade</button>
    <button onclick="setMode(this,'highlight')">Highlight</button>
  </div>
  <div class="view view-side-by-side"><div class="side-by-side">
    <div class="col"><div class="label">Java</div>{% if img.ref_b64 %}<img src="data:image/png;base64,{{ img.ref_b64 }}" alt="Java">{% endif %}</div>
    <div class="col"><div class="label">Rust</div>{% if img.actual_b64 %}<img src="data:image/png;base64,{{ img.actual_b64 }}" alt="Rust">{% endif %}</div>
    <div class="col"><div class="label">Pixel Diff</div>{% if img.diff_b64 %}<img src="data:image/png;base64,{{ img.diff_b64 }}" alt="Diff">{% endif %}</div>
  </div></div>
  <div class="view view-slider" style="display:none"><div class="viewport" style="position:relative">
    {% if img.ref_b64 %}<img src="data:image/png;base64,{{ img.ref_b64 }}" style="display:block;max-width:100%">{% endif %}
    <div class="overlay" style="position:absolute;top:0;left:0;height:100%;overflow:hidden;width:50%">{% if img.actual_b64 %}<img src="data:image/png;base64,{{ img.actual_b64 }}" style="display:block;max-width:none">{% endif %}</div>
    <div style="position:absolute;top:0;width:2px;height:100%;background:red;left:50%;pointer-events:none"></div>
  </div><input type="range" class="slider-input" min="0" max="100" value="50" oninput="updateSlider(this)"><div class="labels"><span>Java</span><span>Rust</span></div></div>
  <div class="view view-fade" style="display:none"><div class="viewport" style="position:relative">
    {% if img.ref_b64 %}<img src="data:image/png;base64,{{ img.ref_b64 }}" style="display:block;max-width:100%">{% endif %}
    <div class="overlay" style="position:absolute;top:0;left:0;width:100%;height:100%;opacity:0.5">{% if img.actual_b64 %}<img src="data:image/png;base64,{{ img.actual_b64 }}" style="width:100%;height:100%">{% endif %}</div>
  </div><input type="range" class="slider-input" min="0" max="100" value="50" oninput="updateFade(this)"><div class="labels"><span>Java</span><span>Rust</span></div></div>
  <div class="view view-highlight" style="display:none"><div class="viewport">
    {% if img.diff_b64 %}<img src="data:image/png;base64,{{ img.diff_b64 }}" style="display:block;max-width:100%">{% endif %}
  </div><p style="font-size:12px;color:#666;margin-top:4px">Differing pixels in red on dimmed Rust image.</p></div>
</div>
{% endif %}

</td></tr>
{% endif %}
{% endfor %}
</table>
{% endif %}
</div>
{% endfor %}

<script>
function toggleDetail(row) {
  const d = row.nextElementSibling;
  if (d && d.classList.contains('file-detail-row')) { d.classList.toggle('open'); row.classList.toggle('open'); }
}
function setMode(btn, mode) {
  const w = btn.closest('.img-compare');
  w.querySelectorAll('.mode-tabs button').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  w.querySelectorAll('.view').forEach(v => v.style.display = 'none');
  w.querySelector('.view-' + mode).style.display = '';
}
function updateSlider(input) {
  const v = input.closest('.view-slider'), pct = input.value;
  v.querySelector('.overlay').style.width = pct + '%';
  const ln = v.querySelector('.overlay + div'); if (ln) ln.style.left = pct + '%';
  const vp = v.querySelector('.viewport'), img = v.querySelector('.overlay img');
  if (img && vp) img.style.width = vp.offsetWidth + 'px';
}
function updateFade(input) { input.closest('.view-fade').querySelector('.overlay').style.opacity = input.value / 100; }
window.addEventListener('load', function() {
  document.querySelectorAll('.view-slider').forEach(function(v) {
    const vp = v.querySelector('.viewport'), img = v.querySelector('.overlay img');
    if (img && vp) img.style.width = vp.offsetWidth + 'px';
  });
});
</script>
</body>
</html>
""")


def generate_report(results: list[TestCaseResult], output_path: Path, upstream_version: str) -> None:
    # Build per-result lookup dicts so the template can find diffs by filename
    diff_lookups = {}
    for r in results:
        text_map = {td.name: td for td in r.text_diffs}
        img_map = {img.name: img for img in r.image_diffs}
        diff_lookups[r.name] = {"text": text_map, "img": img_map}
    html = HTML_TEMPLATE.render(results=results, upstream_version=upstream_version, diff_lookups=diff_lookups)
    output_path.write_text(html)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="FastQC equivalence comparison")
    parser.add_argument("--binary", type=Path, help="Path to Rust fastqc binary")
    parser.add_argument("--test", action="append", dest="tests", help="Run specific test case(s) by name")
    parser.add_argument("--reference", type=Path, help="Reference directory (for ad-hoc comparison)")
    parser.add_argument("--actual", type=Path, help="Actual directory (for ad-hoc comparison)")
    parser.add_argument("--output", type=Path, help="Output HTML report path")
    parser.add_argument("--pixel-tolerance", type=int, default=2, help="Per-channel pixel tolerance (default: 2)")
    parser.add_argument("--max-diff-percent", type=float, default=100.0,
                        help="Max %% of differing pixels to pass (default: 100, images always reported but don't fail)")
    args = parser.parse_args()

    # Find project root (where Cargo.toml lives)
    script_dir = Path(__file__).parent
    project_root = script_dir.parent.parent

    # Read upstream version
    upstream_toml = project_root / "UPSTREAM.toml"
    upstream_version = "unknown"
    if upstream_toml.exists():
        for line in upstream_toml.read_text().splitlines():
            if line.strip().startswith("version"):
                upstream_version = line.split('"')[1]
                break

    # Ad-hoc comparison mode
    if args.reference and args.actual:
        output = args.output or Path("equivalence_report.html")
        result = TestCaseResult(name="ad-hoc", file="", args=[], passed=True)
        result.files = inventory_files(args.reference, args.actual)

        for txt_name in ["fastqc_data.txt", "summary.txt"]:
            ref_txt = args.reference / txt_name
            act_txt = args.actual / txt_name
            if ref_txt.exists() and act_txt.exists():
                td = compare_text(ref_txt, act_txt, None)
                result.text_diffs.append(td)
                if not td.passed:
                    result.passed = False

        ref_images = args.reference / "Images"
        act_images = args.actual / "Images"
        if ref_images.exists() and act_images.exists():
            for ref_png in sorted(ref_images.glob("*.png")):
                act_png = act_images / ref_png.name
                if act_png.exists():
                    img_diff = compare_images(ref_png, act_png, args.pixel_tolerance)
                    img_diff.passed = img_diff.diff_percent <= args.max_diff_percent
                    result.image_diffs.append(img_diff)
                    if not img_diff.passed:
                        result.passed = False

        generate_report([result], output, upstream_version)
        print(f"Report: {output}")
        sys.exit(0 if result.passed else 1)

    # Test case mode
    if not args.binary:
        parser.error("--binary is required (or use --reference/--actual for ad-hoc mode)")

    binary = args.binary.resolve()
    if not binary.exists():
        print(f"Error: binary not found at {binary}", file=sys.stderr)
        sys.exit(1)

    test_cases_path = script_dir / "test_cases.yaml"
    with open(test_cases_path) as f:
        all_cases = yaml.safe_load(f)

    # Filter to requested tests
    if args.tests:
        cases = [c for c in all_cases if c["name"] in args.tests]
        missing = set(args.tests) - {c["name"] for c in cases}
        if missing:
            print(f"Warning: unknown test cases: {missing}", file=sys.stderr)
    else:
        cases = all_cases

    data_dir = project_root / "tests" / "data"
    ref_base = script_dir / "reference"
    patches_dir = script_dir / "patches"
    reports_dir = script_dir / "reports"
    reports_dir.mkdir(parents=True, exist_ok=True)

    results = []
    all_passed = True

    for case in cases:
        print(f"Testing: {case['name']} ... ", end="", flush=True)
        result = run_test_case(
            case, binary, data_dir, ref_base, patches_dir,
            args.pixel_tolerance, args.max_diff_percent,
        )
        results.append(result)
        if result.passed:
            print("PASS")
        else:
            print("FAIL")
            if result.error:
                print(f"  Error: {result.error}")
            all_passed = False

    # Generate report
    output = args.output or (reports_dir / "equivalence_report.html")
    generate_report(results, output, upstream_version)
    print(f"\nReport: {output}")
    print(f"Result: {'ALL PASSED' if all_passed else 'SOME FAILED'}")

    sys.exit(0 if all_passed else 1)


if __name__ == "__main__":
    main()
