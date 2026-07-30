# Release checklist

Run this checklist from a clean `html_and_browser_support` checkout before
merging or tagging a release. Record the OS and browser versions beside each
manual result.

## Automated gate

- [ ] `mise install`
- [ ] `mise exec -- npm --prefix web ci`
- [ ] `mise exec -- ./build.sh`
- [ ] `mise exec -- npm --prefix web test`
- [ ] `mise exec -- npm --prefix web run test:e2e`
- [ ] `mise exec -- cargo audit`
- [ ] `RUSTDOCFLAGS="-D warnings" mise exec -- cargo doc --no-deps`
- [ ] CI Chromium job passes and its gallery artifact has been inspected.
- [ ] Both Linux MUSL archive/installer/web smoke jobs pass.
- [ ] Native macOS tests and both macOS target builds pass.

## Markdown compatibility

- [ ] Open `examples/demo-plan.md` in the TUI.
- [ ] Exercise movement, all five selection units, search, structure, links,
      annotations, copy, submit, and silent discard.
- [ ] Confirm submitted output matches the established Markdown protocol and
      contains no `FORMAT` or `LOCATOR` line.
- [ ] Invoke the bundled skill and confirm it launches the Markdown TUI, waits
      for the fresh capture, and applies only those actions.

## HTML browser matrix

Repeat this section in current Safari on macOS and current Firefox on macOS or
Linux. Chromium is covered by the automated gate.

- [ ] Open `rep --web examples/demo-plan.html`; record browser and OS versions.
- [ ] Confirm the original serif typography, two-column desktop layout,
      responsive single-column layout, colors, spacing, and inline styles.
- [ ] Confirm linked local CSS, images, and fonts render in the acceptance
      fixture while remote/root-relative assets are visibly reported blocked.
- [ ] Confirm plan scripts and inline event handlers do not execute; forms,
      frames, navigation, and downloads remain blocked.
- [ ] Exercise keyboard-only movement, all five units, search, annotation
      jumps, help, outline, links, copy, `q` confirmation, and silent discard.
- [ ] Exercise single-, double-, and triple-click selection.
- [ ] Add, edit, and clear change, feedback, before, after, and delete actions
      across the five selection units.
- [ ] Resize and scroll; confirm selection and annotation overlays stay aligned
      without changing the plan DOM layout.
- [ ] Confirm the active selection fill, border, halo, and leading focus bar
      remain legible over both light and dark plan colors, while annotation
      patterns and badges remain distinguishable.
- [ ] Reload, close/reopen the same URL, and confirm session state is retained.
- [ ] Press `q`, confirm the handoff screen appears, and inspect `WHERE`,
      `LOCATOR`, target/context, ordering, and payloads; confirm the listener
      closes.
- [ ] Invoke the bundled skill for `.html` and `.HTM`; confirm the browser
      opens in an isolated profile, the agent waits, and `q` returns fresh
      actions to the agent. Confirm the temporary browser closes without
      manual app switching before the original HTML is updated, and that
      structure and inline markup remain intact.

For a repeatable engine-level preview before the manual pass:

```sh
REP_BROWSER=firefox mise exec -- npm --prefix web run test:e2e
REP_BROWSER=webkit mise exec -- npm --prefix web run test:e2e
```

Playwright WebKit is a compatibility signal, not a substitute for the Safari
manual result.

## Failure and lifecycle paths

- [ ] Direct browser opener failure leaves a usable manual URL.
- [ ] Skill browser preflight fails clearly when no supported executable is
      available and never starts an orphaned Rep server.
- [ ] Wrong format/flag combinations and missing, non-UTF-8, and oversized
      files fail before server launch.
- [ ] Bad token, Host, Origin, method, content type, MIME, and request size fail
      closed.
- [ ] Traversal and symlink asset escapes fail closed.
- [ ] Malformed/conflicting manifests fail without replacing active state.
- [ ] SIGINT/SIGTERM, injected server failure, and inactivity timeout exit
      non-zero without partial action output.
- [ ] A tab reload does not create duplicate output, and the first terminal
      finish/discard action wins.

## Packaged release

- [ ] Inspect each archive: only `rep`, `LICENSE`, `README.md`, and
      `.agents/skills/rep` are present.
- [ ] Run archive smoke tests in clean x86_64 and arm64 Alpine containers.
- [ ] Confirm `rep --help` documents `--web` and `--no-open`.
- [ ] Confirm the packaged binary serves embedded shell assets and can finish
      `--web --no-open` without Node or a source checkout.
- [ ] Confirm the packaged skill routes HTML and Markdown correctly.
- [ ] Confirm `ldd` reports the Linux MUSL binary as static/not dynamically
      linked.
- [ ] Run the installer and skills-only installer smoke paths on a clean home.
