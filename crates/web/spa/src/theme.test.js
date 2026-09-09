// theme.test.js — pure-node guard for the palette lockstep (no DOM, no JSX).
// theme.js (antd cssinjs tokens) and app.css (:root --oc-* custom properties)
// are two halves of one palette that cannot read each other: antd pins its
// generated css vars to a `.css-var-*` class instead of :root, so the raw-CSS
// surfaces (DAG nodes, sheets, dividers) have no way to consume them. The
// duplication is structural, which is exactly why it needs a test — these
// assertions turn "remember to change both" into a build failure.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { theme as antdTheme } from 'antd';
import { cssVars, palette, shadowTertiary, theme } from './theme.js';

const css = readFileSync(new URL('./app.css', import.meta.url), 'utf8');

/// Comments stripped first: the :root block carries explanatory comments
/// between declarations, and a value must never absorb one.
const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');

const rootBlock = () => {
  const m = /:root\s*\{([\s\S]*?)\n\}/.exec(stripComments(css));
  expect(m, 'app.css must declare a :root block').toBeTruthy();
  return m[1];
};

/// Declared --oc-* vars as { name: value }, values whitespace-normalized.
const declaredVars = () => {
  const out = {};
  const re = /(--oc-[\w-]+)\s*:\s*([^;]+);/g;
  for (const m of rootBlock().matchAll(re)) {
    out[m[1]] = m[2].replace(/\s+/g, ' ').trim();
  }
  return out;
};

/// Color/shadow comparison ignores all whitespace: antd emits
/// `rgba(0,0,0,0.88)` while Prettier-style CSS writes `rgba(0, 0, 0, 0.88)`.
const squash = (v) => String(v).replace(/\s+/g, '').toLowerCase();

describe('palette lockstep: theme.js cssVars <-> app.css :root', () => {
  it('declares exactly the same set of --oc-* variables', () => {
    // Bidirectional on purpose: a var added to only one side means the two
    // palettes have quietly forked.
    expect(Object.keys(declaredVars()).sort()).toEqual(Object.keys(cssVars).sort());
  });

  it('gives every shared variable the same value', () => {
    const declared = declaredVars();
    const drifted = Object.keys(cssVars)
      .filter((k) => squash(declared[k]) !== squash(cssVars[k]))
      .map((k) => `${k}: css=${declared[k]} theme.js=${cssVars[k]}`);
    expect(drifted, drifted.join('\n')).toEqual([]);
  });

  it('leaves no var(--oc-*) reference dangling on a fallback', () => {
    const declared = new Set(Object.keys(declaredVars()));
    const used = new Set([...css.matchAll(/var\((--oc-[\w-]+)/g)].map((m) => m[1]));
    expect([...used].filter((v) => !declared.has(v)).sort()).toEqual([]);
  });

  it('derives --oc-primary-rgb from --oc-primary (rgba() literals stay honest)', () => {
    const declared = declaredVars();
    const hex = declared['--oc-primary'];
    const n = parseInt(hex.slice(1), 16);
    expect(declared['--oc-primary-rgb'].replace(/\s+/g, ' '))
      .toBe(`${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`);
  });
});

describe('palette lockstep: antd derived tokens <-> CSS twins', () => {
  // getDesignToken runs the real seed -> map -> alias pipeline over our
  // config, so these are computed values, not a second hardcoded copy: if
  // someone retunes colorPrimary, colorPrimaryBg follows and the CSS twin is
  // flagged until it is updated too.
  const tokens = antdTheme.getDesignToken(theme);

  const twins = [
    ['--oc-primary', 'colorPrimary'],
    ['--oc-primary-bg', 'colorPrimaryBg'],
    ['--oc-bg-layout', 'colorBgLayout'],
    ['--oc-panel-bg', 'colorBgContainer'],
    ['--oc-border', 'colorBorderSecondary'],
    ['--oc-success', 'colorSuccess'],
    ['--oc-success-bg', 'colorSuccessBg'],
    ['--oc-error', 'colorError'],
    ['--oc-error-bg', 'colorErrorBg'],
    ['--oc-text', 'colorText'],
    ['--oc-text-secondary', 'colorTextSecondary'],
    ['--oc-shadow-panel', 'boxShadowTertiary'],
  ];

  it.each(twins)('%s equals antd %s', (cssVar, tokenKey) => {
    expect(squash(cssVars[cssVar]), `${cssVar} vs ${tokenKey}`)
      .toBe(squash(tokens[tokenKey]));
  });

  it('pins --oc-fill-subtle to the component tokens that consume it', () => {
    // Not colorFillAlter: that alias is a transparent rgba(0,0,0,0.02)
    // overlay. The opaque #fafbfc twin belongs to the Table head and the
    // Descriptions label column, which are the surfaces raw CSS mimics.
    const { Table, Descriptions } = theme.components;
    expect(squash(cssVars['--oc-fill-subtle'])).toBe(squash(Table.headerBg));
    expect(squash(cssVars['--oc-fill-subtle'])).toBe(squash(Descriptions.labelBg));
  });

  it('keeps --oc-text-tertiary an opaque hex (antd colorTextTertiary is rgba)', () => {
    // Documented exception, asserted so it stays deliberate rather than
    // drifting: raw CSS wants a flat color on the DAG canvas.
    expect(cssVars['--oc-text-tertiary']).toBe('#8c8c8c');
    expect(tokens.colorTextTertiary).not.toBe(cssVars['--oc-text-tertiary']);
  });
});

describe('theme config shape', () => {
  it('routes every palette color through the palette object', () => {
    const { token, components } = theme;
    expect(token.colorPrimary).toBe(palette.primary);
    expect(token.colorBgLayout).toBe(palette.bgLayout);
    expect(token.colorBorderSecondary).toBe(palette.border);
    expect(token.boxShadowTertiary).toBe(shadowTertiary);
    expect(components.Table.headerBg).toBe(palette.fillSubtle);
    expect(components.Table.borderColor).toBe(palette.border);
    expect(components.Descriptions.labelBg).toBe(palette.fillSubtle);
    expect(components.Card.colorBorderSecondary).toBe(palette.border);
    expect(components.Layout.bodyBg).toBe(palette.bgLayout);
    expect(components.Layout.siderBg).toBe(palette.panel);
    expect(components.Layout.headerBg).toBe(palette.panel);
    expect(components.Segmented.itemSelectedBg).toBe(palette.panel);
    expect(components.Tag.defaultBg).toBe(palette.bgLayout);
  });

  it('pins --oc-border-strong to the control border tokens', () => {
    // Scoped twin: the global colorBorder intentionally stays antd's
    // #d9d9d9, so the CSS var tracks the three controls that override it.
    const { components } = theme;
    expect(components.Button.defaultBorderColor).toBe(palette.borderStrong);
    expect(components.Input.colorBorder).toBe(palette.borderStrong);
    expect(components.Select.colorBorder).toBe(palette.borderStrong);
    expect(cssVars['--oc-border-strong']).toBe(palette.borderStrong);
  });

  it('keeps the deliberate radius scale 4 < 6 < 8 < 10', () => {
    // borderRadius is a SEED: genRadius(8) derives LG=10 for cards and SM=6,
    // which is why Tag pins SM back to 4 and Menu pins its item radius to 6.
    const { token, components } = theme;
    expect(token.borderRadius).toBe(8);
    expect(antdTheme.getDesignToken(theme).borderRadiusLG).toBe(10);
    expect(components.Card.borderRadiusLG).toBe(10);
    expect(components.Tag.borderRadiusSM).toBe(4);
    expect(components.Menu.itemBorderRadius).toBe(6);
  });

  it('selects the Menu pill instead of the right-hand active bar', () => {
    const { Menu } = theme.components;
    expect(Menu.activeBarBorderWidth).toBe(0);
    expect(Menu.itemSelectedColor).toBe(palette.primary);
    expect(Menu.itemBg).toBe('transparent');
  });

  it('drops the Table header split line', () => {
    expect(theme.components.Table.headerSplitColor).toBe('transparent');
  });
});
