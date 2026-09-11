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

如需额外导出纯文本链接列表，添加 `--export-links`，会在收藏夹目录下生成 `links.txt`，每行一个收藏条目的网页链接：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' -o exports --export-links
```

图片默认保留外链。使用 `--images` 选择图片导出方式，支持回答、文章的 HTML 图片和想法中的图片块：

| 选项 | 导出方式 |
| --- | --- |
| `--images remote` | 默认值，保留图片 URL，不下载图片。 |
| `--images local` | 下载到收藏夹目录下的 `images/` 子文件夹，Markdown 使用相对路径。 |
| `--images base64` | 下载并转为 `data:image/...;base64,...`，直接嵌入 Markdown，不生成图片文件。 |

将图片保存到本地，便于在 Obsidian 中离线查看：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' -o exports --images local
```

输出示例（图片文件名使用内容的 SHA-256，避免同名覆盖，也方便重复图片共用文件）：

```text
exports/收藏夹名/
├── 00_index.md
├── 01_标题.md
└── images/
    └── <sha256>.jpg
```

将图片嵌入 Markdown，便于单文件携带：

```bash
cargo run --release -- 'https://www.zhihu.com/collection/997879559' -o exports --images base64
```

Base64 会增大 Markdown 文件体积，查看器需要支持 `data:` 图片链接。两种下载模式都会优先使用 HTML 的 `data-original`、`data-actualsrc`，最后才使用 `src`；同一图片 URL 在一次导出中只下载一次，已有的 `data:` 图片保持原样。

图片请求不携带知乎登录 Cookie。下载失败、响应不是图片或单张图片超过 50 MiB 时，会提示并保留原链接，继续导出其他内容；瞬时请求错误、HTTP 429 和 5xx 按 `--retries` 重试。本地目录创建或文件写入失败则会报错退出。

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
