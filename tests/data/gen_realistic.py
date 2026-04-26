#!/usr/bin/env python3
"""Deterministically generate a realistic test FASTQ for fastqc-rust equivalence tests.

Produces 1009 reads of 50bp each at uniform Phred 40, with 5 deliberately
overrepresented sequences at non-round percentages. Background reads are
pseudo-random (fixed seed) and below the 0.1% overrepresented threshold.

Designed to expose the percentage-precision bug fixed by ewels/FastQC-Rust#2.

Output is written gzipped (~20 KB vs ~120 KB plain) to keep the repo lean.
The gzip stream uses mtime=0 and the deterministic content above so byte-
identical regeneration is possible across machines.
"""
import gzip
import random
import sys

random.seed(20260426)

QUAL = "I" * 50  # Phred 40
LEN = 50
TOTAL = 1009

OVERREP = [
    ("OVERREP_A_HIGH",  73),  # 73/1009 = 7.23488602...%
    ("OVERREP_B_MID",   37),  # 37/1009 = 3.66699702...%
    ("OVERREP_C_LOW",   11),  # 11/1009 = 1.09018830...%
    ("OVERREP_D_TINY",   5),  #  5/1009 = 0.49554014...%
    ("OVERREP_E_EDGE",   2),  #  2/1009 = 0.19821605...%  (just above 0.1% threshold)
]

def random_seq(rng):
    return "".join(rng.choice("ACGT") for _ in range(LEN))

# Build the five overrepresented sequences. Use deterministic RNG, but reject
# any candidate that collides with a previous sequence to keep counts exact.
overrep_seqs = []
seen = set()
seq_rng = random.Random(20260426)
for label, _count in OVERREP:
    while True:
        s = random_seq(seq_rng)
        if s not in seen:
            overrep_seqs.append(s)
            seen.add(s)
            break

# Build background reads. Each must be unique AND not collide with any
# overrepresented sequence (otherwise the percentages drift).
n_overrep = sum(c for _, c in OVERREP)
n_background = TOTAL - n_overrep
assert n_background > 0
background_seqs = []
while len(background_seqs) < n_background:
    s = random_seq(seq_rng)
    if s in seen:
        continue
    seen.add(s)
    background_seqs.append(s)

# Assemble reads in a deterministic interleaved order so the output is stable.
reads = []
for s, (label, count) in zip(overrep_seqs, OVERREP):
    for i in range(count):
        reads.append((f"{label}_{i+1}", s))
for i, s in enumerate(background_seqs):
    reads.append((f"BACKGROUND_{i+1}", s))

# Shuffle deterministically so overrepresented reads aren't all clustered.
order_rng = random.Random(99)
order_rng.shuffle(reads)
assert len(reads) == TOTAL

out_path = sys.argv[1]
# mtime=0 makes the gzip header deterministic so re-running the generator
# produces byte-identical output on any machine.
with gzip.GzipFile(filename=out_path, mode="wb", mtime=0) as f:
    for header, seq in reads:
        f.write(f"@{header}\n{seq}\n+\n{QUAL}\n".encode("ascii"))

print(f"wrote {out_path}: {TOTAL} reads, {LEN}bp, {len(OVERREP)} overrepresented sequences", file=sys.stderr)
for label, count in OVERREP:
    pct = count * 100 / TOTAL
    print(f"  {label}: {count}/{TOTAL} = {pct}%", file=sys.stderr)
