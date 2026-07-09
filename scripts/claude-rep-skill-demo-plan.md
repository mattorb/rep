# Checkout Recovery Rollout Plan

Ship a resilient checkout recovery flow for customers whose payment succeeds but whose cart finalization fails.

## Goals

- Recover failed cart finalizations within five minutes without double-charging customers.
- Give support agents a clear customer-safe status for every recovered checkout.
- Keep the rollout reversible while payment and inventory edge cases are measured.

## Implementation

1. Introduce a `CheckoutSession` state machine shared by the checkout API, webhook consumer, and support dashboard.
2. Move payment webhook handling into a `PaymentRecoveryWorker` that retries pending captures once per minute.
3. Backfill existing abandoned carts into the new session table before enabling writes.
4. Update support dashboard copy after backend APIs are stable.
5. Launch behind `checkout_recovery_v2` at 10% of checkout traffic.

## Validation

- Add unit tests for duplicate webhook delivery, stale inventory reservations, and expired carts.
- Add an end-to-end test that simulates payment success followed by cart finalization failure.
- Run a one-day shadow job against production events and compare recovered cart counts with current manual triage.

## Rollback

- Keep the existing finalization path available behind the old feature flag.
- Manual triage is enough if the first launch exposes a serious mismatch.

## Risks

- Stripe webhook ordering can produce duplicate recovery attempts.
- Support tooling may expose internal status names to agents.
- Inventory reservation expiry can turn a successfully paid cart into a partial fulfillment case.
