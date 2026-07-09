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
Set Framerate 12
Set PlaybackSpeed 1.0
Set TypingSpeed 0.057

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "cd __REP_DEMO_ROOT__"
Enter
Sleep 200ms
Type "printf %s\\n 'set -g status-left [rep-demo]' 'set -g status-left-length 20' 'set -g status-right rep-demo' 'set -g status-right-length 10' > /tmp/rep-claude-skill-demo.tmux.conf; cp scripts/claude-rep-skill-demo-claude-settings.json /tmp/rep-claude-skill-demo-claude-settings.json; shopt -s expand_aliases; alias tmux='command tmux -L rep-claude-skill-demo -f /tmp/rep-claude-skill-demo.tmux.conf'"
Enter
Sleep 200ms
Type "tmux kill-session -t rep-claude-skill-demo 2>/dev/null || true"
Enter
Sleep 200ms
Type "rm -f /tmp/rep-claude-skill-demo-plan.md /tmp/rep-claude-skill-demo-plan.ready; (for _ in {1..180}; do if [ -f /tmp/rep-claude-skill-demo-plan.md ]; then cp scripts/claude-rep-skill-demo-plan.md /tmp/rep-claude-skill-demo-plan.md; sleep 0.5; cp scripts/claude-rep-skill-demo-plan.md /tmp/rep-claude-skill-demo-plan.md; touch /tmp/rep-claude-skill-demo-plan.ready; break; fi; sleep 0.5; done) >/dev/null 2>&1 &"
Enter
Sleep 200ms
Type "clear"
Enter
Sleep 200ms
Show
Set TypingSpeed 0.035
Type "tmux new-session -A -s rep-claude-skill-demo"
Set TypingSpeed 0.057
Sleep 500ms
Hide
Enter
Sleep 900ms

Type "tmux set-option -g status-left '[rep-demo]' && tmux set-option -g status-right rep-demo && tmux set-option -g status-left-length 20 && tmux set-option -g status-right-length 10"
Enter
Sleep 200ms
Type "export USER=demo LOGNAME=demo HOST=demo HOSTNAME=demo REP_SHOW_KEYS=1; unset RPROMPT; precmd_functions=(); preexec_functions=(); alias claude='command claude --settings /tmp/rep-claude-skill-demo-claude-settings.json'; PROMPT='rep-demo % '; PS1='rep-demo $ '"
Enter
Sleep 200ms
Type "clear"
Enter
Sleep 200ms
Show
Sleep 500ms
Set TypingSpeed 0.035
Type "claude --model opus --permission-mode bypassPermissions"
Enter
Set TypingSpeed 0.057
Sleep 5000ms

Type "Create a concise markdown rollout plan for checkout recovery and write it to /tmp/rep-claude-skill-demo-plan.md."
Enter
Sleep 2500ms
Hide
Sleep 37500ms
Show

Type "/rep /tmp/rep-claude-skill-demo-plan.md"
Enter
Sleep 1800ms
Hide
Sleep 20200ms
Show

Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "f"
Sleep 450ms
Type "name the owning team and the authoritative state source"
Sleep 450ms
Enter
Sleep 700ms

Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "a"
Sleep 450ms
Type "Add a launch gate that compares recovered-cart deltas before rollout."
Sleep 450ms
Enter
Sleep 700ms

Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "x"
Sleep 700ms

Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "j"
Sleep 420ms
Type "f"
Sleep 450ms
Type "use customer-safe labels and document the mapping for support"
Sleep 450ms
Enter
Sleep 700ms

Type "q"
Sleep 800ms
Type "y"
Sleep 4000ms
Hide
Sleep 86000ms
Show
Sleep 2500ms

Hide
Type "/quit"
Enter
Sleep 1000ms
Type "tmux kill-session -t rep-claude-skill-demo"
Enter
Sleep 1500ms
