[![CI](https://github.com/mattorb/rep/actions/workflows/ci.yml/badge.svg)](https://github.com/mattorb/rep/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

# Rep

A human in the loop TUI to revise markdown plan files quickly in collaboration with an LLM.  It is **made for use inside a tmux session** wrapping an agent tool like Claude Code or Codex. This is the way.

## Overview

`rep` opens a markdown file in an interactive terminal UI optimized for providing feedback and requesting changes.  On quit of the app, it prints out list a changes requested, for an AI agent.

For a seamless experience, launch Codex or Claude Code inside a tmux session, which allows rep to launch as modal and automatically pass the results into the agentic loop.

**Why not just talk to the LLM asking for the changes I want**
You can try, but to really target your changes and apply a whole series of them in one shot, you will end up having to provide lots of context.   Rep automatically includes context of _where_ in the plan you are requesting any given change.

## Installation

1. Download and extract a release from
[GitHub Release](https://github.com/mpstx/rep/releases) for macOS
(x86\_64, aarch64) and Linux (x86\_64, aarch64 — statically linked musl).

2. Put the binary 'rep' on your PATH.

## Usage

The BEST way to use this TUI tool is in the agentic loop, with a skill, immediately after you ask AI to help generate a plan (to a file) to accomplish a goal. This allows you to tap a few keys, put some feedback and requests in context quickly.

1. Ensure rep executable in on your PATH
2. Install the agent skill: `./install-skills.sh'
3. Launch tmux, and you Agentic coding tool inside of the tmux session.  Wrapping the agent in a tmux session is what allows rep to present modally and automatically continue the agentic loop on quit.
```
$ tmux new-session -t tryrep
$ claude
```
4.  Invoke the skill after generating a markdown plan file:

**Claude Code**
```
) generate a plan to accomplish [goal] and write it to a plan.md
...ai plans...
) /rep plan.md
...ai applies edits/feedback...
) 
```

**Codex**
```
> generate a plan to accomplish [goal] and write it to a plan.md
...ai plans...
> $rep plan.md
...ai applies edits/feedback...
>
```

## Platform Support
Note: I have been building and testing on Mac.   Linux and Windows binaries are in the release artifacts, but untested.

## Development
```sh
cargo build
# binary is at target/debug/rep
```

## License
MIT — see [LICENSE](LICENSE).
