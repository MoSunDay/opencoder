// Compare actual CSS declarations with antd tokens and check source references.
import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { theme as antdTheme } from 'antd';
import { palette, shadowTertiary, theme } from './theme.js';
import { MONO } from './ui/mono.js';

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

describe('CSS palette', () => {
  it.each([
    ['--oc-accent-user', '#13c2c2'],
    ['--oc-accent-ai', '#9254de'],
    ['--oc-accent-binary', '#722ed1'],
    ['--oc-heading', '#389e0d'],
  ])('keeps the CSS-only color %s', (name, value) => {
    expect(declaredVars()[name]).toBe(value);
  });

  it('leaves no var(--oc-*) reference dangling in production sources', () => {
    const declared = new Set(Object.keys(declaredVars()));
    const srcFiles = readdirSync(new URL('.', import.meta.url), { recursive: true })
      .filter((p) => typeof p === 'string')
      .map((p) => p.split(/[\\/]/).join('/'))
      .filter((p) => /\.(css|js|jsx|ts|tsx)$/.test(p))
      .filter((p) => !/(^|\/)(node_modules|__tests__|test|fixtures)(\/|$)|(?:\.test|testSupport|setup-dom)\./.test(p));
    const usedBy = new Map();
    for (const name of srcFiles) {
      const src = stripComments(readFileSync(new URL(`./${name}`, import.meta.url), 'utf8'));
      for (const m of src.matchAll(/var\(\s*(--oc-[\w-]+)/g)) {
        if (!usedBy.has(m[1])) usedBy.set(m[1], new Set());
        usedBy.get(m[1]).add(name);
      }
    }
    // Prove the walk covers each source type and reads inline JS references.
    for (const extension of ['css', 'js', 'jsx', 'ts', 'tsx']) {
      expect(srcFiles.some((p) => p.endsWith(`.${extension}`)), `scan ${extension}`).toBe(true);
    }
    expect(usedBy.get('--oc-mono')?.has('ui/mono.js')).toBe(true);
    const dangling = [...usedBy.keys()]
      .filter((v) => !declared.has(v))
      .sort()
      .map((v) => `${v} (used by ${[...usedBy.get(v)].join(', ')})`);
    expect(dangling, dangling.join('\n')).toEqual([]);
  });

  it('derives every --oc-*-rgb from its hex twin (rgba() literals stay honest)', () => {
    // Enumerate ALL -rgb vars rather than hardcoding --oc-primary: a drifted
    // twin paints e.g. a cyan-tinted avatar with a blue-green glyph while every
    // other test still passes.
    const declared = declaredVars();
    const rgbVars = Object.keys(declared).filter((k) => k.endsWith('-rgb')).sort();
    expect(rgbVars.length, 'expected at least one --oc-*-rgb var').toBeGreaterThan(0);
    const offenders = [];
    for (const rgb of rgbVars) {
      const hexName = rgb.slice(0, -'-rgb'.length);
      const hex = declared[hexName];
      if (!/^#[0-9a-fA-F]{6}$/.test(hex || '')) {
        offenders.push(`${rgb}: missing #rrggbb twin ${hexName} (got ${hex})`);
        continue;
      }
      const n = parseInt(hex.slice(1), 16);
      const want = `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
      if (declared[rgb] !== want) {
        offenders.push(`${rgb}: css=${declared[rgb]} want=${want} (from ${hexName}=${hex})`);
      }
    }
    expect(offenders, offenders.join('\n')).toEqual([]);
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
    ['--oc-bg-container', 'colorBgContainer'],
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
    expect(squash(declaredVars()[cssVar]), `${cssVar} vs ${tokenKey}`)
      .toBe(squash(tokens[tokenKey]));
  });

  it('pins --oc-fill-subtle to the component tokens that consume it', () => {
    // Not colorFillAlter: that alias is a transparent rgba(0,0,0,0.02)
    // overlay. The opaque #fafbfc twin belongs to the Table head and the
    // Descriptions label column, which are the surfaces raw CSS mimics.
    const { Table, Descriptions } = theme.components;
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Table.headerBg));
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Descriptions.labelBg));
  });

  it('keeps --oc-text-tertiary an opaque hex (antd colorTextTertiary is rgba)', () => {
    // Documented exception, asserted so it stays deliberate rather than
    // drifting: raw CSS wants a flat color on the DAG canvas.
    expect(declaredVars()['--oc-text-tertiary']).toBe('#8c8c8c');
    expect(tokens.colorTextTertiary).not.toBe(declaredVars()['--oc-text-tertiary']);
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
    expect(declaredVars()['--oc-border-strong']).toBe(palette.borderStrong);
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

  it('pins the antd code face to the same stack as --oc-mono', () => {
    // <Typography.Text code> reads token.fontFamilyCode; if it forks from
    // MONO / --oc-mono, inline code renders in a different face than every
    // MONO_VAR identifier (antd's own default carries Courier, no
    // ui-monospace).
    expect(theme.token.fontFamilyCode).toBe(MONO);
    expect(theme.token.fontFamilyCode.replace(/\s+/g, ' ').trim()).toBe(declaredVars()['--oc-mono']);
  });
});
