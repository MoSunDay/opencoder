// markdown.jsx — project-module markdown renderer (goal/milestone detail,
// plan_md, run snapshots). The content is authored inside this local tool by
// the user or the tool's own LLM — the SAME trust domain as the chat
// transcript renderer — so dangerouslySetInnerHTML over marked GFM output is
// acceptable here; no third-party/cross-origin HTML ever enters this path.

import { useMemo } from 'react';
import { marked } from 'marked';
import './project.css';

/// <Markdown text={md}/> — empty/absent text renders an em-dash placeholder
/// (the panel convention for "nothing here yet"), anything else renders GFM.
export function Markdown({ text }) {
  const html = useMemo(
    () => marked.parse(String(text || ''), { gfm: true, breaks: true, async: false }),
    [text],
  );
  if (!text) {
    return <div className="md-body md-body--empty">—</div>;
  }
  return <div className="md-body" dangerouslySetInnerHTML={{ __html: html }} />;
}
