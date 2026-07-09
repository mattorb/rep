[![CI](https://github.com/mattorb/rep/actions/workflows/ci.yml/badge.svg)](https://github.com/mattorb/rep/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

# Rep

A human in the loop TUI to review and revise LLM generated markdown plan files quickly. 

It's best to wrap your agent harness in a tmux session to allow the skill to launch the rep UI modally (see demo below).

![Rep Claude skill demo](docs/rep-claude-skill-demo.gif)

[Watch the smaller MP4 version](docs/rep-claude-skill-demo.mp4).

## Overview

`rep` opens a markdown file in an interactive TUI optimized for providing feedback and requesting changes from an LLM. 

When you exit the [rep] app, it prints out a list a [changes requested](#emitted-annotations-example), for an AI agent to process and revise the markdown plan.  These changes can be a mix of deletions, additions, changes, and intent guidance.

For the most seamless experience, launch Codex or Claude Code *inside a tmux session*, which allows rep to launch as modal from a skill and automatically pass the revisions you ask it for back into the agentic loop.

## Installation
Install rep to ~/.local/bin, and the bundled agent skill to ~/.agents/skills/rep with this command:

```sh
curl -fsSL https://raw.githubusercontent.com/mattorb/rep/main/install.sh | sh
```

## Usage

### Preferred
Use the rep skill to launch in the agentic loop: `/rep` in Claude Code or `$rep` in Codex.

### Fallback Manually run
If you execute it manually like this, you'll have to copy and paste results back to agent.
```sh
rep plan.md
```

## Keybindings

| Key | Action |
| --- | --- |
| `j`, `Down`, `Right` | Move to the next active unit |
| `k`, `Up`, `Left` | Move to the previous active unit |
| `Space` | Cycle to the next selection unit |
| `Backspace` | Cycle to the previous selection unit |
| `i` | Use a finer selection unit |
| `o` | Use a coarser selection unit |
| `c` | Add or edit a literal change request |
| `f` | Add or edit feedback or intent |
| `b` | Insert text before the current unit |
| `a` | Insert text after the current unit |
| `x` | Clear existing annotations or mark the unit for deletion |
| `e` | Edit an existing annotation |
| `[`, `]` | Jump to the previous or next annotation |
| `/` | Search |
| `n`, `N` | Jump to the next or previous search match |
| `?`, `Shift` + `/` | Open or close help |
| `I` | Open or close the AST view |
| `O` | Reveal markdown links for the current sentence |
| `r` | Copy annotations to the clipboard |
| `q` | Quit and print annotations to stdout |
| `Q` | Quit silently and discard annotations |
| `Enter` | Save text in change, feedback, insert, edit, or search modes |
| `Esc` | Cancel the current input mode or close an open popup |

## Platform Support

| Platform | Release artifact | CI coverage | Support status |
| --- | --- | --- | --- |
| macOS x86_64 | `x86_64-apple-darwin` | Build and tests on GitHub-hosted macOS | Supported |
| macOS arm64 | `aarch64-apple-darwin` | Cross-target release build on GitHub-hosted macOS | Supported |

## Emitted Annotations Example
On exit, something like this prints to stdout, for the LLM agent to consume and make modifications to the markdown plan file.

```text
FILE: plan.md

ACTION: change
WHERE: line 12 sentence 2
CONTEXT:
  prev: The release workflow builds archives for every configured target.
  target: Windows artifacts are published even though the installer and tests do not cover Windows.
  next: Checksums are generated after packaging.
CHANGE: Stop publishing Windows archives until CI and installer support are added.
```

## License
MIT — see [LICENSE](LICENSE).
