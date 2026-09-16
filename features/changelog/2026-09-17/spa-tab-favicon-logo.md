Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# SPA Chrome tab 图标改为 logo 图片

浏览器标签页图标从空 favicon（`<link rel="icon" href="data:,">`）换成 `logo/logo.png`（768×768）降采样出的 64×64 PNG，控制台标签页显示 OpenCoder logo。

## 改动

- `crates/web/spa/public/static/favicon.png`：新增（vite `public/` 拷贝源，64×64 PNG，LANCZOS，约 8 KB）。
- `crates/web/spa/dist/static/favicon.png`：同一文件，单二进制 `include_bytes!` 内嵌源。
- `crates/web/spa/index.html` 与 `spa/dist/index.html`：`<link rel="icon" type="image/png" href="./static/favicon.png" />`；`vite build` 重建的 dist/index.html 与手工编辑逐字节一致（已实测），app.js/app.css 保持提交版字节未动。
- `crates/web/src/html.rs`：`FAVICON_PNG` 内嵌 + `/static/:name` 白名单新增 `"favicon.png" => image/png`；`/static/` 前缀本就 auth-exempt，登录前即可加载。`/favicon.ico` 维持 404（无专属路由），control e2e `favicon_is_auth_exempt` 的 404 契约不受影响——Chrome 经 link 标签取图。

## 测试

- `cargo test -p opencoder-web --lib html::`：5 通过——`static_whitelist_serves_fixed_build_outputs` 扩展 favicon.png 断言（200 + image/png + 非空）；`shell_references_resolve_through_the_whitelist` 自动覆盖 shell 新引用（shell 引用的静态资源必须全过白名单）。
- `npx vite build`（spa/）：重建 dist/index.html 与提交版逐字节一致，确认源模板/白名单/dist 三方契约自洽。
