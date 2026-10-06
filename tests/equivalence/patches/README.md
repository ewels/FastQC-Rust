# Equivalence Patches

Unified diff patches applied to Java reference output before comparing against
Rust output. These normalize known, expected differences.

## Naming convention

- `{test_case_name}_fastqc_data.patch` — patches for fastqc_data.txt
- `{test_case_name}_summary.patch` — patches for summary.txt
- `{test_case_name}_{image}_svg.patch` — patches for `Images/{image}.svg`
- `_universal_{stem}.patch` — applied to that file in ALL test cases

If no patch file exists for a test case, exact match is expected.

## Current patches

There are no text patches: `fastqc_data.txt` and `summary.txt` match Java exactly.

The `*_svg.patch` files cover SVG text and legend x-positions, which differ by a
few pixels because Rust measures text with bundled Liberation Sans. SVGs are
normalised first (see `_normalize_svg()` in `compare.py`), so the patches only
hold what is left. Regenerate them with `uv run tests/equivalence/update_svg_patches.py`
and review the diff: anything beyond text/legend positions is a real difference.
