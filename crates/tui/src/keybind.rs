pub const HELP: &str = "\
Keybindings:
  Shift+Tab        switch mode act <--> plan  (Alt+Tab / Ctrl+T fallback)
  Enter            submit (idle) / steer (running \u{2014} promoted at turn boundary)
  Tab              submit (idle) / follow-up queue (running \u{2014} after completion)
  Shift+Enter      insert newline (multi-line input)
  $                select skill (dropdown; fuzzy match)
  /                slash command picker: /task (sessions), /model (config)
  Esc              close help (if open) / close popup / clear input
  Esc Esc          double-tap Esc to interrupt a running task
  Ctrl+C / Ctrl+D  quit
  Ctrl+H           toggle this help
  Ctrl+N / Ctrl+P  next / previous history
  Up / Down        cursor vertical (multi-line) / history (single-line)
  Left / Right     move cursor
  Home / End       cursor to start / end
  PageUp/Down      scroll transcript  (PageDown = jump to bottom)
  Ctrl+U           scroll up
Mouse:            scroll wheel to scroll transcript; click arrow to follow
";
