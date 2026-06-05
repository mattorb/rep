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
Type "j"
Sleep 620ms
Type "j"
Sleep 620ms
Type "j"
Sleep 620ms
Type "j"
Sleep 620ms
Type "j"
Sleep 620ms

Type "c"
Sleep 620ms
Type "Change after payment succeeds to after payment succeeds and ship ETA confirmed"
Sleep 920ms
Enter
Sleep 1200ms

Type "j"
Sleep 620ms
Type "j"
Sleep 620ms

Type "f"
Sleep 620ms
Type "pending, payment_confirmed, scheduled_for_ship"
Sleep 920ms
Enter
Sleep 1200ms

Type "j"
Sleep 620ms

Type "f"
Sleep 620ms
Type "notify customer we are still working on it"
Sleep 920ms
Enter
Sleep 1200ms

Type "k"
Sleep 520ms
Type "k"
Sleep 520ms
Type "k"
Sleep 520ms
Type "k"
Sleep 520ms
Type "k"
Sleep 520ms

Type "x"
Sleep 900ms

Type "q"
Sleep 620ms
Type "y"
Sleep 900ms
