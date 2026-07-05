pub const HELP: &str = "\
Keybindings:
  Enter            submit prompt (or queue follow-up while running)
  Shift/Alt+Enter  insert newline (multi-line input)
  $                select skill (dropdown; loads ~/.opencode/skills)
  /                task picker — switch/create/resume sessions
  Esc              close help (if open) / close popup / clear input
  Esc Esc          double-tap Esc to interrupt a running task
  Ctrl+C / Ctrl+D  quit
  Ctrl+T           switch agent plan <-> act (toggles by current mode)
  Ctrl+H           toggle this help
  Ctrl+O           admit steer (redirect mid-run)
  Ctrl+J           admit follow-up to queue
  Ctrl+N / Ctrl+P  next / previous history
  Up / Down        cursor vertical (multi-line) / history (single-line)
  Left / Right     move cursor
  Home / End       cursor to start / end
  PageUp/Down      scroll transcript  (PageDown = jump to bottom)
  Ctrl+U           scroll up
Mouse:            scroll wheel to scroll transcript; click arrow to follow
";
