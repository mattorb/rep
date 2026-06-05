# Security Policy

## Reporting

Report security issues privately to the repository owner. Do not open a public issue for a suspected vulnerability.

## Local Data Access

`rep` reads the markdown file path supplied on the command line and may read local terminal/tmux state while launching fallback windows. It does not intentionally scan unrelated project files.

## Clipboard and Terminal Escape Behavior

`rep` can copy emitted action blocks through OSC52 terminal escape sequences or OS clipboard commands such as `pbcopy`, `wl-copy`, `xclip`, `xsel`, or `clip`. Clipboard contents may be visible to your terminal emulator, multiplexer, OS clipboard manager, or remote shell environment.

## Temporary Bridge Files

When `rep` falls back to a tmux pane or local terminal window, it creates temporary bridge files under the system temp directory to pass process status and output back to the original invocation. These files may contain emitted action text and source context snippets.

## Agent Handoff

The bundled agent skill captures `rep` output to a temporary file and instructs an agent to apply the emitted action blocks. Review the emitted output before using it with sensitive plans, because annotations can include surrounding source context.
