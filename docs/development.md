# 开发说明

## 模块结构

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | 程序入口。解析参数，串联 Cookie 解析、标题获取、导出和写盘；`--diagnose-cookies` 在此分支。 |
| `src/cli.rs` | clap 派生的参数结构体、`BrowserChoice` 与 `ImageMode` 枚举及默认值。 |
| `src/cookies.rs` | Cookie 来源解析、浏览器读取（含 Firefox 回退）、过滤合并、登录态校验、诊断输出。 |
| `src/zhihu.rs` | HTTP 客户端构造、分页与标题请求、重试与退避、收藏夹 ID 解析、URL 归一化、响应结构体。 |
| `src/export.rs` | 分页遍历、条目渲染、输出目录命名、索引与链接列表生成、文件名清洗。 |
| `src/markdown.rs` | HTML 到 Markdown 的转换，图片地址提取与 Markdown 图片语法转义。 |
| `src/images.rs` | 图片下载与重试、按源 URL 缓存结果、三种图片输出模式。 |
| `src/test_support.rs` | 测试辅助：本地 TCP HTTP 服务器与图片样本，仅在 `cfg(test)` 下编译。 |

## 数据流

```text
Cli
├─ resolve_cookie_header()      → CookieHeader
├─ zhihu_client()               → reqwest::Client（默认携带 Cookie 与 Referer）
├─ fetch_collection_title()     → Option<String>，失败时降级为收藏夹 ID
├─ collection_output_dir()      → 输出目录
├─ export_collection()          → ExportedCollection
│  ├─ fetch_page() × N          → CollectionPage
│  ├─ render_item() × N         → ExportedItem
│  │  └─ ImageExporter          → 按需下载图片并缓存替换结果
│  └─ 计算序号宽度，填充 file_stem
└─ write_collection()           → 00_index.md、links.txt、NN_标题.md
```

条目先全部渲染到内存，再统一计算序号宽度并写盘，因此文件编号与接口返回顺序一致。

## 设计约束

- **API 客户端与图片客户端必须分离。** API 客户端默认携带 Cookie；图片客户端不携带 Cookie 和 Authorization，重定向到 CDN 后同样不携带。两者共用一个客户端会把登录态发送到任意的图片主机。
- **Cookie 只存在于内存。** 任何日志和输出都不包含 Cookie 值，只输出来源、数量和是否包含 `z_c0`。
- **可恢复与不可恢复错误分开处理。** 图片下载失败、响应类型不符、超出大小上限时保留原链接并继续；创建目录和写入文件失败时直接返回错误。
- **同一图片 URL 只请求一次。** `ImageExporter` 缓存每个源 URL 的最终替换结果，失败结果同样缓存，避免重复请求和重复警告。
- **接口字段按可选处理。** 字段缺失时回退到其他字段或降级输出，而不是让整次导出失败。

## 构建与运行

```bash
cargo build --release
cargo run --release -- 997879559 -o exports
```

`--` 之后的参数才会传给程序本身。`.cargo/config.toml` 设置了 `rustflags = ["--cfg=rustix_use_libc"]`，让 `rustix` 使用 libc 后端。

项目使用 edition 2024。依赖中要求最高的最低支持版本为 Rust 1.88，该值随依赖更新可能变化。

## 测试

```bash
cargo test
```

测试覆盖收藏夹 ID 解析、HTML 转 Markdown、三种图片模式、失败与重试、图片请求不携带 Cookie、索引与链接列表渲染。图片相关测试使用 `test_support` 中的本地 TCP 服务器，不访问网络。

## 依赖

| crate | 用途 |
| --- | --- |
| `anyhow` | 错误类型与上下文 |
| `base64` | Base64 图片编码 |
| `clap` | 命令行解析 |
| `kuchikiki` | HTML 解析与节点遍历 |
| `regex` | 从任意字符串中提取收藏夹 ID |
| `reqwest` | HTTP 客户端，使用 rustls |
| `rookie` | 浏览器 Cookie 读取 |
| `serde`、`serde_json` | 接口响应反序列化 |
| `sha2` | 图片文件名的内容摘要 |
| `tokio` | 异步运行时 |
| `url` | URL 解析 |
| `tempfile` | 测试用临时目录（开发依赖） |

## 扩展点

| 需求 | 位置 |
| --- | --- |
| 支持新的收藏内容类型 | `export.rs` 的 `item_url`，必要时配合 `render_item` |
| 支持新的图片格式 | `images.rs` 的 `sniff_image_type` 与扩展名映射 |
| 支持新的浏览器 | `cli.rs` 的 `BrowserChoice`、`cookies.rs` 的 `load_browser_cookies` 与 `auto_browser_order` |
| 调整正文转换规则 | `markdown.rs` 的 `convert_node` |
