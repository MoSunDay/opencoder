// antd ThemeConfig for the console. theme.test.js checks shared tokens
// against app.css :root; CSS-only accents stay in the stylesheet.
import { MONO } from './ui/mono.js';

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
/// - fontFamilyCode pins <Typography.Text code> (runsTable / agentsConfig /
///   usersDrawer) to the same MONO stack as --oc-mono. antd's own default code
///   face carries `Courier` and lacks `ui-monospace`, so without this inline
///   code would render in a different face than every MONO_VAR identifier.
export const theme = {
  token: {
    colorPrimary: palette.primary,
    colorBgLayout: palette.bgLayout,
    colorBorderSecondary: palette.border,
    fontSize: 14,
    borderRadius: 8,
    boxShadowTertiary: shadowTertiary,
    fontFamilyCode: MONO,
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
