pub const HELP: &str = "\
Keybindings:
  Shift+Tab        switch mode act <--> plan  (Alt+Tab fallback)
  Enter            submit (idle) / steer (running \u{2014} promoted at turn boundary)
  Tab              submit (idle) / follow-up queue (running \u{2014} after completion)
  Ctrl+Shift+Tab  switch mode act <--> plan (keep context, no handoff reset)
  Shift+Enter / Alt+Enter / Ctrl+J   insert newline (multi-line input)
  $                pick skill anywhere -> {$name}; loaded on submit
  /                slash command picker: /task (sessions), /config (settings), /model (providers), /compact (compress history)
  Esc              close help (if open) / close popup / clear input
  Esc Esc          double-tap Esc to interrupt a running task
  Ctrl+D           quit
  Ctrl+H           toggle this help
  Ctrl+W           delete word before cursor (backward-kill-word)
  Ctrl+N / Ctrl+P  next / previous history
  Home / End       cursor to start / end
  Ctrl+A / Ctrl+E  cursor to start / end (same as Home / End)
  PageUp/Down      scroll transcript  (PageDown = jump to bottom)
  Ctrl+U / Ctrl+L  exit subagent view (if focused) / collapse all thinking / clear input
Mouse:            scroll wheel to scroll transcript; click arrow to follow
                  drag in the body to select text and copy it to the clipboard (OSC52)
                  SHIFT+drag = terminal-native selection (fallback when OSC52 is blocked)
                  steer panel: \u{2715} delete, > submit now (interrupt & promote)
";
