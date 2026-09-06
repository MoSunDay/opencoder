// theme.js — the single antd v6 ThemeConfig behind the fleet console's
// "light unified" shell: white header + white sider floating on a light
// gray layout canvas. Pure data, no imports — main.jsx feeds it to
// <ConfigProvider theme={theme}>. Every color here has a CSS twin in the
// app.css :root block (--oc-* variables): raw CSS surfaces (DAG nodes,
// sheets, dividers) cannot read cssinjs tokens, so the two blocks must be
// changed together, in both directions.

/// Base colors shared with app.css :root (--oc-*). Keep in sync — change
/// one, change the other.
export const palette = {
  primary: '#1677ff', // classic antd blue — matches existing DAG/env accents
  bgLayout: '#f5f5f5', // page canvas behind the white panels
  panel: '#ffffff', // header / sider / card surface
};

/// antd ThemeConfig (v6). Component notes:
/// - Layout goes white-on-gray (headerBg/siderBg/bodyBg + colorBgLayout):
///   Content panels float on the canvas instead of sitting on white.
/// - Menu itemBg transparent keeps the light sider seamless (no inner box).
/// - antd 6 has no Table `size` component token, so default-sized tables
///   are normalized to middle density via the cell padding tokens.
/// - Card unifies body/header padding at 16 (default is paddingLG = 24).
export const theme = {
  token: {
    colorPrimary: palette.primary,
    borderRadius: 6,
    colorBgLayout: palette.bgLayout,
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
      itemBg: 'transparent',
    },
    Table: {
      cellPaddingBlock: 12,
      cellPaddingInline: 8,
    },
    Card: {
      bodyPadding: 16,
      headerPadding: 16,
    },
  },
};
