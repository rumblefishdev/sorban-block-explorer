---
id: 'NNNN'
title: 'Short descriptive title'
status: proposed # proposed | accepted | deprecated | superseded
deciders: [team-member]
related_tasks: []
related_adrs: []
tags: []
links: []
history:
  - date: YYYY-MM-DD
    status: proposed
    who: team-member
    note: 'ADR created'
---

# ADR NNNN: [Title]

**Related:**

- [Task NNNN: Description](../1-tasks/active/NNNN_TYPE_slug.md)

---

## Context

What is the issue that we're seeing that motivates this decision or change?

---

## Decision

What is the change that we're proposing and/or doing?

---

## Rationale

Why is this the best choice among the alternatives?

---

## Alternatives Considered

### Alternative 1: [Name]

**Description:** Brief explanation.

**Pros:**

- ...

**Cons:**

- ...

**Decision:** REJECTED - [reason]

---

## Consequences

### Positive

- What becomes easier?

### Negative

- What becomes harder?

---

## Delivery Checklist

Per [ADR 0032](./0032_docs-architecture-evergreen-maintenance.md),
any ADR that changes the shape of the system MUST be landed together with the
corresponding updates to `docs/architecture/**`. Tick each that applies before
marking the ADR `accepted`:

- [ ] `docs/architecture/technical-design-general-overview.md` updated (or N/A)
- [ ] `docs/architecture/database-schema/database-schema-overview.md` updated (or N/A)
- [ ] `docs/architecture/backend/backend-overview.md` updated (or N/A)
- [ ] `docs/architecture/frontend/frontend-overview.md` updated (or N/A)
- [ ] `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` updated (or N/A)
- [ ] `docs/architecture/infrastructure/infrastructure-overview.md` updated (or N/A)
- [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` updated (or N/A)
- [ ] This ADR is linked from each updated doc at the relevant section

Mark every unchecked box with `N/A — reason` so the reviewer can verify the
"no docs changed" claim. A pure policy or process ADR (e.g. an ADR about how we
run CI) can legitimately mark all boxes N/A.

---

## References

- [External Link](https://example.com) - Description
