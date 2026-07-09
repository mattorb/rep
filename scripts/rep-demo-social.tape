Output docs/rep-demo-social.gif

Require ./target/release/rep

Set Shell "bash"
Set FontSize 18
Set FontFamily "Menlo"
Set LetterSpacing 0
Set Width 1200
Set Height 720
Set Padding 12
Set Framerate 10
Set PlaybackSpeed 0.85
Set TypingSpeed 0.1

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "./target/release/rep --show-keys --demo"
Enter
Sleep 520ms
Show

Sleep 520ms
Type "j"
Sleep 540ms
Type "j"
Sleep 540ms

Type "x"
Sleep 640ms

Type "j"
Sleep 540ms

Type "a"
Sleep 420ms
Type "add owner"
Sleep 520ms
Enter
Sleep 720ms

Type "f"
Sleep 420ms
Type "tighten scope"
Sleep 520ms
Enter
Sleep 720ms

Type "Q"
