# 命令行参考

## 用法

```text
zhihu-collection-export [OPTIONS] [COLLECTION]
```

`COLLECTION` 是位置参数，可以省略，但省略后没有可导出的目标，程序会报错退出。唯一不需要该参数的情况是 `--diagnose-cookies`。

## 收藏夹标识

位置参数接受以下三种写法：

| 写法 | 示例 |
| --- | --- |
| 纯数字 ID | `997879559` |
| 收藏夹链接 | `https://www.zhihu.com/collection/997879559` |
| 含 `/collection/数字` 的任意字符串 | `https://www.zhihu.com/collection/997879559?utm_id=0` |

解析顺序为：内容全是 ASCII 数字时直接作为 ID；否则按 URL 解析路径，寻找 `collection` 之后紧跟数字的一段；仍失败时用正则 `/collection/(\d+)` 提取。三种方式都失败时报错：

```text
Error: 无法从参数中解析收藏夹 ID: not-a-collection
```

## 选项

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `-o, --output <DIR>` | 当前目录 | 输出根目录。程序在其下创建收藏夹子目录，目录名见[输出格式](output-format.md)。 |
| `--images <MODE>` | `remote` | 图片处理方式，可选 `remote`、`local`、`base64`。 |
| `--export-links` | 关闭 | 额外生成 `links.txt`，每行一个收藏条目的网页链接。 |
| `--browser <BROWSER>` | `auto` | 读取 Cookie 的浏览器，可选值见下。 |
| `--cookie <COOKIE>` | 无 | 直接传入 Cookie header。设置后跳过浏览器读取。 |
| `--cookies-db <PATH>` | 无 | 自定义浏览器 Cookie 数据库路径。 |
| `--key-file <PATH>` | 无 | Chromium `Local State` 文件路径，只能与 `--cookies-db` 一起使用。 |
| `--diagnose-cookies` | 关闭 | 显示各浏览器的 Cookie 可用性，不打印 Cookie 值。 |
| `--allow-anonymous` | 关闭 | 没有 `z_c0` 登录 Cookie 时继续执行。 |
| `--limit <N>` | `20` | 每页请求条数，取值范围 1 到 100。 |
| `--delay-ms <MS>` | `800` | 分页请求之间的等待时间，同时作为重试退避基数。 |
| `--retries <N>` | `3` | 瞬时请求错误的重试次数。 |

`--browser` 可选值：

```text
auto, chrome, chromium, edge, brave, firefox, libre-wolf,
vivaldi, opera, opera-gx, arc, zen, safari
```

`--limit` 超出 1 到 100 的范围时程序报错退出。知乎接口对较大的分页值会返回空结果，实测 25 及以上的值都会得到 0 条，程序据此把收藏夹当作空收藏夹处理，因此建议保持默认值 20。

Cookie 来源的优先级和浏览器选择逻辑见 [Cookie 获取](cookies.md)。

## 日志与退出码

进度信息写入标准错误，标准输出保留给 `--diagnose-cookies` 的结果。一次典型导出：

```text
使用 Firefox (/home/user/.config/mozilla/firefox/ab12cd34.default-release/cookies.sqlite) 的 zhihu.com Cookie（16 个，包含 登录 Cookie）
收藏夹共 128 条，开始分页导出
已处理 20/128
已处理 40/128
...
导出完成: exports/Linux
```

接口没有返回总数时，第二行改为 `未拿到总数，按分页结束标记导出`，进度行改为 `已处理 20`。

退出码只有两种：

| 退出码 | 含义 |
| --- | --- |
| `0` | 导出完成，或 `--diagnose-cookies` 至少从一个浏览器读到 Cookie。 |
| `1` | 出现错误，错误信息以 `Error: ` 开头写入标准错误。 |

以下情况会以退出码 1 结束：

- 收藏夹标识无法解析。
- 没有找到可用的 Cookie，或 Cookie 中缺少 `z_c0` 且未加 `--allow-anonymous`。
- 分页请求在重试后仍然失败。
- 创建输出目录、写入 Markdown 或写入图片失败。

以下情况只打印警告并继续导出：

- 收藏夹标题请求失败。程序改用收藏夹 ID 作为目录名。
- 单张图片下载失败、响应类型不是图片或超过 50 MiB。程序保留该图片的原始链接，继续处理后续条目。

## 限速与重试

程序在每个分页请求之间等待 `--delay-ms` 毫秒。HTTP 429 和 5xx 响应，以及网络层错误，会按 `--retries` 重试；单次请求的总尝试次数为 `--retries` 加一。第 n 次重试前等待 `--delay-ms × n` 毫秒，不足 300 毫秒时按 300 毫秒计算。其他 4xx 响应不重试，直接报错。

图片下载使用相同的重试次数与退避基数。获取收藏夹标题的请求固定重试 1 次，失败后静默降级为使用收藏夹 ID。

遇到 403 或 429 时，优先确认浏览器已登录知乎，再提高 `--delay-ms`，例如：

```bash
zhihu-collection-export 997879559 -o exports --delay-ms 2000
```

## 示例

导出到指定目录：

```bash
zhihu-collection-export 997879559 -o exports
```

下载图片到本地，便于离线查看：

```bash
zhihu-collection-export 997879559 -o exports --images local
```

把图片嵌入 Markdown，便于单文件携带：

```bash
zhihu-collection-export 997879559 -o exports --images base64
```

同时生成链接列表：

```bash
zhihu-collection-export 997879559 -o exports --export-links
```

指定浏览器，并放慢请求：

```bash
zhihu-collection-export 997879559 -o exports --browser chrome --delay-ms 2000
```

手动传入 Cookie：

```bash
zhihu-collection-export 997879559 \
  --cookie 'z_c0=...; _xsrf=...; d_c0=...'
```

检查本机浏览器的 Cookie 状态：

```bash
zhihu-collection-export --diagnose-cookies
```
