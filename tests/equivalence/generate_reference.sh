#!/bin/bash
# Generate Java FastQC reference data for equivalence testing.
#
# Usage: ./generate_reference.sh /path/to/java/fastqc/directory
#
# Prerequisites:
#   - Java FastQC must be built (ant build) in the specified directory
#   - Java 11+ must be available

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DATA_DIR="$REPO_ROOT/tests/data"
REF_DIR="$SCRIPT_DIR/reference"

if [ $# -lt 1 ]; then
    echo "Usage: $0 /path/to/java/fastqc/directory"
    echo "  The directory should contain bin/ with compiled classes and the JAR files."
    exit 1
fi

JAVA_FASTQC_DIR="$1"

# Verify Java FastQC is built
if [ ! -d "$JAVA_FASTQC_DIR/bin" ]; then
    echo "Error: $JAVA_FASTQC_DIR/bin not found. Run 'ant build' first."
    exit 1
fi

# Build classpath
CLASSPATH="$JAVA_FASTQC_DIR/bin"
for jar in "$JAVA_FASTQC_DIR"/*.jar; do
    [ -f "$jar" ] && CLASSPATH="$CLASSPATH:$jar"
done
if [ -d "$JAVA_FASTQC_DIR/lib" ]; then
    for jar in "$JAVA_FASTQC_DIR"/lib/*.jar; do
        [ -f "$jar" ] && CLASSPATH="$CLASSPATH:$jar"
    done
fi

TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# Map Rust CLI flags to Java system properties
map_args_to_java_props() {
    local props=""
    local i=0
    local args=("$@")
    while [ $i -lt ${#args[@]} ]; do
        case "${args[$i]}" in
            --nogroup)    props="$props -Dfastqc.nogroup=true" ;;
            --expgroup)   props="$props -Dfastqc.expgroup=true" ;;
            --casava)     props="$props -Dfastqc.casava=true" ;;
            --nofilter)   props="$props -Dfastqc.nofilter=true" ;;
            --kmers)      i=$((i+1)); props="$props -Dfastqc.kmer_size=${args[$i]}" ;;
            --min_length) i=$((i+1)); props="$props -Dfastqc.min_length=${args[$i]}" ;;
            --dup_length) i=$((i+1)); props="$props -Dfastqc.dup_length=${args[$i]}" ;;
            --nano)       props="$props -Dfastqc.nano=true" ;;
            --format)     i=$((i+1)); props="$props -Dfastqc.sequence_format=${args[$i]}" ;;
            *)            echo "  WARNING: Unknown arg ${args[$i]}" ;;
        esac
        i=$((i+1))
    done
    echo "$props"
}

run_java_fastqc() {
    local name="$1"
    local input_file="$2"
    shift 2
    local args=("$@")

    echo "Generating reference: $name"

    local outdir="$TMPDIR/$name"
    mkdir -p "$outdir"

    # Convert args to Java system properties
    local java_props
    java_props=$(map_args_to_java_props "${args[@]}")

    # Run Java FastQC
    # shellcheck disable=SC2086
    java -Djava.awt.headless=true \
         "-Dfastqc.output_dir=$outdir" \
         $java_props \
         -cp "$CLASSPATH" \
         uk.ac.babraham.FastQC.FastQCApplication \
         "$input_file" 2>&1 || true

    # Find the zip
    local basename
    basename=$(basename "$input_file" | sed 's/\.[^.]*$//')
    local zip_path="$outdir/${basename}_fastqc.zip"

    if [ ! -f "$zip_path" ]; then
        echo "  ERROR: ZIP not found at $zip_path"
        echo "  Files: $(ls "$outdir" 2>/dev/null || echo '(empty)')"
        return 1
    fi

    # Extract to reference directory
    local dest="$REF_DIR/$name"
    rm -rf "$dest"
    mkdir -p "$dest"

    local extract_dir="$TMPDIR/${name}_extract"
    mkdir -p "$extract_dir"
    unzip -q -o "$zip_path" -d "$extract_dir"

    # Move contents from the *_fastqc/ subdirectory to dest
    local inner_dir="$extract_dir/${basename}_fastqc"
    if [ -d "$inner_dir" ]; then
        cp -r "$inner_dir"/* "$dest/"
    fi

    echo "  OK: $(ls "$dest" | tr '\n' ' ')"
}

# Parse YAML test cases
current_name=""
current_file=""
current_args=""

process_case() {
    if [ -n "$current_name" ] && [ -n "$current_file" ]; then
        # shellcheck disable=SC2086
        run_java_fastqc "$current_name" "$DATA_DIR/$current_file" $current_args
    fi
}

while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    [[ -z "${line// }" ]] && continue

    if [[ "$line" =~ ^-[[:space:]]+name:[[:space:]]+(.*) ]]; then
        process_case
        current_name="${BASH_REMATCH[1]}"
        current_file=""
        current_args=""
    elif [[ "$line" =~ ^[[:space:]]+file:[[:space:]]+(.*) ]]; then
        current_file="${BASH_REMATCH[1]}"
    elif [[ "$line" =~ ^[[:space:]]+args:[[:space:]]+\[(.*)\] ]]; then
        local_args="${BASH_REMATCH[1]}"
        current_args=$(echo "$local_args" | sed 's/"//g; s/,/ /g; s/^ *//; s/ *$//')
    fi
done < "$SCRIPT_DIR/test_cases.yaml"

process_case

echo ""
echo "Reference data generated in $REF_DIR"
echo "Test cases: $(ls "$REF_DIR" | wc -l | tr -d ' ')"
