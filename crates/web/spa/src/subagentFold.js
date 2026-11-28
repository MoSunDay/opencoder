// subagentFold.js — the transcript-wide disclosure shared by every subagent
// block (TUI chat.rs parity: `toggle_subagents` flips ALL subagent blocks
// at once, never one alone). TranscriptView owns the single `{ open,
// toggle }` pair (default closed, so subagent bodies stay hidden until the
// reader asks) and hands it down through SubagentFoldContext; each
// SubagentContent (subagentBlock.jsx) reads it via useSubagentFold() and
// renders the ladder's shared glyph (❯ open / ▸ closed — disclosureIcon in
// stepsBlock.jsx) in its header. Ctrl/Cmd+L and the `⤒ 收起` link close the
// shared state together with the bubble-remount epoch, in transcript.jsx.
// Outside a provider (defensive: a block mounted on its own) the hook falls
// back to a block-local toggle with the same shape, so the block still works.

import { createContext, useContext, useState } from 'react';

export const SubagentFoldContext = createContext(null);

/// The provider's transcript-wide `{ open, toggle }` when one is mounted
/// above (the TranscriptView path), else a block-local pair.
export function useSubagentFold() {
  const fold = useContext(SubagentFoldContext);
  const [localOpen, setLocalOpen] = useState(false);
  if (fold) return fold;
  return { open: localOpen, toggle: () => setLocalOpen((v) => !v) };
}
