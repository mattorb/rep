# Remote Control Notes

This fixture gives the word-click regression test a stable Markdown document
with enough wrapping behavior to cover words near punctuation, links, and inline
formatting without depending on a developer-local checkout.

The interface should keep selecting the intended **visible word** even when a
click lands on the trailing space after that word. It also needs to handle
`inline code`, [linked text](https://example.com/remote-control), and prose that
wraps across several terminal rows at eighty columns.

## Checklist

- Verify short list items.
- Verify longer list items that wrap across the test backend width and include
  commas, parentheses, and emphasized phrases in the middle of the row.
- Verify final words at row boundaries.

> Quoted text is present so the renderer sees block-level structure while the
> test filters down to paragraph nodes.
