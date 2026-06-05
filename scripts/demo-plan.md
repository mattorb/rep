# Checkout Cleanup Plan

Make the Acme Commerce checkout flow easier to understand.

## Improvements

- Move checkout state into one shared model.
- Keep inventory checks in one service.
- Send confirmation emails only after payment succeeds.

## Follow-up Questions

- Which checkout states should support agents see?
- What should happen when payment succeeds but inventory fails?
