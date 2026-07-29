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

Sleep 4s
Hide
Wait+Screen@300s /plan-ready/
Show

Wait+Screen@300s /apply-running/
Sleep 4s
Hide
Wait+Screen@300s /revision-ready/
Show

Wait+Screen@300s /REP_HTML_DEMO_COMPLETE/
Sleep 2s
