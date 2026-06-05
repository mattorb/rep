Output docs/rep-demo.gif

Require ./target/release/rep

Set Shell "bash"
Set FontSize 18
Set FontFamily "Menlo"
Set LetterSpacing 0
Set Width 1200
Set Height 720
Set Padding 12
Set Framerate 24
Set PlaybackSpeed 0.3125

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "./target/release/rep scripts/demo-plan.md"
Enter
Sleep 780ms
Show

Sleep 1040ms
Type "?"
Sleep 1600ms
Escape
Sleep 780ms

Type "i"
Sleep 620ms
Type "j"
Sleep 620ms
Type "j"
Sleep 620ms

Type "c"
Sleep 620ms
Type "Ship the skill with every release archive"
Sleep 920ms
Enter
Sleep 1200ms

Type "]"
Sleep 780ms
Type "q"
Sleep 900ms
