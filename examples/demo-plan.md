# Checkout Cleanup Plan

Make the Acme Commerce checkout flow easier to understand and safer to change.

## Improvements

- Move checkout state into one shared model.
- Keep inventory checks in one service.
- Send confirmation emails only after payment succeeds.
- Replace scattered retry logic with one payment retry policy.

## Follow-up Questions

- Which checkout states should support agents see?
- What should happen when payment succeeds but inventory fails?
- Should abandoned carts keep their inventory reservation?

## Edge Cases

The current implementation handles *happy path* checkout well, but failure paths are inconsistent.

```text
payment authorized
inventory reserved
email queued
```

Reference: [checkout incident notes](https://example.com/incidents/checkout).
