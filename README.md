# zhihu-collection-export

把知乎收藏夹导出为 Obsidian 友好的 Markdown 文件夹。

程序复用浏览器中已有的知乎登录态，直接调用知乎 Web API 分页读取收藏夹内容，把每条收藏渲染为独立的 Markdown 文件，并生成使用 Obsidian 双链的索引页。整个过程只读取数据，不修改知乎上的任何内容。

## 功能

- 支持回答、文章、想法和视频四类收藏条目，保留标题、作者、时间和原文链接。
- 生成 `00_index.md` 索引页，用 Obsidian 双链指向每条内容。
- 图片可保留外链、下载到本地或嵌入 Base64。
- 可选生成纯链接列表 `links.txt`。
- 只读取 `zhihu.com` 域下的 Cookie，不打印、不写入磁盘。

## 工作原理

程序从浏览器或命令行取得 `zhihu.com` 的 Cookie，然后请求知乎的分页接口：

```text
GET https://www.zhihu.com/api/v4/collections/{collection_id}/items?offset=0&limit=20
```

接口返回的条目转换为 Markdown 后写入本地目录。实现见[开发说明](docs/development.md)。

## 环境要求

- 一个已登录知乎的桌面浏览器，或一份有效的 Cookie header。
- 从源码构建时需要 Rust 1.88 或更高版本。

## 安装

### 预编译二进制

从 [Releases](https://github.com/AkiraLyu/zhihu-collection-export/releases/latest) 页面下载对应平台的压缩包：

| 平台 | 文件名后缀 |
| --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc.zip` |

Linux 上解压并试运行：

```bash
tar xzf zhihu-collection-export-v*-x86_64-unknown-linux-gnu.tar.gz
cd zhihu-collection-export-v*-x86_64-unknown-linux-gnu
./zhihu-collection-export --help
```

Windows 上解压压缩包，在解压出的目录中运行 `zhihu-collection-export.exe`。压缩包内附本 README。

### 从源码构建

需要 Rust 1.88 或更高版本：

```bash
git clone https://github.com/AkiraLyu/zhihu-collection-export.git
cd zhihu-collection-export
cargo build --release
```

生成的二进制文件为 `target/release/zhihu-collection-export`。

下文示例用 `zhihu-collection-export` 表示可执行文件。从源码构建时，也可以改用 `cargo run --release --` 运行。

## 快速开始

```bash
zhihu-collection-export 'https://www.zhihu.com/collection/997879559' -o exports
```

程序把结果写入 `exports/收藏夹名/`。收藏夹名取自接口返回的标题，标题不可用时改用收藏夹 ID。

位置参数接受收藏夹链接或纯数字 ID，两种写法等价：

```bash
zhihu-collection-export 997879559 -o exports
```

## 输出结构

```text
exports/
└── Linux/
    ├── 00_index.md
    ├── 01_如何入门 Linux.md
    ├── 02_常用命令速查.md
    ├── links.txt          # 使用 --export-links 时生成
    └── images/            # 使用 --images local 时生成
        └── <sha256>.jpg
```

各文件的字段和渲染规则见[输出格式](docs/output-format.md)。

## 常用选项

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `-o, --output <DIR>` | 当前目录 | 输出根目录，程序在其下创建收藏夹子目录 |
| `--images <MODE>` | `remote` | 图片处理方式：`remote`、`local` 或 `base64` |
| `--export-links` | 关闭 | 额外生成 `links.txt` |
| `--browser <BROWSER>` | `auto` | 读取 Cookie 的浏览器 |
| `--cookie <COOKIE>` | 无 | 直接传入 Cookie header，跳过浏览器读取 |
| `--diagnose-cookies` | 关闭 | 检查各浏览器的 Cookie 状态，不打印 Cookie 值 |
| `--limit <N>` | `20` | 每页请求条数，建议保持默认值 |
| `--delay-ms <MS>` | `800` | 分页请求之间的等待时间，同时作为重试退避基数 |
| `--retries <N>` | `3` | 瞬时请求错误的重试次数 |

完整参数说明见[命令行参考](docs/usage.md)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [命令行参考](docs/usage.md) | 全部参数、收藏夹标识解析、日志与退出码、限速与重试 |
| [输出格式](docs/output-format.md) | 目录结构、索引页、条目文件、图片与文件名规则 |
| [Cookie 获取](docs/cookies.md) | Cookie 来源优先级、浏览器支持、自定义 Profile、故障排查 |
| [开发说明](docs/development.md) | 模块结构、数据流、构建与测试 |

## 注意事项

- 只能导出当前 Cookie 有权限访问的内容。
- 默认要求 Cookie 中包含知乎登录态 `z_c0`，避免误用匿名 Cookie 后请求失败。确实需要匿名请求时加 `--allow-anonymous`。
- 知乎接口和风控策略可能变化。遇到 401、403、429 时，先确认浏览器已登录知乎，再降低请求频率，例如 `--delay-ms 2000`。
- 图片下载失败、响应不是图片或单张图片超过 50 MiB 时，程序保留原链接并继续导出其他内容。
- 视频条目的正文位置写入固定说明，导出文件只包含头部信息和原文链接。接口未返回内容的条目（已删除或不可见）标记为 `[内容不可用]`，同样保留在索引中。
- 请遵守知乎的服务条款，仅导出自己有权访问的内容。
