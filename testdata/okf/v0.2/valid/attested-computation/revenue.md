---
type: Attested Computation
title: Revenue by fiscal year
status: stable
runtime: postgres
parameters:
  - name: year
    type: integer
    required: true
executor:
  resource: runbook.md
  receipt: [query_id, executed_sql, result]
attester:
  resource: attest.py
generated: {by: process:catalog-build, at: 2026-06-20T22:53:05Z}
verified: {by: human:finance-owner, at: 2026-06-25T09:00:00Z}
---
# Computation

```sql
SELECT SUM(amount) FROM orders WHERE fiscal_year = :year
```

