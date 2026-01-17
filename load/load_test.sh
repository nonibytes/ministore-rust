#!/usr/bin/env bash
# Ministore Load Test - Needle in a Haystack
#
# This script pre-generates large corpuses (100k and 1M docs) and measures
# how long it takes to find a unique "needle" document.
#
# Usage: ./load_test.sh [100k|1m|both]

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MINISTORE="${SCRIPT_DIR}/../target/release/ministore"
LOAD_DIR="${SCRIPT_DIR}/data"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log() { echo -e "${BLUE}[$(date +%H:%M:%S)]${NC} $1"; }
success() { echo -e "${GREEN}✓${NC} $1"; }
warn() { echo -e "${YELLOW}⚠${NC} $1"; }
error() { echo -e "${RED}✗${NC} $1"; }

# Ensure release binary exists
if [ ! -f "$MINISTORE" ]; then
    log "Building release binary..."
    (cd "$SCRIPT_DIR/.." && cargo build --release)
fi

mkdir -p "$LOAD_DIR"

# Generate JSONL corpus with one "needle" doc
generate_corpus() {
    local count=$1
    local name=$2
    local jsonl_file="${LOAD_DIR}/${name}.jsonl"
    local needle_position=$((RANDOM % count))
    
    log "Generating ${count} documents for ${name} corpus..."
    
    # Generate in batches to avoid memory issues
    rm -f "$jsonl_file"
    
    local batch_size=10000
    local batches=$((count / batch_size))
    local remainder=$((count % batch_size))
    
    local current=0
    for ((b=0; b<batches; b++)); do
        for ((i=0; i<batch_size; i++)); do
            local idx=$((current + i))
            if [ $idx -eq $needle_position ]; then
                # THE NEEDLE - unique searchable content
                echo "{\"path\":\"/doc/${idx}\",\"title\":\"NEEDLE_UNIQUE_XYZ_12345\",\"body\":\"This is the needle document with unique identifier MAGIC_HAYSTACK_FINDER\",\"category\":\"needle\",\"priority\":999}"
            else
                # Regular haystack document
                local cat_idx=$((idx % 10))
                local priority=$((idx % 100))
                echo "{\"path\":\"/doc/${idx}\",\"title\":\"Document number ${idx}\",\"body\":\"This is regular content for document ${idx} with some words\",\"category\":\"cat${cat_idx}\",\"priority\":${priority}}"
            fi
        done >> "$jsonl_file"
        current=$((current + batch_size))
        echo -ne "\r  Progress: $((current * 100 / count))%"
    done
    
    # Remainder
    for ((i=0; i<remainder; i++)); do
        local idx=$((current + i))
        if [ $idx -eq $needle_position ]; then
            echo "{\"path\":\"/doc/${idx}\",\"title\":\"NEEDLE_UNIQUE_XYZ_12345\",\"body\":\"This is the needle document with unique identifier MAGIC_HAYSTACK_FINDER\",\"category\":\"needle\",\"priority\":999}"
        else
            local cat_idx=$((idx % 10))
            local priority=$((idx % 100))
            echo "{\"path\":\"/doc/${idx}\",\"title\":\"Document number ${idx}\",\"body\":\"This is regular content for document ${idx} with some words\",\"category\":\"cat${cat_idx}\",\"priority\":${priority}}"
        fi
    done >> "$jsonl_file"
    
    echo ""
    success "Generated ${jsonl_file} ($(du -h "$jsonl_file" | cut -f1))"
    echo "$needle_position" > "${LOAD_DIR}/${name}.needle_pos"
}

# Create index and import corpus
create_and_import() {
    local name=$1
    local db_file="${LOAD_DIR}/${name}.db"
    local jsonl_file="${LOAD_DIR}/${name}.jsonl"
    local schema_file="${LOAD_DIR}/schema.json"
    
    # Create schema if not exists
    cat > "$schema_file" << 'EOF'
{
    "fields": {
        "title": {"type": "text", "weight": 3.0},
        "body": {"type": "text", "weight": 1.0},
        "category": {"type": "keyword"},
        "priority": {"type": "number"}
    }
}
EOF
    
    rm -f "$db_file"
    
    log "Creating index for ${name}..."
    $MINISTORE index create -i "$db_file" --schema "$schema_file"
    
    log "Importing corpus (this may take a while)..."
    local start_time=$(date +%s.%N)
    $MINISTORE put -i "$db_file" --import "$jsonl_file"
    local end_time=$(date +%s.%N)
    local import_time=$(echo "$end_time - $start_time" | bc)
    
    success "Import completed in ${import_time}s"
    echo "$import_time" > "${LOAD_DIR}/${name}.import_time"
    
    # Optimize index
    log "Optimizing index..."
    $MINISTORE index optimize -i "$db_file"
    success "Index size: $(du -h "$db_file" | cut -f1)"
}

# Run needle-in-haystack search benchmark
run_benchmark() {
    local name=$1
    local db_file="${LOAD_DIR}/${name}.db"
    local iterations=10
    
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "  BENCHMARK: ${name} corpus"
    echo "═══════════════════════════════════════════════════════════════"
    
    # Warmup
    log "Warming up (3 queries)..."
    for i in {1..3}; do
        $MINISTORE search -i "$db_file" -w "category:cat5" --limit 1 > /dev/null 2>&1
    done
    
    # Test 1: FTS search for needle (unique term)
    log "Test 1: FTS search for unique needle term..."
    local total=0
    for i in $(seq 1 $iterations); do
        local start=$(date +%s.%N)
        local result=$($MINISTORE search -i "$db_file" -w "NEEDLE_UNIQUE_XYZ_12345" --limit 5 --format json 2>/dev/null)
        local end=$(date +%s.%N)
        local elapsed=$(echo "($end - $start) * 1000" | bc)
        total=$(echo "$total + $elapsed" | bc)
    done
    local avg=$(echo "scale=3; $total / $iterations" | bc)
    success "FTS needle search: avg ${avg}ms over ${iterations} iterations"
    
    # Test 2: Keyword exact match
    log "Test 2: Keyword exact match for needle category..."
    total=0
    for i in $(seq 1 $iterations); do
        local start=$(date +%s.%N)
        $MINISTORE search -i "$db_file" -w "category:needle" --limit 5 > /dev/null 2>&1
        local end=$(date +%s.%N)
        local elapsed=$(echo "($end - $start) * 1000" | bc)
        total=$(echo "$total + $elapsed" | bc)
    done
    avg=$(echo "scale=3; $total / $iterations" | bc)
    success "Keyword needle search: avg ${avg}ms over ${iterations} iterations"
    
    # Test 3: Number comparison (priority = 999)
    log "Test 3: Number comparison for needle priority..."
    total=0
    for i in $(seq 1 $iterations); do
        local start=$(date +%s.%N)
        $MINISTORE search -i "$db_file" -w "priority>900" --limit 5 > /dev/null 2>&1
        local end=$(date +%s.%N)
        local elapsed=$(echo "($end - $start) * 1000" | bc)
        total=$(echo "$total + $elapsed" | bc)
    done
    avg=$(echo "scale=3; $total / $iterations" | bc)
    success "Number range needle search: avg ${avg}ms over ${iterations} iterations"
    
    # Test 4: Complex query (FTS + keyword filter)
    log "Test 4: Complex query (FTS + keyword)..."
    total=0
    for i in $(seq 1 $iterations); do
        local start=$(date +%s.%N)
        $MINISTORE search -i "$db_file" -w "MAGIC_HAYSTACK_FINDER category:needle" --limit 5 > /dev/null 2>&1
        local end=$(date +%s.%N)
        local elapsed=$(echo "($end - $start) * 1000" | bc)
        total=$(echo "$total + $elapsed" | bc)
    done
    avg=$(echo "scale=3; $total / $iterations" | bc)
    success "Complex needle search: avg ${avg}ms over ${iterations} iterations"
    
    # Test 5: Broad query (should return many results)
    log "Test 5: Broad query (category:cat5, expect ~10% of docs)..."
    total=0
    for i in $(seq 1 $iterations); do
        local start=$(date +%s.%N)
        $MINISTORE search -i "$db_file" -w "category:cat5" --limit 100 > /dev/null 2>&1
        local end=$(date +%s.%N)
        local elapsed=$(echo "($end - $start) * 1000" | bc)
        total=$(echo "$total + $elapsed" | bc)
    done
    avg=$(echo "scale=3; $total / $iterations" | bc)
    success "Broad search (100 results): avg ${avg}ms over ${iterations} iterations"
    
    echo ""
}

# Main
main() {
    local mode="${1:-100k}"
    
    echo ""
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║       MINISTORE LOAD TEST - NEEDLE IN A HAYSTACK              ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo ""
    
    case "$mode" in
        100k)
            if [ ! -f "${LOAD_DIR}/100k.jsonl" ]; then
                generate_corpus 100000 "100k"
            else
                log "Using existing 100k corpus"
            fi
            if [ ! -f "${LOAD_DIR}/100k.db" ]; then
                create_and_import "100k"
            else
                log "Using existing 100k index"
            fi
            run_benchmark "100k"
            ;;
        1m)
            if [ ! -f "${LOAD_DIR}/1m.jsonl" ]; then
                generate_corpus 1000000 "1m"
            else
                log "Using existing 1M corpus"
            fi
            if [ ! -f "${LOAD_DIR}/1m.db" ]; then
                create_and_import "1m"
            else
                log "Using existing 1M index"
            fi
            run_benchmark "1m"
            ;;
        both)
            $0 100k
            $0 1m
            ;;
        clean)
            log "Cleaning load test data..."
            rm -rf "$LOAD_DIR"
            success "Cleaned"
            ;;
        *)
            echo "Usage: $0 [100k|1m|both|clean]"
            echo ""
            echo "  100k  - Run with 100,000 documents"
            echo "  1m    - Run with 1,000,000 documents"
            echo "  both  - Run both 100k and 1M tests"
            echo "  clean - Remove generated data"
            exit 1
            ;;
    esac
    
    echo ""
    log "Load test complete!"
}

main "$@"
