---
url: 'https://github.com/drizzle-team/drizzle-orm/discussions/2093'
title: 'Postgres Table Partition Support (Discussion #2093)'
fetched_date: 2026-03-26
task_id: '0007'
---

# Postgres Table Partition Support (Discussion #2093)

## The Request

The original poster notes that "while you could query those partitions from Drizzle, a first-class support where you can also define partitions from Schema is nice."

## Community Support

- One commenter: "tens of millions of rows and adding partitioning out of the box would make deployment and configuration so much easier"
- Maintainer AlexBlokh acknowledged: "yeah, makes total sense"

## Status

Consolidated with issue #2854 as the formal tracking issue.

## Current Workaround

Manually edit generated SQL migration files — acknowledged as an imperfect temporary approach.
