# zhihu-collection-export

把知乎收藏夹导出为 Obsidian 友好的 Markdown 文件夹。它参考油猴脚本的接口流程，直接分页请求：

```text
GET https://www.zhihu.com/api/v4/collections/{collection_id}/items?offset=0&limit=20
```

程序默认只读取浏览器里 `zhihu.com` 域名下的 Cookie，不打印、不写入 Cookie。支持 Chrome、Chromium、Edge、Brave、Firefox、LibreWolf、Vivaldi、Opera、Arc、Zen，macOS 上还支持 Safari。浏览器 Cookie 读取由 `rookie` crate 完成。

## 使用

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559'
```

指定输出目录：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' -o exports
```

导出结果会写到 `输出目录/收藏夹名/`。如果接口没有返回收藏夹名，则使用收藏夹 ID 作为文件夹名。文件夹内包含：

- `00_index.md`：目录页，使用 Obsidian 双链链接到各条内容。
- `01_标题.md`、`02_标题.md`：每条收藏内容一个 Markdown 文件。

指定浏览器：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' --browser chrome
```

诊断本机浏览器里是否有知乎登录 Cookie。这个命令只显示数量和是否有 `z_c0`，不会打印 Cookie 值：

```bash
cargo run -- --diagnose-cookies
```

如果自动读取 Cookie 失败，可以从浏览器 DevTools 里复制请求的 Cookie header：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' \
  --cookie 'z_c0=...; _xsrf=...; d_c0=...'
```

自定义浏览器 Profile：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' \
  --cookies-db '/absolute/path/to/Cookies' \
  --key-file '/absolute/path/to/Local State'
```

`--key-file` 主要用于 Windows Chromium 系浏览器；Firefox 通常只需要 `--cookies-db`。

## 注意

- 只能导出当前 Cookie 有权限访问的内容。
- 默认要求 Cookie 中包含知乎登录态 `z_c0`，避免误用匿名 Cookie 后请求失败。需要强行匿名请求时加 `--allow-anonymous`。
- 知乎接口和风控策略可能变化；遇到 401、403、429 时，先确认浏览器已登录知乎，再降低请求频率，例如 `--delay-ms 2000`。
- 视频、删除、不可见、接口未返回正文的条目会保留标题和链接，并在 Markdown 中标注原因。
