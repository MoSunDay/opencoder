// Compare actual CSS declarations with antd tokens and check source references.
import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { theme as antdTheme } from 'antd';
import { algorithm, dropShadowPopover, FONT_SIZE, palette, SANS, shadowFloating, shadowPanel, tints, theme, withAlpha } from './theme.js';
import { MONO } from './ui/mono.js';

/// app.css inlines the token / shell / canvas sheets; the --oc-* block lives
/// in the token sheet, the shell and canvas rules are read where a layout
/// contract (48px header, 16px padding, 6px radius, no shadow) is asserted.
const stylesheet = (name) => readFileSync(new URL(name, import.meta.url), 'utf8');
const appCss = stylesheet('./app.css');
const tokenCss = stylesheet('./styles/tokens.css');
const shellCss = stylesheet('./styles/shell.css');
const canvasCss = stylesheet('./styles/canvas.css');

/// Comments stripped first: the :root block carries explanatory comments
/// between declarations, and a value must never absorb one.
const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');

const rootBlock = () => {
  const m = /:root\s*\{([\s\S]*?)\n\}/.exec(stripComments(tokenCss));
  expect(m, 'tokens.css must declare a :root block').toBeTruthy();
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

/// Declaration block of one top-level rule, e.g. ruleBlock(shellCss, '.fleet-content').
const ruleBlock = (source, selector) => {
  const re = new RegExp(`(?:^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`);
  const m = re.exec(stripComments(source));
  expect(m, `${selector} rule must exist`).toBeTruthy();
  return m[1].replace(/\s+/g, ' ');
};

/// Color/shadow comparison ignores all whitespace: antd emits
/// `rgba(0,0,0,0.88)` while Prettier-style CSS writes `rgba(0, 0, 0, 0.88)`.
const squash = (v) => String(v).replace(/\s+/g, '').toLowerCase();

/// Every literal the light conversion left behind, in one list. A single
/// occurrence anywhere (sheet or config) means a token slipped back.
const LIGHT_ERA = [
  '#1677ff', '22, 119, 255', '#f4f6f8', '#eef1f5', '#d3dae2', '#fafbfc', '#e8f1ff', '#f5f9ff', '#f0f2f5',
  '#ffffff', '#fff', '#8c8c8c', '#52c41a', '#f6ffed', '#ff4d4f', '#fff2f0', '#faad14', '#13c2c2', '#9254de',
  '#722ed1', '#389e0d', 'rgba(0, 0, 0,', 'rgba(16, 24, 40', 'rgba(22, 119, 255',
];

/// `{ name: source }` -> offenders; a hex must end at a non-hex char so
/// #fff does not flag #fff2f0 twice, rgba() literals match verbatim.
const lightEraOffenders = (sources) => {
  const offenders = [];
  for (const [name, src] of Object.entries(sources)) {
    const lower = src.toLowerCase();
    for (const literal of LIGHT_ERA) {
      const pattern = literal.startsWith('#') ? `${literal}(?![0-9a-f])` : literal.replace(/[()]/g, '\\$&');
      if (new RegExp(pattern).test(lower)) offenders.push(`${name}: ${literal}`);
    }
  }
  return offenders;
};

describe('stylesheet wiring', () => {
  it('app.css imports the token, shell and canvas sheets before any rule', () => {
    // CSS @import must lead the file; Vite inlines the three sheets into the
    // single dist/static/app.css, so the cascade is tokens, shell, canvas,
    // then the base rules below.
    const lines = stripComments(appCss).split('\n').map((l) => l.trim()).filter(Boolean);
    expect(lines.slice(0, 3)).toEqual([
      "@import './styles/tokens.css';",
      "@import './styles/shell.css';",
      "@import './styles/canvas.css';",
    ]);
    expect(lines[3]).toBe('html,');
  });

  it('declares a dark color-scheme so native controls follow the theme', () => {
    expect(rootBlock()).toMatch(/color-scheme\s*:\s*dark;/);
  });
});

describe('CSS palette', () => {
  it.each([
    ['--oc-accent-user', '#39c5cf'],
    ['--oc-accent-ai', '#b392f0'],
    ['--oc-accent-binary', '#d2a8ff'],
    ['--oc-heading', '#56d364'],
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
    // The split sheets are part of the walk (they reference the tokens).
    expect(usedBy.get('--oc-panel-bg')?.has('styles/shell.css')).toBe(true);
    expect(usedBy.get('--oc-primary-rgb')?.has('styles/canvas.css')).toBe(true);
    const dangling = [...usedBy.keys()]
      .filter((v) => !declared.has(v))
      .sort()
      .map(
        (v) =>
          `${v} used by ${[...usedBy.get(v)].sort().join(', ')}`,
      );
    expect(dangling, dangling.join('\n')).toEqual([]);
  });

  it('keeps every --oc-*-rgb twin equal to its hex sibling', () => {
    // rgba(var(--oc-primary-rgb), a) washes and focus rings must mix the
    // exact hue the hex var paints, or a retuned hex silently leaves a
    // drifted twin painting e.g. a cyan-tinted avatar with a blue-green glyph while every
    // other test still passes.
    const declared = declaredVars();
    const rgbVars = Object.keys(declared).filter((k) => k.endsWith('-rgb')).sort();
    expect(rgbVars).toEqual([
      '--oc-accent-ai-rgb',
      '--oc-accent-user-rgb',
      '--oc-error-rgb',
      '--oc-primary-rgb',
      '--oc-success-rgb',
      '--oc-warning-rgb',
    ]);
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

  it.each([
    ['--oc-primary-bg', '--oc-primary-rgb'],
    ['--oc-success-bg', '--oc-success-rgb'],
    ['--oc-warning-bg', '--oc-warning-rgb'],
    ['--oc-error-bg', '--oc-error-rgb'],
  ])('mixes the %s wash from %s', (wash, rgb) => {
    const declared = declaredVars();
    const m = /^rgba\((\d+), (\d+), (\d+), (0\.\d+)\)$/.exec(declared[wash]);
    expect(m, `${wash} must be an rgba() wash`).toBeTruthy();
    expect(`${m[1]}, ${m[2]}, ${m[3]}`).toBe(declared[rgb]);
    expect(m[4]).toBe('0.15');
  });

  it('keeps the light-era literals out of the stylesheets and the theme', () => {
    const sources = {
      'app.css': stripComments(appCss),
      'styles/tokens.css': stripComments(tokenCss),
      'styles/shell.css': stripComments(shellCss),
      'styles/canvas.css': stripComments(canvasCss),
      'theme.js': JSON.stringify({ palette, tints, theme, shadowPanel, shadowFloating }),
    };
    expect(lightEraOffenders(sources)).toEqual([]);
  });
});

describe('palette lockstep: antd derived tokens <-> CSS twins', () => {
  // getDesignToken runs the real seed -> map -> alias pipeline over our
  // config (dark algorithm + reference-hue pin), so these are computed
  // values, not a second hardcoded copy: if someone retunes colorPrimary,
  // colorPrimaryBg follows and the CSS twin is flagged until it is updated too.
  const tokens = antdTheme.getDesignToken(theme);

  const twins = [
    ['--oc-primary', 'colorPrimary'],
    ['--oc-primary-bg', 'colorPrimaryBg'],
    ['--oc-bg-layout', 'colorBgLayout'],
    ['--oc-panel-bg', 'colorBgContainer'],
    ['--oc-bg-container', 'colorBgElevated'],
    ['--oc-border', 'colorBorderSecondary'],
    ['--oc-border-strong', 'colorBorder'],
    ['--oc-success', 'colorSuccess'],
    ['--oc-success-bg', 'colorSuccessBg'],
    ['--oc-error', 'colorError'],
    ['--oc-error-bg', 'colorErrorBg'],
    ['--oc-warning', 'colorWarning'],
    ['--oc-warning-bg', 'colorWarningBg'],
    ['--oc-text', 'colorText'],
    ['--oc-text-secondary', 'colorTextSecondary'],
    ['--oc-text-tertiary', 'colorTextTertiary'],
    ['--oc-shadow-panel', 'boxShadowTertiary'],
  ];

  it.each(twins)('%s equals antd %s', (cssVar, tokenKey) => {
    expect(squash(declaredVars()[cssVar]), `${cssVar} vs ${tokenKey}`)
      .toBe(squash(tokens[tokenKey]));
  });

  it('pins --oc-fill-subtle to the component tokens that consume it', () => {
    // Not colorFillAlter: that alias is a transparent rgba(255,255,255,0.04)
    // overlay. The opaque #21262d twin belongs to the Table head, the
    // Descriptions label column, the Tag fill and the Segmented track,
    // which are the surfaces raw CSS mimics.
    const { Table, Descriptions, Tag, Segmented } = theme.components;
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Table.headerBg));
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Descriptions.labelBg));
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Tag.defaultBg));
    expect(squash(declaredVars()['--oc-fill-subtle'])).toBe(squash(Segmented.trackBg));
  });

  it('keeps --oc-text-tertiary an opaque hex (antd dark derives rgba by default)', () => {
    // Pinned as an alias override so the raw-CSS DAG canvas and the antd
    // description text share one flat color.
    expect(declaredVars()['--oc-text-tertiary']).toMatch(/^#[0-9a-f]{6}$/);
    expect(declaredVars()['--oc-text-tertiary']).toBe(palette.textTertiary);
  });

  it('derives the washes from the palette hex, not from a second copy', () => {
    expect(withAlpha('#58a6ff', 0.15)).toBe('rgba(88, 166, 255, 0.15)');
    expect(tints.primary).toBe('rgba(88, 166, 255, 0.15)');
    expect(tints.primaryHover).toBe(withAlpha(palette.primary, 0.25));
    expect(tints.success).toBe(withAlpha(palette.success, 0.15));
    expect(tints.warning).toBe(withAlpha(palette.warning, 0.15));
    expect(tints.error).toBe(withAlpha(palette.error, 0.15));
    expect(tints.focus).toBe(withAlpha(palette.primary, 0.4));
  });
});

describe('theme config shape', () => {
  const tokens = antdTheme.getDesignToken(theme);

  it('runs the dark algorithm and pins the reference hues after it', () => {
    expect(theme.algorithm).toBe(algorithm);
    expect(algorithm[0]).toBe(antdTheme.darkAlgorithm);
    expect(algorithm).toHaveLength(2);
    // The dark palette alone returns #4e90dc for the #58a6ff seed; the pin
    // restores the contract hue while the palette keeps hover / active.
    const hues = {
      colorPrimary: palette.primary,
      colorInfo: palette.primary,
      colorLink: palette.primary,
      colorSuccess: palette.success,
      colorWarning: palette.warning,
      colorError: palette.error,
    };
    for (const [key, value] of Object.entries(hues)) {
      expect(tokens[key], key).toBe(value);
    }
    expect(tokens.colorLinkHover).toBe(tokens.colorPrimaryHover);
    expect(tokens.colorLinkActive).toBe(tokens.colorPrimaryActive);
    expect(tokens.colorPrimaryHover).not.toBe(palette.primary);
    expect(tokens.colorPrimaryActive).not.toBe(palette.primary);
    // Surfaces and text prove the dark map, not just the seeds.
    expect(tokens.colorBgContainer).toBe(palette.panel);
    expect(tokens.colorText).toBe(palette.text);
    expect(tokens.colorBgMask).toBe('rgba(1, 4, 9, 0.75)');
  });

  it('routes every palette color through the palette object', () => {
    const { token, components } = theme;
    expect(token.colorPrimary).toBe(palette.primary);
    expect(token.colorBgLayout).toBe(palette.bgLayout);
    expect(token.colorBorderSecondary).toBe(palette.border);
    expect(token.colorBorder).toBe(palette.borderStrong);
    expect(token.boxShadowTertiary).toBe(shadowPanel);
    expect(token.boxShadow).toBe(shadowFloating);
    expect(token.boxShadowSecondary).toBe(shadowFloating);
    expect(token.dropShadowPopover).toBe(dropShadowPopover);
    expect(components.Table.headerBg).toBe(palette.fillSubtle);
    expect(components.Table.borderColor).toBe(palette.border);
    expect(components.Descriptions.labelBg).toBe(palette.fillSubtle);
    expect(components.Card.colorBorderSecondary).toBe(palette.border);
    expect(components.Layout.bodyBg).toBe(palette.bgLayout);
    expect(components.Layout.siderBg).toBe(palette.panel);
    expect(components.Layout.lightSiderBg).toBe(palette.panel);
    expect(components.Layout.headerBg).toBe(palette.panel);
    expect(components.Segmented.itemSelectedBg).toBe(palette.panel);
    expect(components.Tag.defaultBg).toBe(palette.fillSubtle);
  });

  it('pins --oc-border-strong to the control border tokens', () => {
    // The global colorBorder carries it too (inputs, selects, default
    // buttons all read the same value), so the CSS var has one source.
    const { components } = theme;
    expect(tokens.colorBorder).toBe(palette.borderStrong);
    expect(components.Button.defaultBorderColor).toBe(palette.borderStrong);
    expect(components.Input.colorBorder).toBe(palette.borderStrong);
    expect(components.Select.colorBorder).toBe(palette.borderStrong);
    expect(declaredVars()['--oc-border-strong']).toBe(palette.borderStrong);
  });

  it('keeps the radius scale: Tag 4 < primary 6 (controls, cards, sheet, nodes) < floating LG 8', () => {
    // borderRadius is a SEED: genRadius(6) derives SM=4 and LG=8; the LG
    // only reaches floating layers (popover / dropdown boxes), Card is
    // pinned back to the primary 6 like the sheet and the DAG canvases.
    const { token, components } = theme;
    expect(token.borderRadius).toBe(6);
    expect(tokens.borderRadiusLG).toBe(8);
    expect(tokens.borderRadiusSM).toBe(4);
    expect(components.Card.borderRadiusLG).toBe(6);
    expect(components.Tag.borderRadiusSM).toBe(4);
    expect(components.Menu.itemBorderRadius).toBe(6);
    for (const [sheet, selector] of [
      [shellCss, '.fleet-sheet'],
      [shellCss, '.fleet-brand'],
      [canvasCss, '.dag-detail-graph'],
      [canvasCss, '.dag-edit-stage'],
      [canvasCss, '.dag-edit-node'],
      [canvasCss, '.dag-node'],
    ]) {
      expect(ruleBlock(sheet, selector), selector).toContain('border-radius: 6px;');
    }
  });

  it('sizes the header at 48px and the content gap / padding at 16px', () => {
    const { Layout } = theme.components;
    expect(Layout.headerHeight).toBe(48);
    expect(Layout.headerPadding).toBe('0 16px');
    expect(ruleBlock(shellCss, '.fleet-content')).toContain('padding: 16px;');
    expect(ruleBlock(shellCss, '.fleet-header')).toContain('gap: 16px;');
    expect(ruleBlock(shellCss, '.oc-page-header')).toContain('margin-bottom: 16px;');
    // Header / sheet separate by their border alone: no soft shadow.
    expect(ruleBlock(shellCss, '.fleet-header')).toContain('border-bottom: 1px solid var(--oc-border);');
    expect(ruleBlock(shellCss, '.fleet-header')).not.toContain('box-shadow');
    expect(ruleBlock(shellCss, '.fleet-sheet')).toContain('border: 1px solid var(--oc-border);');
    expect(ruleBlock(shellCss, '.fleet-sheet')).toContain('box-shadow: var(--oc-shadow-panel);');
    expect(declaredVars()['--oc-shadow-panel']).toBe('none');
  });

  it('floats modal / popup / popover layers on one dark drop, panels cast none', () => {
    // antd's dark defaults for these are white-based washes (rgba(255,255,255,
    // 0.01-0.12)) that vanish on the canvas, and the popover one is a glow:
    // every floating shadow mirrors shadowFloating, panel shadows are none.
    expect(tokens.boxShadow).toBe(shadowFloating); // Modal / Notification container
    expect(tokens.boxShadowSecondary).toBe(shadowFloating); // dropdown / select / picker popups
    expect(tokens.dropShadowPopover).toBe(`drop-shadow(${shadowFloating})`); // Popover / Tooltip root
    expect(tokens.boxShadowTertiary).toBe('none'); // borderless Card
    for (const key of ['boxShadow', 'boxShadowSecondary', 'dropShadowPopover']) {
      expect(squash(tokens[key]), key).not.toContain('255,255,255');
    }
    expect(squash(shadowFloating)).toBe('08px24pxrgba(1,4,9,0.5)');
  });

  it('borders the antd 6 panel classes, never the antd 5 element names', () => {
    // antd 6 renamed the inner elements: the Modal panel is
    // .ant-modal-container, the popover panel .ant-popover-container, the
    // drawer panel .ant-drawer-content-wrapper > -section, the collapse
    // body .ant-collapse-body. A rule on the antd 5 names (-content /
    // -inner / -content-box) matches nothing and silently does nothing.
    const sheets = { appCss, shellCss, canvasCss };
    const legacy = [
      /\.ant-modal-content(?![-\w])/,
      /\.ant-popover-inner(?![-\w])/,
      /\.ant-drawer-content(?![-\w])/,
      /\.ant-collapse-content(?![-\w])/,
    ];
    for (const [name, source] of Object.entries(sheets)) {
      for (const re of legacy) {
        expect(re.test(stripComments(source)), `${name}: ${re.source}`).toBe(false);
      }
    }
    // Modal container / dropdown menu / select + picker popups: antd ships
    // them borderless, so app.css restates the hairline (one grouped rule).
    const grouped = /\.ant-modal \.ant-modal-container,[\s\S]*?\{([^}]*)\}/.exec(stripComments(appCss));
    expect(grouped, 'floating-layer border rule').toBeTruthy();
    for (const selector of [
      '.ant-modal .ant-modal-container',
      '.ant-dropdown .ant-dropdown-menu',
      '.ant-select-dropdown',
      '.ant-picker-panel-container',
    ]) {
      expect(grouped[0]).toContain(selector);
    }
    expect(grouped[1]).toContain('border: 1px solid var(--oc-border);');
    expect(grouped[1]).not.toContain('box-shadow: none');
    // Popover / Tooltip rely on the drop-shadow filter (the clip-path arrow
    // cannot carry a border); only the arrowless brand popover gets a
    // hairline, in the shell sheet, on the antd 6 container class.
    const brand = ruleBlock(shellCss, '.fleet-brand-popover .ant-popover-container');
    expect(brand).toContain('border: 1px solid var(--oc-border);');
    expect(brand).toContain('padding: 6px;');
    // Docked drawers: hairline on the inner edge only, border-box so a
    // 100vw drawer cannot overflow by it; header / body keep the 16px gap.
    for (const side of ['right', 'left']) {
      const edge = side === 'right' ? 'border-left' : 'border-right';
      const wrapper = ruleBlock(appCss, `.ant-drawer-${side} > .ant-drawer-content-wrapper`);
      expect(wrapper).toContain(`${edge}: 1px solid var(--oc-border);`);
      expect(wrapper).toContain('box-sizing: border-box;');
    }
    expect(ruleBlock(appCss, '.ant-drawer .ant-drawer-body')).toContain('padding: 16px;');
    expect(ruleBlock(appCss, '.ant-collapse-body pre')).toContain('white-space: pre-wrap;');
  });

  it('selects the Menu pill instead of the right-hand active bar', () => {
    const { Menu } = theme.components;
    expect(Menu.activeBarBorderWidth).toBe(0);
    expect(Menu.itemBg).toBe('transparent');
    expect(Menu.itemSelectedBg).toBe(tints.primary);
    expect(Menu.itemSelectedColor).toBe(palette.primary);
    expect(Menu.itemHoverBg).toBe(palette.fillSubtle);
    expect(Menu.itemHoverColor).toBe(palette.text);
    expect(Menu.itemColor).toBe(palette.textSecondary);
  });

  it('drops the Table header split line and keeps rows light', () => {
    const { Table } = theme.components;
    expect(Table.headerSplitColor).toBe('transparent');
    expect(Table.headerColor).toBe(palette.textSecondary);
    expect(Table.rowHoverBg).toBe(tints.rowHover);
    expect(Table.rowSelectedBg).toBe(tints.primary);
    expect(Table.rowSelectedHoverBg).toBe(tints.primaryHover);
    expect(Table.cellPaddingBlock).toBe(10);
    expect(Table.cellPaddingInline).toBe(12);
  });

  it('leaves no light selected / hover / track token in the component config', () => {
    // Every selected / hover / track surface is a palette fill or a tint of
    // the reference hue; buttons cast no shadow at all.
    const { Segmented, Tag, Button, Tabs, Menu } = theme.components;
    expect(Segmented.trackBg).toBe(palette.fillSubtle);
    expect(Segmented.itemSelectedColor).toBe(palette.text);
    expect(Tag.defaultColor).toBe(palette.textSecondary);
    expect(Button.primaryShadow).toBe('none');
    expect(Button.defaultShadow).toBe('none');
    expect(Button.dangerShadow).toBe('none');
    expect(Tabs.itemColor).toBe(palette.textSecondary);
    expect(Tabs.itemSelectedColor).toBe(palette.primary);
    expect(Menu.itemSelectedBg).toMatch(/^rgba\(/);
    expect(lightEraOffenders({ theme: JSON.stringify(theme) })).toEqual([]);
  });

  it('pins the antd code face to the same stack as --oc-mono', () => {
    // <Typography.Text code> reads token.fontFamilyCode; if it forks from
    // MONO / --oc-mono, inline code renders in a different face than every
    // MONO_VAR identifier (antd's own default carries Courier, no
    // ui-monospace).
    expect(theme.token.fontFamilyCode).toBe(MONO);
    expect(theme.token.fontFamilyCode.replace(/\s+/g, ' ').trim()).toBe(declaredVars()['--oc-mono']);
  });

  it('pins the prose face and the base size to the stylesheet', () => {
    expect(theme.token.fontFamily).toBe(SANS);
    expect(squash(theme.token.fontFamily)).toBe(squash(declaredVars()['--oc-font-sans']));
    for (const face of ['-apple-system', 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', 'sans-serif']) {
      expect(SANS).toContain(face);
    }
    expect(theme.token.fontSize).toBe(FONT_SIZE);
    expect(theme.token.fontSize).toBe(13);
    expect(theme.token.fontSizeSM).toBe(12);
    expect(declaredVars()['--oc-font-size-base']).toBe('13px');
    expect(ruleBlock(appCss, 'body')).toContain('font-size: var(--oc-font-size-base);');
    expect(ruleBlock(appCss, 'body')).toContain('font-family: var(--oc-font-sans);');
  });
});
