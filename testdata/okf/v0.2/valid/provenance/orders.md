---
type: BigQuery Table
title: Customer Orders
description: One row per completed customer order.
resource: https://example.test/datasets/sales/tables/orders
tags: [sales, orders]
status: stable
stale_after: 2027-01-31
generated:
  by: reference_agent/gemini-2.5-pro
  at: 2026-06-20T22:53:05Z
verified:
  - by: process:nightly-catalog-check
    at: 2026-06-21T02:00:00Z
  - by: human:catalog-owner
    at: 2026-06-25T09:00:00Z
usage_window:
  start: 2026-01-01
  end: 2026-06-30
sources:
  - id: warehouse
    resource: https://example.test/datasets/sales/tables/orders
    author: data-platform
    usage_count: 1200
    last_modified: 2026-06-19T12:00:00Z
---
# Customer Orders

The warehouse table is the system of record.[^warehouse]

[^warehouse]: Data platform catalog entry.

