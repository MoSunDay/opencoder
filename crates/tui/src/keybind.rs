pub const HELP: &str = "\
Keybindings:
  Enter            submit prompt (or queue follow-up while running)
  $                select skill (popup; loads ~/.opencoder/skills)
  Esc              close help (if open) / clear input
  Esc Esc          double-tap Esc to interrupt a running task
  Ctrl+C / Ctrl+D  quit
  Ctrl+T           switch agent plan <-> act (toggles by current mode)
  Ctrl+H           toggle this help
  Ctrl+O           admit steer (redirect mid-run)
  Ctrl+J           admit follow-up to queue
  Ctrl+N / Ctrl+P  next / previous history
  Up / Down        history navigation
  Left / Right     move cursor
  Home / End       cursor to start / end
  PageUp/Down      scroll transcript  (PageDown = jump to bottom)
  Ctrl+U           scroll up
Mouse:            click the bottom-right arrow to jump back to bottom
";
