// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import "../../../testSetup";
import { palette } from "../../../../theme.js";
import TextPreview from "./TextPreview";

const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, "");
const escape = (selector: string) => selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/// Declaration block of one rule (minified or pretty), whitespace-normalized.
const ruleBlock = (source: string, selector: string) => {
  const m = new RegExp(`(?:^|\\n)${escape(selector)}\\s*\\{([^}]*)\\}`).exec(stripComments(source));
  expect(m, `${selector} rule must exist`).toBeTruthy();
  const block = m![1].replace(/\s+/g, " ").trim();
  return block.endsWith(";") ? block : `${block};`; // a single declaration has no trailing `;`
};
const hexesOf = (source: string) => (source.match(/#[0-9a-f]{3,8}\b/gi) ?? []).map((hex) => hex.toLowerCase());
const declaredVars = (block: string) => new Set([...block.matchAll(/(--oc-[\w-]+)\s*:/g)].map((m) => m[1]));
const PALETTE_HEX = new Set(Object.values(palette).map((value) => String(value).toLowerCase()));

const html = "<h1>标题</h1><p>正文 <code>run</code> <mark>重点</mark></p><pre>echo hi</pre>";
const styleOf = (doc: string) => /<style>([\s\S]*?)<\/style>/.exec(doc)?.[1] ?? "";
const renderHtml = () => {
  render(<TextPreview name="说明" content={html} format="html" />);
  const frame = document.querySelector("iframe.entity-html-preview");
  expect(frame).toBeTruthy();
  return { frame: frame as HTMLIFrameElement, doc: frame!.getAttribute("srcdoc") ?? "" };
};

describe("HTML preview iframe (srcDoc)", () => {
  it("keeps the sandbox, referrer policy and CSP while embedding the content verbatim", () => {
    const { frame, doc } = renderHtml();
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(doc).toContain(`http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'"`);
    expect(doc.endsWith(`<body>${html}</body></html>`)).toBe(true);
    expect(doc).not.toMatch(/<script/i);
  });

  it("paints the dark palette from the shared theme tokens, never a light page", () => {
    const { doc } = renderHtml();
    const css = styleOf(doc);
    const root = ruleBlock(css, ":root");
    expect(root).toContain("color-scheme:dark;");
    for (const [name, value] of [
      ["--oc-bg-layout", palette.bgLayout], ["--oc-fill-subtle", palette.fillSubtle], ["--oc-border", palette.border],
      ["--oc-text", palette.text], ["--oc-text-secondary", palette.textSecondary], ["--oc-primary", palette.primary],
    ]) expect(root).toContain(`${name}:${String(value).toLowerCase()};`);
    const body = ruleBlock(css, "body");
    expect(body).toContain("color:var(--oc-text);");
    expect(body).toContain("background:var(--oc-bg-layout);");
    // Every hex in the document is a palette value: no light-era literal.
    const hexes = hexesOf(doc);
    expect(hexes.length).toBeGreaterThan(0);
    for (const hex of hexes) expect(PALETTE_HEX.has(hex), hex).toBe(true);
    expect(doc.toLowerCase()).not.toMatch(/#fff(?:fff)?(?![0-9a-f])|#1f1f1f|#fafafa|#f0f0f0/);
    expect(doc).not.toMatch(/[\s:;(]white[\s;)]/); // `white` as a color, not `white-space`
  });

  it("gives code and highlight elements token surfaces instead of UA defaults", () => {
    const css = styleOf(renderHtml().doc);
    const pre = ruleBlock(css, "pre");
    expect(pre).toContain("white-space:pre-wrap;");
    expect(pre).toContain("font-family:var(--oc-mono);");
    expect(pre).toContain("background:var(--oc-fill-subtle);");
    expect(pre).toContain("border:1px solid var(--oc-border);");
    expect(ruleBlock(css, "code")).toContain("background:var(--oc-fill-subtle);");
    expect(ruleBlock(css, "mark")).toContain("background:var(--oc-warning);");
    expect(ruleBlock(css, "a")).toContain("color:var(--oc-primary);");
    expect(ruleBlock(css, "img")).toContain("max-width:100%;");
    // The iframe cannot see the app's custom properties: every var() it reads
    // is declared on its own :root.
    const declared = declaredVars(ruleBlock(css, ":root"));
    for (const [, name] of css.matchAll(/var\((--oc-[\w-]+)/g)) expect(declared.has(name), name).toBe(true);
  });

  it("switches to the source view: no iframe, the raw content in a pre", () => {
    render(<TextPreview name="说明" content={html} format="html" />);
    fireEvent.click(screen.getByText("源码"));
    expect(document.querySelector("iframe")).toBeNull();
    expect(document.querySelector("pre")?.textContent).toBe(html);
  });
});

it("renders non-html formats as Markdown without an iframe", () => {
  render(<TextPreview name="说明" content={"# 标题\n\n正文"} format="md" />);
  expect(document.querySelector("iframe")).toBeNull();
  expect(document.querySelector("h1")?.textContent).toBe("标题");
});
