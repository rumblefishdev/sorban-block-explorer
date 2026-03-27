# Brandur Leach: Building Robust Systems with Idempotency Keys

**Source:** https://brandur.org/idempotency-keys
**Fetched:** 2026-03-27

---

Published: Oct 27, 2017

## Overview

This comprehensive guide explains how to design robust API backends that handle failures gracefully using idempotency keys and atomic phases. The article uses a fictional "Rocket Rides" jetpack service as a practical example.

## Key Concepts

### Idempotency

"An idempotent endpoint is one that can be called any number of times while guaranteeing that the side effects will occur only once." This proves essential when clients and servers may crash mid-request.

### Idempotency Keys

An idempotency key is a unique client-generated value sent with API requests (typically via HTTP header) that allows servers to track and recover request state. When a client retries a failed request with the same key, the server can resume from where it left off rather than repeating operations.

### Foreign State Mutations

Operations that modify data outside your ACID database (API calls to Stripe, sending emails, DNS updates) present special challenges. Once such mutations occur, they cannot be rolled back within your own transaction system.

### Atomic Phases

"An atomic phase is a set of local state mutations that occur in transactions between foreign state mutations." These phases use ACID compliance to ensure all-or-nothing execution, creating safe recovery points.

## Schema Design

The article provides a complete PostgreSQL schema for tracking idempotency keys:

```sql
CREATE TABLE idempotency_keys (
    id              BIGSERIAL   PRIMARY KEY,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    idempotency_key TEXT        NOT NULL
        CHECK (char_length(idempotency_key) <= 100),
    last_run_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_at       TIMESTAMPTZ DEFAULT now(),
    request_method  TEXT        NOT NULL
        CHECK (char_length(request_method) <= 10),
    request_params  JSONB       NOT NULL,
    request_path    TEXT        NOT NULL
        CHECK (char_length(request_path) <= 100),
    response_code   INT         NULL,
    response_body   JSONB       NULL,
    recovery_point  TEXT        NOT NULL
        CHECK (char_length(recovery_point) <= 50),
    user_id         BIGINT      NOT NULL
);

CREATE UNIQUE INDEX idempotency_keys_user_id_idempotency_key
    ON idempotency_keys (user_id, idempotency_key);
```

## Rocket Rides Implementation

The example application demonstrates a ride-sharing service that charges users via Stripe. The request lifecycle includes:

1. Insert idempotency key record
2. Create ride record
3. Create audit record
4. Call Stripe to charge user
5. Update ride with charge ID
6. Send receipt email
7. Update idempotency key with results

### Atomic Phase Implementation

The author provides Ruby code demonstrating how to wrap operations in transactions with three possible outcomes:

- `RecoveryPoint`: Sets new recovery point, execution continues
- `Response`: Completes request, returns response to client
- `NoOp`: Continues without state changes

```ruby
def atomic_phase(key, &block)
  error = false
  begin
    DB.transaction(isolation: :serializable) do
      ret = block.call

      if ret.is_a?(NoOp) || ret.is_a?(RecoveryPoint) || ret.is_a?(Response)
        ret.call(key)
      else
        raise "Blocks to #atomic_phase should return one of NoOp, RecoveryPoint, or Response"
      end
    end
  rescue Sequel::SerializationFailure
    error = true
    halt 409, JSON.generate(wrap_error(Messages.error_retry))
  rescue
    error = true
    halt 500, JSON.generate(wrap_error(Messages.error_internal))
  ensure
    if error && !key.nil?
      begin
        key.update(locked_at: nil)
      rescue StandardError
        puts "Failed to unlock key #{key.id}."
      end
    end
  end
end
```

## Supporting Processes

The implementation requires three background processes:

**The Enqueuer**: Moves staged jobs from the database to the job queue after transaction commit.

**The Completer**: Finds abandoned requests and pushes them to completion, ensuring clients don't disappear mid-process.

**The Reaper**: Deletes old idempotency keys (suggested threshold: 72 hours) to prevent indefinite storage while maintaining debugging capability.

## Failure Scenarios

The article illustrates how the system handles various failures:

- Connection breaks before request reaches backend: Client retries safely
- Concurrent requests create same key: Database constraint ensures only one succeeds
- Database downtime: Client retries until online; request resumes from recovery point
- External service (Stripe) downtime: Requests retry until service returns
- Server process dies during foreign call: Idempotency key on foreign service prevents double-charging
- Bad deployment mid-request: Completer process finishes abandoned requests after fix deploys

## Limitations

**Non-idempotent foreign mutations**: Services lacking idempotency keys require conservative failure handling—indeterminate errors must be marked as permanent failures.

**Non-ACID databases**: Implementations like MongoDB cannot guarantee atomic phases, making every database operation equivalent to a foreign state mutation.

## Beyond APIs

The technique applies to web forms: hidden input fields containing idempotency keys prevent double-submission charges from multiple clicks.

## Conclusion

The article emphasizes "passive safety"—designing systems that reach stable states despite failures, requiring minimal operator intervention. This combines purely idempotent transactions with strategic use of idempotency keys and atomic phases to build resilient backends.

**Source Code**: A complete working implementation is available in the [Atomic Rocket Rides repository](https://github.com/brandur/rocket-rides-atomic) with tests and all supporting processes included.
