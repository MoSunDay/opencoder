// markdown.jsx — project-module markdown renderer (goal/milestone detail,
// plan_md, run snapshots). This is the SPA's ONLY dangerouslySetInnerHTML
// sink: marked GFM output is sanitized through DOMPurify before rendering
// (defense in depth), so even if user- or LLM-authored content carries
// hostile HTML (script/onevent/javascript: URLs), it never reaches the DOM.

import { useMemo } from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import './project.css';

/// <Markdown text={md}/> — empty/absent text renders an em-dash placeholder
/// (the panel convention for "nothing here yet"), anything else renders GFM.
export function Markdown({ text }) {
  const html = useMemo(
    () => DOMPurify.sanitize(marked.parse(String(text || ''), { gfm: true, breaks: true, async: false })),
    [text],
  );
  if (!text) {
    return <div className="md-body md-body--empty">—</div>;
  }
  return <div className="md-body" dangerouslySetInnerHTML={{ __html: html }} />;
}
