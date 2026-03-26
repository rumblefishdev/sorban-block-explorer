---
url: 'https://www.postgresql.org/docs/current/ddl-partitioning.html'
title: 'PostgreSQL — Table Partitioning'
fetched_date: 2026-03-26
task_id: '0007'
---

# PostgreSQL — Table Partitioning

## Range Partitioning Syntax

```sql
CREATE TABLE measurement (
    city_id     int not null,
    logdate     date not null,
    peaktemp    int,
    unitsales   int
) PARTITION BY RANGE (logdate);
```

## Creating Partitions

```sql
CREATE TABLE measurement_y2006m02 PARTITION OF measurement
    FOR VALUES FROM ('2006-02-01') TO ('2006-03-01');

CREATE TABLE measurement_y2006m03 PARTITION OF measurement
    FOR VALUES FROM ('2006-03-01') TO ('2006-04-01');
```

Upper bounds are exclusive. Adjacent partitions can share a bound value.

## Foreign Keys on Partitioned Tables

**Key limitation:** Unique/primary key constraints must include all partition key columns. This means single-column FK references may be impossible if the referenced column alone is not unique.

Example composite unique:

```sql
ALTER TABLE ONLY measurement ADD UNIQUE (city_id, logdate);
```

## Adding New Partitions

**Direct:**

```sql
CREATE TABLE measurement_y2008m02 PARTITION OF measurement
    FOR VALUES FROM ('2008-02-01') TO ('2008-03-01');
```

**Create then attach (avoids lock):**

```sql
CREATE TABLE measurement_y2008m02
  (LIKE measurement INCLUDING DEFAULTS INCLUDING CONSTRAINTS);

ALTER TABLE measurement_y2008m02 ADD CONSTRAINT y2008m02
   CHECK (logdate >= DATE '2008-02-01' AND logdate < DATE '2008-03-01');

ALTER TABLE measurement ATTACH PARTITION measurement_y2008m02
    FOR VALUES FROM ('2008-02-01') TO ('2008-03-01');
```

## Removing Partitions

```sql
-- Quick drop
DROP TABLE measurement_y2006m02;

-- Detach (keep data)
ALTER TABLE measurement DETACH PARTITION measurement_y2006m02;

-- Detach with lower lock (CONCURRENTLY)
ALTER TABLE measurement DETACH PARTITION measurement_y2006m02 CONCURRENTLY;
```

## Sub-Partitioning

```sql
CREATE TABLE measurement_y2006m02 PARTITION OF measurement
    FOR VALUES FROM ('2006-02-01') TO ('2006-03-01')
    PARTITION BY RANGE (peaktemp);
```
