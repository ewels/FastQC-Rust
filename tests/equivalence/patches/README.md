# Equivalence Patches

Unified diff patches applied to Java reference output before comparing against
Rust output. These normalize known, expected differences.

## Naming convention

- `{test_case_name}_fastqc_data.patch` — patches for fastqc_data.txt
- `{test_case_name}_summary.patch` — patches for summary.txt
- `_universal_fastqc_data.patch` — applied to ALL test cases' fastqc_data.txt

If no patch file exists for a test case, exact match is expected.

## Current patches

### `_universal_fastqc_data.patch`

Normalizes the version header line. The Java reference data was generated from
a slightly newer Java FastQC build (0.12.2.devel) than the v0.12.1 release we
track, because the v0.12.1 tag has build issues. The actual analysis output is
identical; only the version string in the header line differs.

This patch should be removed once reference data is regenerated from a clean
v0.12.1 build or when the tracked version is updated to match.
