#!/bin/bash
set -e

echo "========================================="
echo "Ministore Patch Verification E2E Suite"
echo "========================================="
echo

# Cleanup
rm -rf /tmp/ministore-verify.db
INDEX="/tmp/ministore-verify.db"
CLI="./target/release/ministore"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

pass() {
    echo -e "${GREEN}✓${NC} $1"
}

fail() {
    echo -e "${RED}✗${NC} $1"
    exit 1
}

info() {
    echo -e "${BLUE}➜${NC} $1"
}

# 1. CREATE INDEX
info "Creating index with comprehensive schema..."
cat > /tmp/schema.json <<'EOF'
{
  "fields": {
    "title": { "type": "text", "weight": 2.0 },
    "body": { "type": "text", "weight": 1.0 },
    "tags": { "type": "keyword", "multi": true },
    "category": { "type": "keyword" },
    "priority": { "type": "number" },
    "score": { "type": "number" },
    "due": { "type": "date" },
    "published": { "type": "date", "multi": true },
    "active": { "type": "bool" }
  }
}
EOF

$CLI index create --index $INDEX --schema /tmp/schema.json || fail "Index creation failed"
pass "Index created"

# 2. INSERT TEST DATA
info "Inserting diverse test data..."

# Insert items one by one (JSONL format - one JSON object per line without commas)
$CLI put --index $INDEX --json <<'EOF'
{"path": "/docs/rust-guide", "title": "Rust Programming Guide", "body": "Learn Rust programming with examples", "tags": ["rust", "programming", "tutorial"], "category": "technical", "priority": 5, "score": 95.5, "due": "2024-03-15", "active": true}
EOF

$CLI put --index $INDEX --json <<'EOF'
{"path": "/docs/python-basics", "title": "Python Basics", "body": "Introduction to Python programming", "tags": ["python", "programming"], "category": "technical", "priority": 3, "score": 88.0, "active": true}
EOF

$CLI put --index $INDEX --json <<'EOF'
{"path": "/notes/meeting-2024", "title": "Team Meeting Notes", "body": "Discussed project roadmap and milestones", "tags": ["meeting", "planning"], "category": "notes", "priority": 2, "score": 70.0, "due": "2024-02-01", "active": false}
EOF

$CLI put --index $INDEX --json <<'EOF'
{"path": "/blog/search-engines", "title": "Building Search Engines", "body": "How to build efficient search systems", "tags": ["search", "engineering"], "category": "blog", "priority": 4, "score": 92.0, "published": ["2024-01-15", "2024-01-20"], "active": true}
EOF

$CLI put --index $INDEX --json <<'EOF'
{"path": "/drafts/archived-post", "title": "Old Draft", "body": "This is archived content", "tags": ["draft"], "category": "archive", "priority": 1, "score": 50.0, "active": false}
EOF
pass "Test data inserted"

# 3. BASIC QUERIES
info "Testing basic predicates..."

# has: predicate
RESULT=$($CLI search --index $INDEX --where 'has:title' --format json | jq -r '.items | length')
[ "$RESULT" -eq 5 ] || fail "has:title should return 5 items, got $RESULT"
pass "has: predicate works"

# keyword exact match
RESULT=$($CLI search --index $INDEX --where 'tags:rust' --format json | jq -r '.items | length')
[ "$RESULT" -eq 1 ] || fail "tags:rust should return 1 item, got $RESULT"
pass "keyword exact match works"

# keyword prefix
RESULT=$($CLI search --index $INDEX --where 'tags:prog*' --format json | jq -r '.items | length')
[ "$RESULT" -eq 2 ] || fail "tags:prog* should return 2 items, got $RESULT"
pass "keyword prefix match works"

# text search
RESULT=$($CLI search --index $INDEX --where 'programming' --format json | jq -r '.items | length')
[ "$RESULT" -ge 2 ] || fail "text search 'programming' should return at least 2 items, got $RESULT"
pass "text search works"

# bool predicate
RESULT=$($CLI search --index $INDEX --where 'active:true' --format json | jq -r '.items | length')
[ "$RESULT" -eq 3 ] || fail "active:true should return 3 items, got $RESULT"
pass "bool predicate works"

# 4. MULTI-PREDICATE QUERIES (tests parameter binding fix)
info "Testing multi-predicate queries (critical for Patch 4)..."

# AND query
RESULT=$($CLI search --index $INDEX --where 'tags:programming AND active:true' --format json | jq -r '.items | length')
[ "$RESULT" -eq 2 ] || fail "tags:programming AND active:true should return 2 items, got $RESULT"
pass "AND query with multiple predicates works"

# OR query  
RESULT=$($CLI search --index $INDEX --where 'tags:rust OR tags:python' --format json | jq -r '.items | length')
[ "$RESULT" -eq 2 ] || fail "tags:rust OR tags:python should return 2 items, got $RESULT"
pass "OR query works"

# NOT query
RESULT=$($CLI search --index $INDEX --where 'has:title AND NOT active:false' --format json | jq -r '.items | length')
[ "$RESULT" -eq 3 ] || fail "has:title AND NOT active:false should return 3 items, got $RESULT"
pass "NOT query works"

# Complex query with 3+ predicates
RESULT=$($CLI search --index $INDEX --where 'category:technical AND active:true AND tags:programming' --format json | jq -r '.items | length')
[ "$RESULT" -eq 2 ] || fail "3-predicate query should return 2 items, got $RESULT"
pass "Complex multi-predicate query works"

# 5. PATH QUERIES
info "Testing path predicates..."

RESULT=$($CLI search --index $INDEX --where 'path:/docs/*' --format json | jq -r '.items | length')
[ "$RESULT" -eq 2 ] || fail "path:/docs/* should return 2 items, got $RESULT"
pass "path glob works"

# 6. RANKING
info "Testing ranking modes..."

# Recency ranking
FIRST_PATH=$($CLI search --index $INDEX --where 'has:title' --rank recency --format json | jq -r '.items[0].path')
[ -n "$FIRST_PATH" ] || fail "Recency ranking returned no results"
pass "Recency ranking works"

# Field ranking (requires Patch 4)
FIRST_PATH=$($CLI search --index $INDEX --where 'has:score' --rank 'field:score' --format json | jq -r '.items[0].path')
[ "$FIRST_PATH" = "/docs/rust-guide" ] || fail "Field ranking by score should return rust-guide first, got $FIRST_PATH"
pass "Field ranking works (Patch 4)"

# 7. COUNT
info "Testing count..."

COUNT=$($CLI count --index $INDEX --where 'tags:programming')
[ "$COUNT" -eq 2 ] || fail "count tags:programming should return 2, got $COUNT"
pass "Count works"

# 8. DISCOVER
info "Testing discover..."

# Discover fields
FIELD_COUNT=$($CLI discover fields --index $INDEX --format json | jq '.fields | length')
[ "$FIELD_COUNT" -ge 9 ] || fail "discover fields should show at least 9 fields, got $FIELD_COUNT"
pass "Discover fields works"

# Discover values
TAG_COUNT=$($CLI discover values --index $INDEX --field tags --format json | jq '.values | length')
[ "$TAG_COUNT" -ge 5 ] || fail "discover values for tags should show at least 5 values, got $TAG_COUNT"
pass "Discover values works"

# Discover with query filter
RESULT=$($CLI discover values --index $INDEX --field tags --where 'active:true' --format json | jq '.values | length')
[ "$RESULT" -ge 3 ] || fail "discover values with filter should work, got $RESULT values"
pass "Discover with query filter works"

# 9. STATS
info "Testing stats..."

STATS=$($CLI stats --index $INDEX --field priority --format json)
AVG=$(echo "$STATS" | jq -r '.avg')
[ -n "$AVG" ] && [ "$(echo "$AVG > 0" | bc)" -eq 1 ] || fail "stats should return valid average"
pass "Stats works"

# Stats with filter
STATS=$($CLI stats --index $INDEX --field score --where 'active:true' --format json)
COUNT=$(echo "$STATS" | jq -r '.count')
[ "$COUNT" -eq 3 ] || fail "filtered stats should count 3 items, got $COUNT"
pass "Stats with filter works"

# 10. UPDATE (tests doc_freq correctness from Patch 1)
info "Testing update (doc_freq correctness)..."

# Update an existing item
$CLI put --index $INDEX --path /docs/rust-guide --set 'tags=["rust","advanced","tutorial"]' || fail "Update failed"

# Verify the update
RESULT=$($CLI search --index $INDEX --where 'tags:advanced' --format json | jq -r '.items | length')
[ "$RESULT" -eq 1 ] || fail "Updated tag should be searchable, got $RESULT"
pass "Update works (doc_freq correct per Patch 1)"

# Verify old tag still works for other docs
RESULT=$($CLI search --index $INDEX --where 'tags:programming' --format json | jq -r '.items | length')
[ "$RESULT" -eq 1 ] || fail "Old tag should still work for other docs, got $RESULT"
pass "doc_freq maintained correctly after update"

# 11. DELETE
info "Testing delete..."

$CLI delete --index $INDEX --path /drafts/archived-post || fail "Delete by path failed"

RESULT=$($CLI search --index $INDEX --where 'category:archive' --format json | jq -r '.items | length')
[ "$RESULT" -eq 0 ] || fail "Deleted item should not be found, got $RESULT"
pass "Delete works"

# 12. DELETE WHERE
info "Testing delete where..."

$CLI delete --index $INDEX --where 'active:false' || fail "Delete where failed"

RESULT=$($CLI count --index $INDEX --where 'active:false')
[ "$RESULT" -eq 0 ] || fail "Items matching delete where should be gone, got $RESULT"
pass "Delete where works"

# 13. PAGINATION (basic test - won't work until Patches 5-6 applied)
info "Testing pagination..."

RESULT=$($CLI search --index $INDEX --where 'has:title' --limit 2 --format json)
ITEM_COUNT=$(echo "$RESULT" | jq -r '.items | length')
HAS_MORE=$(echo "$RESULT" | jq -r '.has_more')
[ "$ITEM_COUNT" -eq 2 ] || fail "Limit 2 should return 2 items, got $ITEM_COUNT"
pass "Basic pagination limit works"

echo
echo -e "${GREEN}=========================================${NC}"
echo -e "${GREEN}All verification tests passed!${NC}"
echo -e "${GREEN}=========================================${NC}"
