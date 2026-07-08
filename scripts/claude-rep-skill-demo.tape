Output docs/rep-claude-skill-demo.gif

Require ./target/release/rep
Require claude
Require tmux

Set Shell "bash"
Set FontSize 18
Set FontFamily "Menlo"
Set LetterSpacing 0
Set Width 1280
Set Height 760
Set Padding 12
Set Framerate 24
Set PlaybackSpeed 1.0
Set TypingSpeed 0.08

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "cd __REP_DEMO_ROOT__"
Enter
Sleep 200ms
Type "tmux kill-session -t rep-claude-skill-demo 2>/dev/null || true"
Enter
Sleep 200ms
Type "rm -f /tmp/rep-claude-skill-demo-plan.md && cp scripts/demo-plan.md /tmp/rep-claude-skill-demo-plan.md"
Enter
Sleep 200ms
Type "tmux new-session -A -s rep-claude-skill-demo"
Enter
Sleep 900ms
Show

Sleep 500ms
Type "REP_BIN=__REP_BIN__ claude --permission-mode bypassPermissions"
Enter
Sleep 500ms
Hide
Sleep 6500ms

Type "/rep /tmp/rep-claude-skill-demo-plan.md"
Enter
Sleep 12000ms
Show

Type "j"
Sleep 320ms
Type "j"
Sleep 320ms
Type "j"
Sleep 320ms
Type "f"
Sleep 450ms
Type "make the owner explicit"
Sleep 450ms
Enter
Sleep 700ms

Type "j"
Sleep 320ms
Type "a"
Sleep 450ms
Type "Confirm rollback criteria before launch."
Sleep 450ms
Enter
Sleep 700ms

Type "q"
Sleep 800ms
Type "y"
Sleep 30000ms

Type "/quit"
Enter
Sleep 1000ms
Type "tmux kill-session -t rep-claude-skill-demo"
Enter
Sleep 1500ms
