import { Fragment, useMemo } from 'react';
import { textRows } from './markdown.js';
import { Markdown } from '../project/markdown.jsx';
import { MONO_VAR } from '../ui/mono.js';

export function TextRows({ rows }) {
  return <div className="transcript-text" style={{ fontFamily: MONO_VAR, fontSize: 13, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
    {rows.map((row, i) => <Fragment key={i}>
      {i ? '\n' : null}
      {row.map((part, j) => {
        const style = {
          color: part.muted ? 'var(--oc-text-tertiary)' : part.heading ? 'var(--oc-heading)' : part.code || part.link ? 'var(--oc-primary)' : undefined,
          fontStyle: part.italic ? 'italic' : undefined,
          textDecoration: part.strike ? 'line-through' : part.link ? 'underline' : undefined,
        };
        return part.bold ? <strong key={j} style={style}>{part.text}</strong> : <span key={j} style={style}>{part.text}</span>;
      })}
    </Fragment>)}
  </div>;
}

export function AssistantText({ turn }) {
  const rows = useMemo(() => textRows(turn.text, true), [turn.text]);
  return turn.open === true ? <TextRows rows={rows} /> : <Markdown text={turn.text} />;
}
