Output "__TERMINAL_OUTPUT__.mp4"

Set Shell "bash"
Set FontSize 17
Set FontFamily "Menlo"
Set LetterSpacing 0
Set Width 1440
Set Height 900
Set Padding 12
Set Framerate 24
Set PlaybackSpeed 1.0
Set TypingSpeed 1ms

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "cd __REP_DEMO_ROOT__"
Enter
Sleep 100ms
Type "clear"
Enter
Sleep 100ms
Type "python3 -c 'import time; print(round(time.time() * 1000))' > __VHS_START_FILE__"
Enter
Sleep 100ms
Type "__TMUX_BIN__ -L __TMUX_SOCKET__ attach-session -t claude"
Enter
Sleep 1s
Show

Wait+Screen@30s /permissions on/
Sleep 500ms
Set TypingSpeed 0.015
Type "__CLAUDE_PLAN_PROMPT__"
Enter
Set TypingSpeed 0.001
Sleep 4s
Hide
Wait+Screen@300s /plan-ready/
Show

Sleep 2500ms
Set TypingSpeed 0.07
Type "/rep @demo-plan.html"
Enter
Sleep 400ms
Enter
Set TypingSpeed 0.001
Sleep 1200ms
Hide
Wait+Screen@300s /browser-ready/
Show
Wait+Screen@300s /apply-running/
Sleep 4s
Hide
Wait+Screen@300s /revision-ready/
Show

Sleep 5s
Hide
Set TypingSpeed 0.07
Type "/quit"
Enter
Wait+Screen@300s /REP_HTML_DEMO_COMPLETE/
Sleep 2s
