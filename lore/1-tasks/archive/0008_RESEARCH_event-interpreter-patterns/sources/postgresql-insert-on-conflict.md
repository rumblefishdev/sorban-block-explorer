# PostgreSQL: INSERT ... ON CONFLICT

**Source:** https://www.postgresql.org/docs/current/sql-insert.html#SQL-ON-CONFLICT
**Fetched:** 2026-03-27

---

## Syntax

```sql
[ ON CONFLICT [ conflict_target ] conflict_action ]

where conflict_target can be one of:

    ( { index_column_name | ( index_expression ) } [ COLLATE collation ] [ opclass ] [, ...] ) [ WHERE index_predicate ]
    ON CONSTRAINT constraint_name

and conflict_action is one of:

    DO NOTHING
    DO UPDATE SET { column_name = { expression | DEFAULT } |
                    ( column_name [, ...] ) = [ ROW ] ( { expression | DEFAULT } [, ...] ) |
                    ( column_name [, ...] ) = ( sub-SELECT )
                  } [, ...]
              [ WHERE condition ]
```

## Description

The optional `ON CONFLICT` clause specifies an alternative action to raising a unique violation or exclusion constraint violation error. For each individual row proposed for insertion, either the insertion proceeds, or, if an _arbiter_ constraint or index specified by `conflict_target` is violated, the alternative `conflict_action` is taken.

- **`ON CONFLICT DO NOTHING`** — Simply avoids inserting a row as its alternative action.
- **`ON CONFLICT DO UPDATE`** — Updates the existing row that conflicts with the row proposed for insertion as its alternative action.

`ON CONFLICT DO UPDATE` guarantees an atomic `INSERT` or `UPDATE` outcome; provided there is no independent error, one of those two outcomes is guaranteed, even under high concurrency. This is also known as _UPSERT_ — "UPDATE or INSERT".

## conflict_target Parameters

### Unique Index Inference

`conflict_target` can perform _unique index inference_. When performing inference, it consists of one or more `index_column_name` columns and/or `index_expression` expressions, and an optional `index_predicate`. All `table_name` unique indexes that, without regard to order, contain exactly the `conflict_target`-specified columns/expressions are inferred (chosen) as arbiter indexes. If an `index_predicate` is specified, it must, as a further requirement for inference, satisfy arbiter indexes.

A non-partial unique index (a unique index without a predicate) will be inferred if such an index satisfying every other criteria is available. If an attempt at inference is unsuccessful, an error is raised.

### Parameters

**`index_column_name`**

The name of a `table_name` column. Used to infer arbiter indexes. Follows `CREATE INDEX` format. `SELECT` privilege on `index_column_name` is required.

**`index_expression`**

Similar to `index_column_name`, but used to infer expressions on `table_name` columns appearing within index definitions (not simple columns). Follows `CREATE INDEX` format. `SELECT` privilege on any column appearing within `index_expression` is required.

**`collation`**

When specified, mandates that corresponding `index_column_name` or `index_expression` use a particular collation in order to be matched during inference. Typically omitted, as collations usually do not affect whether or not a constraint violation occurs.

**`opclass`**

When specified, mandates that corresponding `index_column_name` or `index_expression` use a particular operator class in order to be matched during inference. Typically omitted, as equality semantics are often equivalent across a type's operator classes.

**`index_predicate`**

Used to allow inference of partial unique indexes. Any indexes that satisfy the predicate (which need not actually be partial indexes) can be inferred. Follows `CREATE INDEX` format. `SELECT` privilege on any column appearing within `index_predicate` is required.

**`constraint_name`**

Explicitly specifies an arbiter _constraint_ by name, rather than inferring a constraint or index.

## conflict_action Parameters

**`DO NOTHING`**

Specifies that if a conflict occurs, the row should not be inserted.

**`DO UPDATE`** with `SET` clause specifies the exact details of the `UPDATE` action to be performed in case of a conflict:

- **`column_name = { expression | DEFAULT }`** — Updates a single column
- **`( column_name [, ...] ) = [ ROW ] ( { expression | DEFAULT } [, ...] )`** — Updates multiple columns
- **`( column_name [, ...] ) = ( sub-SELECT )`** — Updates multiple columns from a subquery

The `SET` and `WHERE` clauses in `ON CONFLICT DO UPDATE` have access to:

- The existing row using the table's name (or an alias)
- The row proposed for insertion using the special `excluded` table

**`condition`**

An expression that returns a value of type `boolean`. Only rows for which this expression returns `true` will be updated, although all rows will be locked when the `ON CONFLICT DO UPDATE` action is taken. The `condition` is evaluated last, after a conflict has been identified as a candidate to update.

## The EXCLUDED Table

The special `excluded` table represents the row proposed for insertion. It can be referenced in `DO UPDATE` clauses to access the values that were originally proposed for insertion.

The effects of all per-row `BEFORE INSERT` triggers are reflected in `excluded` values, since those effects may have contributed to the row being excluded from insertion.

`SELECT` privilege is required on any column in the target table where corresponding `excluded` columns are read.

## Important Notes and Caveats

1. **Deterministic Statement**: `INSERT ... ON CONFLICT DO UPDATE` is a "deterministic" statement. The command will not be allowed to affect any single existing row more than once; a cardinality violation error will be raised. Rows proposed for insertion should not duplicate each other in terms of attributes constrained by an arbiter index or constraint.

2. **Exclusion Constraints Not Supported**: Exclusion constraints are not supported as arbiters with `ON CONFLICT DO UPDATE`. Only `NOT DEFERRABLE` constraints and unique indexes are supported as arbiters.

3. **Partitioned Tables**: It is currently not supported for the `ON CONFLICT DO UPDATE` clause of an `INSERT` applied to a partitioned table to update the partition key of a conflicting row such that it requires the row be moved to a new partition.

4. **Concurrent Index Creation Warning**: While `CREATE INDEX CONCURRENTLY` or `REINDEX CONCURRENTLY` is running on a unique index, `INSERT ... ON CONFLICT` statements on the same table may unexpectedly fail with a unique violation.

5. **Optional conflict_target for DO NOTHING**: For `ON CONFLICT DO NOTHING`, specifying a `conflict_target` is optional; when omitted, conflicts with all usable constraints (and unique indexes) are handled. For `ON CONFLICT DO UPDATE`, a `conflict_target` _must_ be provided.

6. **Inference Preferred Over Named Constraints**: It is often preferable to use unique index inference rather than naming a constraint directly via `ON CONFLICT ON CONSTRAINT`. Inference will continue to work correctly when the underlying index is replaced by another equivalent index.

## Examples

### DO UPDATE with EXCLUDED

```sql
INSERT INTO distributors (did, dname)
    VALUES (5, 'Gizmo Transglobal'), (6, 'Associated Computing, Inc')
    ON CONFLICT (did) DO UPDATE SET dname = EXCLUDED.dname;
```

### DO NOTHING

```sql
INSERT INTO distributors (did, dname) VALUES (7, 'Redline GmbH')
    ON CONFLICT (did) DO NOTHING;
```

### DO UPDATE with WHERE Clause (Conditional Upsert)

```sql
INSERT INTO distributors AS d (did, dname) VALUES (8, 'Anvil Distribution')
    ON CONFLICT (did) DO UPDATE
    SET dname = EXCLUDED.dname || ' (formerly ' || d.dname || ')'
    WHERE d.zipcode <> '21201';
```

### ON CONSTRAINT (Named Constraint)

```sql
INSERT INTO distributors (did, dname) VALUES (9, 'Antwerp Design')
    ON CONFLICT ON CONSTRAINT distributors_pkey DO NOTHING;
```

### Partial Index Inference with WHERE

```sql
INSERT INTO distributors (did, dname) VALUES (10, 'Conrad International')
    ON CONFLICT (did) WHERE is_active DO NOTHING;
```

### DO UPDATE with RETURNING (OLD and NEW values)

```sql
INSERT INTO distributors (did, dname)
    VALUES (5, 'Gizmo Transglobal'), (6, 'Associated Computing, Inc')
    ON CONFLICT (did) DO UPDATE SET dname = EXCLUDED.dname
    RETURNING old.did AS old_did, old.dname AS old_dname,
              new.did AS new_did, new.dname AS new_dname;
```
