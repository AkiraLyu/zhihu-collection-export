# 输出格式

## 目录结构

导出结果是一个以收藏夹命名的文件夹：

```text
exports/
└── Linux/
    ├── 00_index.md
    ├── 01_如何入门 Linux.md
    ├── 02_常用命令速查.md
    ├── links.txt
    └── images/
        └── 3f2a...c81.jpg
```

- `00_index.md` 始终生成。
- 每个收藏条目对应一个 Markdown 文件，按接口返回顺序编号。
- `links.txt` 只在指定 `--export-links` 时生成。
- `images/` 只在指定 `--images local` 且确实下载到图片时生成。

文件夹名取自接口返回的收藏夹标题，经文件名清洗后使用。标题获取失败或清洗后为空时，改用收藏夹 ID，例如 `997879559`。

## 索引页

`00_index.md` 的结构如下：

```markdown
# Linux

- 收藏夹链接: https://www.zhihu.com/collection/997879559
- 接口报告条目数: 128
- 导出条目数: 128

## 目录

1. [[01_如何入门 Linux|如何入门 Linux]]
2. [[02_常用命令速查|常用命令速查]]
```

`接口报告条目数` 只在接口返回总数时出现。`导出条目数` 是实际写入文件的条目数量，两者不一致通常意味着导出期间收藏夹内容发生了变化。

目录条目使用 Obsidian 双链，`[[` 和 `|` 之间是条目文件名（不含 `.md`），`|` 之后是显示名称。显示名称中的 `|`、`[`、`]` 会替换为全角字符 `｜`、`［`、`］`，避免破坏双链语法。

## 条目文件

每个条目文件由头部信息列表和正文组成：

```markdown
# 如何入门 Linux

- 序号: 1
- 类型: answer
- 原文链接: https://www.zhihu.com/question/123/answer/456
- 作者: 某某
- 作者主页: https://www.zhihu.com/people/someone
- 作者简介: 专注操作系统
- 创建时间: 1700000000
- 更新时间: 1700000000

正文……
```

除 `序号` 和 `类型` 外，所有字段只在该值存在时输出。`序号` 与文件名中的编号一致，从 1 开始。`类型` 是接口返回的内容类型，常见取值有 `answer`、`article`、`pin`、`zvideo`，接口未提供时写 `unknown`。

`创建时间` 和 `更新时间` 是接口返回的 Unix 秒级时间戳。程序不做时区或格式转换，直接输出原始数值。

`原文链接` 按内容类型构造：

| 类型 | 链接格式 |
| --- | --- |
| `answer` | `https://www.zhihu.com/question/{问题ID}/answer/{回答ID}` |
| `article` | `https://zhuanlan.zhihu.com/p/{文章ID}` |
| `zvideo` | `https://www.zhihu.com/zvideo/{视频ID}` |
| 其他 | 接口返回的 `url` 字段，缺省时回退到问题链接 |

前三种类型缺少构造所需的 ID 时，同样回退到最后一行。链接为 `//` 开头时补 `https:`，为 `/` 开头时补 `https://www.zhihu.com`。

同时缺少 ID 和 `url` 时不输出该字段，`links.txt` 中也会跳过这一条。

## 正文渲染

回答和文章的 `content` 是 HTML，程序将其转换为 Markdown：

| HTML | Markdown |
| --- | --- |
| `h1`–`h6` | `#`–`######` 标题 |
| `p` | 段落 |
| `br` | 换行 |
| `hr` | `---` |
| `strong`、`b` | `**粗体**` |
| `em`、`i` | `*斜体*` |
| `s`、`del` | `~~删除线~~` |
| `code` | 行内代码 |
| `pre` | 围栏代码块，内容取自纯文本 |
| `blockquote` | 引用，非空行加 `> `，空行输出 `>` |
| `a` | `[文本](链接)`，文本为空或与链接相同时输出 `<链接>` |
| `ul`、`ol`、`li` | `*` 和 `1.` 列表 |
| `img` | `![替代文本](图片地址)` |
| `script`、`style`、`noscript` | 丢弃 |
| 其他标签 | 保留子内容 |

正文中的链接和图片地址按原文链接一节的规则归一化。

想法条目的 `content` 是结构化块数组，程序逐块渲染：

- 块中包含 HTML 文本（`content` 或 `own_text` 字段）时，转换该段 HTML。
- 块类型为 `image` 时输出图片。
- 块包含链接且带标题时输出 `[标题](链接)`，只有链接时直接输出链接地址。

正文的处理规则：

- 视频条目固定写入 `视频条目通常不包含正文，已保留标题和链接。`，不渲染正文。
- 其他条目先渲染正文；渲染结果为空时改用接口返回的摘要 `excerpt`；摘要也为空时写入 `接口未返回正文。`。
- 接口完全没有返回内容的条目，标题记为 `[内容不可用]`，正文写入 `该收藏条目已删除、不可见，或接口没有返回内容。`。

转换后还会做少量清理：去除零宽空格、去除行尾空白、把连续空行压缩为一个。

## 链接列表

`links.txt` 每行一个原文链接，只包含能构造出链接的条目，非空时以换行符结尾。没有任何条目能构造出链接时，文件内容为空。

```text
https://www.zhihu.com/question/123/answer/456
https://zhuanlan.zhihu.com/p/789
```

## 图片

每个 `<img>` 的地址按 `data-original`、`data-actualsrc`、`src` 的顺序取第一个非空值。只有 `http` 和 `https` 地址会被下载，`data:` 等其他形式的地址保持原样。`pre`、`code`、`script`、`style`、`noscript` 内部的图片不发起下载；同一地址若在正文其他位置下载成功，`code` 内的引用仍会改用替换后的地址。

### remote 模式

默认模式。不发起图片请求，Markdown 中保留原始地址。

### local 模式

图片保存到收藏夹目录下的 `images/` 子文件夹，Markdown 使用相对路径：

```markdown
![封面](images/3f2a1b...c81.jpg)
```

文件名是图片字节的 SHA-256 摘要，扩展名根据响应类型确定。相同内容的图片共用一个文件，即使它们的原始 URL 不同。

### base64 模式

图片以 `data:{MIME};base64,{数据}` 形式直接嵌入 Markdown，不生成图片文件。这会让 Markdown 文件明显变大，且查看器必须支持 `data:` 图片链接。

### 类型识别

响应的 `Content-Type` 决定 MIME 类型和扩展名：

| MIME | 扩展名 |
| --- | --- |
| `image/jpeg`、`image/jpg` | `.jpg` |
| `image/png` | `.png` |
| `image/gif` | `.gif` |
| `image/webp` | `.webp` |
| `image/svg+xml` | `.svg` |
| `image/avif` | `.avif` |
| `image/bmp`、`image/x-ms-bmp` | `.bmp` |
| `image/tiff` | `.tiff` |
| `image/x-icon`、`image/vnd.microsoft.icon` | `.ico` |
| 其他 `image/*` | `.img` |

`Content-Type` 缺失或为 `application/octet-stream` 时，程序按文件头识别 PNG、JPEG、GIF、WebP、BMP、TIFF、ICO 和 AVIF。识别失败或响应类型不属于 `image/*` 时，视为下载失败。

### 失败处理

同一 URL 在一次导出中只请求一次，失败结果同样缓存，因此损坏的图片只会产生一条警告：

```text
图片下载失败，保留外链: https://pic.example/a.jpg (HTTP 404)
```

Markdown 中该图片保留原始地址，其他条目继续导出。

## 文件名规则

以下字符在文件名中替换为 `_`：

```text
/  \  :  *  ?  "  <  >  |  [  ]  #  ^  换行  回车  制表符
```

替换后去除首尾空白和句点，超过 80 个字符时截断。结果为空时使用 `zhihu_collection`。

条目文件名格式为 `序号_标题.md`。序号宽度取条目总数的十进制位数，最少 2 位：条目数不超过 99 时为 `01`，100 到 999 时为 `001`，依此类推。
