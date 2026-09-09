// theme.js — the single antd v6 ThemeConfig behind the fleet console's
// "light unified" shell: white header + white sider floating on a cool light
// gray layout canvas. Pure data, no imports — main.jsx feeds it to
// <ConfigProvider theme={theme}>. Every color here has a CSS twin in the
// app.css :root block (--oc-* variables): raw CSS surfaces (DAG nodes,
// sheets, dividers) cannot read cssinjs tokens, so the two halves must stay
// in lockstep. theme.test.js asserts that invariant — antd pins its css vars
// to a `.css-var-*` class rather than :root, so app.css cannot consume them
// and the duplication is unavoidable, not an oversight.

/// Base colors shared with app.css :root (--oc-*). Each key maps to the CSS
/// variable of the same kebab-case name: `bgLayout` <-> `--oc-bg-layout`.
/// Change one, change the other (guarded by theme.test.js).
export const palette = {
  // Classic antd blue — DAG edges and the STATUS_META semantic colors are
  // built on it, so it stays put.
  primary: '#1677ff', // --oc-primary
  // Page canvas behind the white panels. Deliberately cooler than antd's
  // muddy #f5f5f5 default so the white surfaces read as elevated.
  bgLayout: '#f4f6f8', // --oc-bg-layout
  panel: '#ffffff', // --oc-panel-bg — header / sider / card surface
  border: '#eef1f5', // --oc-border — hairline dividers and table borders
  borderStrong: '#d3dae2', // --oc-border-strong — control borders, DAG nodes
  fillSubtle: '#fafbfc', // --oc-fill-subtle — table head / label backgrounds
};

/// Soft elevation for surfaces floating on the canvas. Note antd applies
/// boxShadowTertiary to a Card only when it is borderless (`&:not(
/// -bordered)`); every Card here is outlined, so cards stay border-only and
/// this literal reaches .fleet-sheet in app.css (which mirrors it by hand).
export const shadowTertiary =
  '0 1px 2px rgba(16, 24, 40, 0.04), 0 1px 3px rgba(16, 24, 40, 0.06)';

/// Focus ring shared by inputs and selects — one soft blue halo instead of
/// antd's default controlOutline. Select takes a bare color (it builds the
/// `0 0 0 2px` shadow itself), Input takes the whole shadow string.
const focusRing = 'rgba(22, 119, 255, 0.10)';

/// `#rrggbb` -> the `r, g, b` triplet form used inside rgba() literals, so
/// --oc-primary-rgb can never drift away from --oc-primary.
const rgbTriplet = (hex) => {
  const n = parseInt(hex.slice(1), 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
};

/// The app.css :root contract — every `--oc-*` custom property, with its
/// value spelled exactly as the CSS declares it (theme.test.js compares the
/// two after whitespace normalization, in both directions, so a var added to
/// one side only is a build failure). Keys deliberately absent from
/// `palette` are antd defaults that raw-CSS surfaces still need a copy of:
/// colorPrimaryBg / colorSuccess(Bg) / colorError(Bg) / colorText(Secondary).
/// --oc-text-tertiary is NOT antd's colorTextTertiary (rgba(0,0,0,.45)):
/// raw CSS wants an opaque hex that stays legible on the DAG canvas.
export const cssVars = {
  '--oc-primary': palette.primary,
  '--oc-primary-rgb': rgbTriplet(palette.primary),
  '--oc-primary-bg': '#e6f4ff',
  '--oc-bg-layout': palette.bgLayout,
  '--oc-panel-bg': palette.panel,
  // Referenced with a #fff fallback by .dag-edit-pal; declared here so no
  // var(--oc-*) in the sheet is left dangling on a fallback.
  '--oc-bg-container': palette.panel,
  '--oc-border': palette.border,
  // Scoped to controls (Button/Input/Select) rather than the global
  // colorBorder, which intentionally stays antd's #d9d9d9.
  '--oc-border-strong': palette.borderStrong,
  '--oc-fill-subtle': palette.fillSubtle,
  '--oc-text': 'rgba(0, 0, 0, 0.88)',
  '--oc-text-secondary': 'rgba(0, 0, 0, 0.65)',
  '--oc-text-tertiary': '#8c8c8c',
  '--oc-success': '#52c41a',
  '--oc-success-bg': '#f6ffed',
  '--oc-error': '#ff4d4f',
  '--oc-error-bg': '#fff2f0',
  '--oc-shadow-panel': shadowTertiary,
  // Monospace stack for id / uuid / session columns (raw identifiers, not
  // prose). src/ui/mono.js owns the same literal on the JS side.
  '--oc-mono': `ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas,
    'Liberation Mono', monospace`,
};

/// antd ThemeConfig (v6). Component notes:
/// - Layout goes white-on-gray (headerBg/siderBg/bodyBg + colorBgLayout):
///   Content panels float on the canvas instead of sitting on white.
/// - Radius is a deliberate four-step scale: Tag 4 < Menu item 6 <
///   controls 8 (borderRadius is a SEED, so genRadius(8) drives buttons and
///   inputs too, and derives borderRadiusLG = 10 for cards) < sheet 10.
/// - Menu drops the right-hand blue active bar for a pill selection.
/// - antd 6 has no Table `size` component token, so default-sized tables
///   are normalized to middle density via the cell padding tokens. The head
///   keeps no vertical split lines (headerSplitColor) — hairlines only.
/// - Card unifies body/header padding at 16 (default is paddingLG = 24).
export const theme = {
  token: {
    colorPrimary: palette.primary,
    colorBgLayout: palette.bgLayout,
    colorBorderSecondary: palette.border,
    fontSize: 14,
    borderRadius: 8,
    boxShadowTertiary: shadowTertiary,
  },
  components: {
    Layout: {
      headerBg: palette.panel,
      headerHeight: 56,
      headerPadding: '0 24px', // tighter than the 50px default (mobile CSS
      // overrides this further with !important)
      siderBg: palette.panel,
      bodyBg: palette.bgLayout,
    },
    Menu: {
      itemBg: 'transparent', // seamless light sider, no inner box
      activeBarBorderWidth: 0, // no right-hand blue bar
      itemBorderRadius: 6,
      itemMarginInline: 8,
      itemHeight: 36,
      itemColor: 'rgba(0, 0, 0, 0.72)',
      itemSelectedBg: '#e8f1ff',
      itemSelectedColor: palette.primary,
    },
    Table: {
      cellPaddingBlock: 12,
      cellPaddingInline: 8,
      headerBg: palette.fillSubtle,
      headerColor: 'rgba(0, 0, 0, 0.65)',
      headerSplitColor: 'transparent',
      borderColor: palette.border,
      rowHoverBg: '#f5f9ff',
    },
    Card: {
      bodyPadding: 16,
      headerPadding: 16,
      borderRadiusLG: 10,
      colorBorderSecondary: palette.border,
    },
    Button: {
      fontWeight: 500,
      primaryShadow: '0 1px 2px rgba(22, 119, 255, 0.24)',
      defaultBorderColor: palette.borderStrong,
    },
    Segmented: {
      trackBg: '#f0f2f5',
      itemSelectedBg: palette.panel,
      itemSelectedColor: palette.primary,
    },
    Input: {
      activeShadow: `0 0 0 2px ${focusRing}`,
      colorBorder: palette.borderStrong,
    },
    Select: {
      activeOutlineColor: focusRing,
      colorBorder: palette.borderStrong,
    },
    Tag: {
      borderRadiusSM: 4,
      defaultBg: palette.bgLayout,
    },
    Descriptions: {
      labelBg: palette.fillSubtle,
    },
    Tabs: {
      inkBarColor: palette.primary,
      horizontalItemGutter: 24,
    },
  },
};
