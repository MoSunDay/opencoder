// antd ThemeConfig for the console. theme.test.js checks the shared tokens
// against the token stylesheet (src/styles/tokens.css) and against the
// design tokens antd computes from this config; CSS-only accents stay in
// the stylesheet.
import { theme as antdTheme } from 'antd';
import { MONO } from './ui/mono.js';

/// Base colors shared with src/styles/tokens.css (--oc-*). Each key maps to
/// the CSS variable of the same kebab-case name: `bgLayout` <->
/// `--oc-bg-layout`. Change one, change the other (guarded by theme.test.js).
/// Dark, GitHub-style: a near-black canvas, one panel surface for header /
/// sider / cards / overlays, a lighter fill for table heads and labels, and
/// hairline borders that carry the whole hierarchy (no soft shadows).
export const palette = {
  // Reference blue. DAG edges and the STATUS_META semantic colors are built
  // on it, so it stays put.
  primary: '#58a6ff', // --oc-primary
  bgLayout: '#0d1117', // --oc-bg-layout - page canvas behind every panel
  panel: '#161b22', // --oc-panel-bg - header / sider / card / overlay surface
  fillSubtle: '#21262d', // --oc-fill-subtle - table head / label / tag fill
  border: '#30363d', // --oc-border - hairline dividers and table borders
  borderStrong: '#484f58', // --oc-border-strong - control borders, DAG nodes
  text: '#e6edf3', // --oc-text
  textSecondary: '#8b949e', // --oc-text-secondary
  textTertiary: '#6e7681', // --oc-text-tertiary
  success: '#3fb950', // --oc-success
  warning: '#d29922', // --oc-warning
  error: '#f85149', // --oc-error
};

/// Prose face. Same stack as the --oc-font-sans custom property (checked by
/// theme.test.js); the CJK faces after the Latin ones keep Chinese copy out
/// of the browser's default serif fallback.
export const SANS = `-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial,
    'Noto Sans', 'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', sans-serif,
    'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji'`;

/// Base type size (px) for body copy and every antd component.
export const FONT_SIZE = 13;

/// Translucent tint of a palette hex. antd's dark palette renders status
/// backgrounds as near-black hexes (#152217 for green); a wash of the exact
/// reference hue reads as the intended dark surface instead. tokens.css
/// declares the same rgba() values by hand (theme.test.js lockstep).
export const withAlpha = (hex, alpha) => {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
};

/// Translucent surfaces. Each primary / status entry is both an antd alias
/// token override and a --oc-* CSS var twin (theme.test.js).
export const tints = {
  primary: withAlpha(palette.primary, 0.15), // colorPrimaryBg / --oc-primary-bg
  primaryHover: withAlpha(palette.primary, 0.25), // colorPrimaryBgHover
  primaryBorder: withAlpha(palette.primary, 0.4), // colorPrimaryBorder
  success: withAlpha(palette.success, 0.15), // colorSuccessBg / --oc-success-bg
  successBorder: withAlpha(palette.success, 0.4), // colorSuccessBorder
  warning: withAlpha(palette.warning, 0.15), // colorWarningBg / --oc-warning-bg
  warningBorder: withAlpha(palette.warning, 0.4), // colorWarningBorder
  error: withAlpha(palette.error, 0.15), // colorErrorBg / --oc-error-bg
  errorBorder: withAlpha(palette.error, 0.4), // colorErrorBorder
  // Table row hover: body text at 4% over the panel, below the selected wash.
  rowHover: 'rgba(230, 237, 243, 0.04)',
  // Focus ring around inputs / selects / buttons (controlOutline).
  focus: withAlpha(palette.primary, 0.4),
};

/// Panels separate from the canvas by their 1px border alone; the
/// --oc-shadow-panel twin is `none` as well. Only the layers that float over
/// the canvas (modal / notification, dropdown / select / picker popups,
/// popover / tooltip) keep one dark drop shadow; a drawer docks to the
/// viewport edge and separates by its hairline. The mask behind modal /
/// drawer uses the canvas hue. antd's dark defaults for every one of these
/// are white-based washes at 1-12% that vanish (or glow) on the canvas.
export const shadowPanel = 'none';
export const shadowFloating = '0 8px 24px rgba(1, 4, 9, 0.5)';
/// Popover / Tooltip root runs a drop-shadow filter so the shadow follows
/// the clip-path arrow (which cannot carry a border); drop-shadow() takes
/// the same offsets / blur / color as the box shadow.
export const dropShadowPopover = `drop-shadow(${shadowFloating})`;
export const maskBg = 'rgba(1, 4, 9, 0.75)';

/// antd's darkAlgorithm re-tunes every seed hue through its dark palette
/// (#58a6ff comes back as #4e90dc). The reference hues are the contract, so
/// this second derivative pins them back once the dark palette has produced
/// their hover / active stops and background scales. Components only ever
/// read colorPrimary & co, never a literal, so nothing else follows the
/// retuned value.
const pinReferenceHues = (seed, dark) => ({
  ...dark,
  colorPrimary: seed.colorPrimary,
  colorInfo: seed.colorInfo,
  colorLink: seed.colorLink,
  colorSuccess: seed.colorSuccess,
  colorWarning: seed.colorWarning,
  colorError: seed.colorError,
  // Link / info hover + active stops follow primary's lighter / darker
  // shades (the dark palette maps them to darker colors on its own).
  colorInfoHover: dark.colorPrimaryHover,
  colorInfoActive: dark.colorPrimaryActive,
  colorLinkHover: dark.colorPrimaryHover,
  colorLinkActive: dark.colorPrimaryActive,
});

/// Dark palette + the reference-hue pin. Exported for the tests.
export const algorithm = [antdTheme.darkAlgorithm, pinReferenceHues];

/// antd ThemeConfig (v6). Component notes:
/// - Layout runs panel-on-canvas (headerBg / siderBg / lightSiderBg /
///   bodyBg + colorBgLayout); main.jsx passes theme="light" to the Sider and
///   the Menu, so both the dark-sider and light-sider tokens name the panel.
/// - Header is 48px with 16px inline padding; radius is 6 everywhere (the
///   seed), Tag pinned to SM 4; the derived LG 8 only reaches floating
///   layers (popover / dropdown boxes), Card is pinned back to 6.
/// - Menu drops the right-hand blue active bar for a pill selection.
/// - antd 6 has no Table `size` component token, so default-sized tables
///   are normalized through the cell padding tokens (10/12 keeps 13px text
///   airy). The head keeps no vertical split lines (headerSplitColor).
/// - Card unifies body/header padding at 16 (default is paddingLG = 24).
/// - fontFamilyCode pins <Typography.Text code> to the same MONO stack as
///   --oc-mono; antd's own default code face carries Courier and lacks
///   ui-monospace, and fontFamily pins the prose face to --oc-font-sans.
export const theme = {
  algorithm,
  token: {
    // Seeds: run through the dark palette (see pinReferenceHues).
    colorPrimary: palette.primary,
    colorInfo: palette.primary,
    colorLink: palette.primary,
    colorSuccess: palette.success,
    colorWarning: palette.warning,
    colorError: palette.error,
    fontFamily: SANS,
    fontFamilyCode: MONO,
    fontSize: FONT_SIZE,
    borderRadius: 6,
    // Alias overrides: applied after the algorithm, verbatim.
    colorBgLayout: palette.bgLayout,
    colorBgContainer: palette.panel,
    colorBgElevated: palette.panel, // popover / modal / drawer surface
    colorBgSpotlight: palette.borderStrong, // Tooltip surface
    colorBgMask: maskBg,
    colorText: palette.text,
    colorTextSecondary: palette.textSecondary,
    colorTextTertiary: palette.textTertiary,
    colorBorder: palette.borderStrong,
    colorBorderSecondary: palette.border,
    colorPrimaryBg: tints.primary,
    colorPrimaryBgHover: tints.primaryHover,
    colorPrimaryBorder: tints.primaryBorder,
    colorInfoBg: tints.primary,
    colorInfoBorder: tints.primaryBorder,
    colorSuccessBg: tints.success,
    colorSuccessBorder: tints.successBorder,
    colorWarningBg: tints.warning,
    colorWarningBorder: tints.warningBorder,
    colorErrorBg: tints.error,
    colorErrorBorder: tints.errorBorder,
    controlOutline: tints.focus,
    fontSizeSM: 12, // the 13px seed would derive 10 (unreadable for tags)
    // Floating layers: Modal / Notification read the base shadow, dropdown /
    // select / picker popups boxShadowSecondary, popover / tooltip the
    // drop-shadow filter. Panels (Cards, sheet) cast none.
    boxShadow: shadowFloating,
    boxShadowSecondary: shadowFloating,
    dropShadowPopover,
    boxShadowTertiary: shadowPanel,
  },
  components: {
    Layout: {
      headerBg: palette.panel,
      headerHeight: 48,
      headerPadding: '0 16px',
      siderBg: palette.panel,
      lightSiderBg: palette.panel,
      bodyBg: palette.bgLayout,
    },
    Menu: {
      itemBg: 'transparent', // seamless sider, no inner box
      activeBarBorderWidth: 0, // no right-hand blue bar: pill selection
      itemBorderRadius: 6,
      itemMarginInline: 8,
      itemHeight: 34,
      itemColor: palette.textSecondary,
      itemHoverColor: palette.text,
      itemHoverBg: palette.fillSubtle,
      itemSelectedBg: tints.primary,
      itemSelectedColor: palette.primary,
    },
    Table: {
      cellPaddingBlock: 10,
      cellPaddingInline: 12,
      headerBg: palette.fillSubtle,
      headerColor: palette.textSecondary,
      headerSplitColor: 'transparent',
      borderColor: palette.border,
      rowHoverBg: tints.rowHover,
      rowSelectedBg: tints.primary,
      rowSelectedHoverBg: tints.primaryHover,
    },
    Card: {
      bodyPadding: 16,
      headerPadding: 16,
      borderRadiusLG: 6,
      colorBorderSecondary: palette.border,
    },
    Button: {
      fontWeight: 500,
      primaryShadow: 'none',
      defaultShadow: 'none',
      dangerShadow: 'none',
      defaultBorderColor: palette.borderStrong,
    },
    Segmented: {
      trackBg: palette.fillSubtle,
      itemSelectedBg: palette.panel,
      itemSelectedColor: palette.text,
    },
    Input: {
      colorBorder: palette.borderStrong,
    },
    Select: {
      colorBorder: palette.borderStrong,
    },
    Tag: {
      borderRadiusSM: 4,
      defaultBg: palette.fillSubtle,
      defaultColor: palette.textSecondary,
    },
    Descriptions: {
      labelBg: palette.fillSubtle,
    },
    Tabs: {
      inkBarColor: palette.primary,
      itemColor: palette.textSecondary,
      itemHoverColor: palette.text,
      itemSelectedColor: palette.primary,
      horizontalItemGutter: 16,
    },
    Form: {
      verticalLabelPadding: '0 0 4px',
      itemMarginBottom: 16,
    },
  },
};
