import { Fragment, useMemo } from 'react';
import { textRows } from './markdown.js';

export function TextRows({ rows }) {
  return <div className="transcript-text" style={{ fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', fontSize: 13, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
    {rows.map((row, i) => <Fragment key={i}>
      {i ? '\n' : null}
      {row.map((part, j) => {
        const style = {
          color: part.muted ? '#8c8c8c' : part.heading ? '#389e0d' : part.code || part.link ? '#1677ff' : undefined,
          fontStyle: part.italic ? 'italic' : undefined,
          textDecoration: part.strike ? 'line-through' : part.link ? 'underline' : undefined,
        };
        return part.bold ? <strong key={j} style={style}>{part.text}</strong> : <span key={j} style={style}>{part.text}</span>;
      })}
    </Fragment>)}
  </div>;
}

export function AssistantText({ turn }) {
  const rows = useMemo(() => textRows(turn.text, turn.open === true), [turn.text, turn.open]);
  return <TextRows rows={rows} />;
}
