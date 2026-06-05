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
Set PlaybackSpeed 1.0
Set TypingSpeed 0.1

Env TERM "xterm-256color"
Env COLORTERM "truecolor"

Hide
Type "./target/release/rep scripts/demo-plan.md"
Enter
Sleep 780ms
Show

Sleep 540ms
Type "j"
Sleep 320ms
Type "j"
Sleep 320ms
Type "j"

Type "x"
Sleep 320ms

Sleep 320ms
Type "j"
Sleep 320ms
Type "j"
Sleep 320ms

Type "a"
Sleep 320ms
Type "and ship ETA confirmed"
Sleep 320ms
Enter
Sleep 800ms

Type "j"
Sleep 620ms
Type "j"
Sleep 620ms

Type "f"
Sleep 620ms
Type "pending, payment_confirmed"
Sleep 920ms
Enter
Sleep 1200ms

Type "j"
Sleep 620ms

Type "f"
Sleep 620ms
Type "notify customer we are still working on it"
Sleep 620ms
Enter
Sleep 600ms

Type "q"
Sleep 620ms
Type "y"
Sleep 900ms
