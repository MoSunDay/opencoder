import { Segmented, Space, Typography } from "antd";
import { useState } from "react";
import ReactMarkdown from "react-markdown";
import { SANS, palette } from "../../../../theme.js";
import { MONO } from "../../../../ui/mono.js";

/// The preview iframe is a separate document: the --oc-* custom properties the
/// app stylesheet declares never cross that boundary, so the preview declares
/// the subset it paints with from the same `palette` object theme.js and
/// styles/tokens.css share (never a second copy of the hex values). Each entry
/// becomes one `<name>:<value>;` declaration on the document's :root.
const PREVIEW_TOKENS: Record<string, string> = {
  "--oc-bg-layout": palette.bgLayout,
  "--oc-fill-subtle": palette.fillSubtle,
  "--oc-border": palette.border,
  "--oc-border-strong": palette.borderStrong,
  "--oc-text": palette.text,
  "--oc-text-secondary": palette.textSecondary,
  "--oc-primary": palette.primary,
  "--oc-warning": palette.warning,
  "--oc-font-sans": SANS,
  "--oc-mono": MONO,
};

const cssText = (text: string) => text.replace(/\s+/g, " ").trim();

/// Document CSS for the previewed HTML. Mirrors the dark Markdown surfaces in
/// app.css (body text on the canvas, code on the subtle fill behind a hairline
/// border, links in the reference blue) and pins color-scheme so UA chrome
/// (scrollbars, form controls, hr) follows the theme instead of painting a
/// light page inside the dark console. Content stays the only thing between
/// <body> and </body>; rules only style elements the content may contain.
const PREVIEW_CSS = `:root{color-scheme:dark;${Object.entries(PREVIEW_TOKENS).map(([name, value]) => `${name}:${cssText(value)};`).join("")}}
body{font:14px/1.6 var(--oc-font-sans);color:var(--oc-text);background:var(--oc-bg-layout);margin:12px;overflow-wrap:anywhere}
a{color:var(--oc-primary)}
img{max-width:100%}
pre{white-space:pre-wrap;font-family:var(--oc-mono);background:var(--oc-fill-subtle);border:1px solid var(--oc-border);border-radius:6px;padding:8px 10px}
code{font-family:var(--oc-mono);background:var(--oc-fill-subtle);border-radius:4px;padding:0 4px}
pre code{background:none;padding:0}
mark{background:var(--oc-warning);color:var(--oc-bg-layout)}
blockquote{border-left:3px solid var(--oc-border-strong);color:var(--oc-text-secondary)}
table{max-width:100%;border-collapse:collapse}
th,td{border:1px solid var(--oc-border);padding:4px 8px}
th{background:var(--oc-fill-subtle)}
hr{border:0;border-top:1px solid var(--oc-border)}`;

function htmlDocument(content: string): string {
  const policy = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><style>${PREVIEW_CSS}</style></head><body>${content}</body></html>`;
}

export default function TextPreview({ name, content, format }: { name: string; content: string; format?: string }) {
  const [mode, setMode] = useState<string | number>("preview");
  if (format !== "html") {
    const path = !content.includes("\n") && /^(?:\.?\.?\/|[\w.-]+\/)/.test(content) && !/^https?:\/\//i.test(content);
    return path ? <Typography.Paragraph copyable={{ text: content }} style={{ whiteSpace: "pre-wrap" }}>{content}</Typography.Paragraph>
      : <ReactMarkdown skipHtml components={{ a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer noopener">{children}</a> }}>{content}</ReactMarkdown>;
  }
  return <Space direction="vertical" style={{ width: "100%" }}>
    <Segmented aria-label={`${name}展示方式`} value={mode} onChange={setMode}
      options={[{ label: "预览", value: "preview" }, { label: "源码", value: "source" }]} />
    {mode === "preview" ? <iframe className="entity-html-preview" title={`${name}预览`} sandbox=""
      referrerPolicy="no-referrer" srcDoc={htmlDocument(content)} />
      : <Typography.Paragraph copyable={{ text: content }}><pre>{content}</pre></Typography.Paragraph>}
  </Space>;
}
