import { Segmented, Space, Typography } from "antd";
import { useState } from "react";
import ReactMarkdown from "react-markdown";

function htmlDocument(content: string): string {
  const policy = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><style>body{font:14px/1.6 system-ui,sans-serif;color:#1f1f1f;margin:12px;overflow-wrap:anywhere}img{max-width:100%}pre{white-space:pre-wrap}table{max-width:100%}</style></head><body>${content}</body></html>`;
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
