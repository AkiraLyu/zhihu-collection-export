# Cookie 获取

知乎收藏夹接口需要登录态，程序必须先拿到一份包含 `z_c0` 的 `zhihu.com` Cookie。浏览器 Cookie 读取由 `rookie` crate 实现。

## 来源优先级

程序按以下顺序确定 Cookie 来源，先满足者生效：

| 顺序 | 条件 | 来源标记 |
| --- | --- | --- |
| 1 | 指定 `--cookie` | `--cookie` |
| 2 | 指定 `--cookies-db` | `custom cookie DB` |
| 3 | 读取浏览器 | 浏览器名称，Firefox 回退时附带数据库路径 |

因此 `--cookie` 与 `--cookies-db` 同时给出时，只有 `--cookie` 生效；`--cookies-db` 与 `--browser` 同时给出时，`--browser` 被忽略。`--key-file` 只能配合 `--cookies-db` 使用，单独使用时程序直接报错：

```text
Error: --key-file 只能和 --cookies-db 一起使用
```

## 浏览器读取

`--browser` 的可选值：

```text
auto, chrome, chromium, edge, brave, firefox, libre-wolf,
vivaldi, opera, opera-gx, arc, zen, safari
```

默认值 `auto` 会按 Chrome、Edge、Chromium、Brave、Firefox、LibreWolf、Vivaldi、Opera、Opera GX、Arc、Zen、Safari 的顺序逐个尝试，收集所有读取成功的候选，然后：

1. 优先选择包含 `z_c0` 的候选。
2. 多个候选都包含登录 Cookie 时，选择 Cookie 数量最多的。
3. 指定 `--allow-anonymous` 时使用排序后的第一个候选；否则没有候选包含 `z_c0` 就报错退出，并在错误信息中列出各浏览器的读取结果。

指定具体浏览器时只读取该浏览器，不会尝试其他浏览器。`firefox` 例外，它有下面的数据库回退流程。`safari` 只在 macOS 上可用，其他平台报错 `Safari Cookie 读取只支持 macOS`。

Firefox 有一个额外的回退流程：默认读取失败后，程序会扫描以下位置，从 `profiles.ini` 和子目录中查找 `cookies.sqlite`：

```text
~/.config/mozilla/firefox
~/.mozilla/firefox
~/.var/app/org.mozilla.firefox/.mozilla/firefox
~/snap/firefox/common/.mozilla/firefox
```

回退成功时，来源会显示为 `Firefox (/path/to/cookies.sqlite)`。

## Cookie 过滤规则

读取到的原始 Cookie 会按以下规则筛选：

- 只保留域名包含 `zhihu.com` 的 Cookie。
- 跳过名称为空的 Cookie。
- 跳过已经过期的 Cookie。
- 同名 Cookie 只保留一个。程序先按域名长度和路径长度从长到短排序，因此优先保留作用域更精确的那个。

筛选后没有剩余任何 Cookie 时报错：

```text
Error: Chrome 中没有可用的 zhihu.com Cookie
```

## 登录态要求

程序通过 Cookie 中是否存在 `z_c0` 判断登录态。缺少 `z_c0` 时默认报错退出：

```text
Error: Chrome 中没有 z_c0 登录 Cookie。知乎收藏夹接口通常需要登录态；请确认这个浏览器已登录知乎，或用 --cookie 手动传入包含 z_c0 的 Cookie header。需要强行匿名尝试时加 --allow-anonymous。
```

指定 `--allow-anonymous` 后跳过这项检查，继续尝试请求。匿名请求通常只能访问公开内容，可能返回 401 或 403。

## 自定义 Profile

浏览器使用非默认 Profile，或安装在非标准位置时，用 `--cookies-db` 直接指定 Cookie 数据库文件：

```bash
zhihu-collection-export 997879559 \
  --cookies-db '/absolute/path/to/profile/Cookies'
```

Chromium 系浏览器的数据库文件名为 `Cookies`，Firefox 为 `cookies.sqlite`。相对路径按当前工作目录解析。

Windows 上的 Chromium 系浏览器用 DPAPI 加密 Cookie，需要同时提供 `Local State` 文件：

```bash
zhihu-collection-export 997879559 \
  --cookies-db 'C:\Users\me\AppData\Local\Google\Chrome\User Data\Default\Network\Cookies' \
  --key-file 'C:\Users\me\AppData\Local\Google\Chrome\User Data\Local State'
```

Firefox 通常只需要 `--cookies-db`。`--key-file` 主要在使用 Windows 上的 Chromium 系浏览器时需要。

## 诊断

`--diagnose-cookies` 显示每个浏览器的 Cookie 数量和是否包含 `z_c0`，不打印 Cookie 值，也不需要位置参数：

```bash
zhihu-collection-export --diagnose-cookies
```

输出示例：

```text
Chrome                               读取失败: Chrome 中没有可用的 zhihu.com Cookie
Edge                                 读取失败: 读取 Edge Cookie 失败: can't find cookies file
Firefox (/home/user/.config/mozilla/firefox/ab12.default-release/cookies.sqlite) zhihu.com Cookie: 16, z_c0: yes
Safari                               读取失败: Safari Cookie 读取只支持 macOS
```

与正常导出不同，`auto` 在诊断模式下会逐个尝试所有浏览器并打印每个结果，而不是挑出最佳候选。至少一个浏览器读取成功时退出码为 0，全部失败时报错 `没有从任何浏览器读到 zhihu.com Cookie` 并以退出码 1 结束。

## 手动传入 Cookie

浏览器读取失败时，可以从 DevTools 复制请求头中的 Cookie 值：

1. 在浏览器中打开知乎，按 F12 打开开发者工具。
2. 切换到网络面板，刷新页面。
3. 选择任意一个 `zhihu.com` 请求，在请求头中找到 `Cookie`。
4. 复制完整值，通过 `--cookie` 传入。

```bash
zhihu-collection-export 997879559 \
  --cookie 'z_c0=...; _xsrf=...; d_c0=...'
```

`--diagnose-cookies` 也接受 `--cookie`，此时只统计传入值中的 Cookie 数量并检查是否包含 `z_c0=`，不校验其有效性。

## 隐私

- Cookie 只存在于进程内存中，不打印、不写入磁盘。
- 日志和诊断输出中只包含 Cookie 来源、数量和是否包含登录 Cookie，不含 Cookie 值。
- 图片请求使用独立的 HTTP 客户端，不携带 Cookie 和 Authorization 头，重定向到 CDN 后同样不携带。

## 故障排查

| 现象 | 可能原因 | 处理方式 |
| --- | --- | --- |
| `没有找到包含 z_c0 的知乎登录 Cookie` | 所有浏览器都没有知乎登录态，或登录态在未支持的位置 | 在浏览器中登录知乎；用 `--browser` 指定浏览器；运行 `--diagnose-cookies` 查看各浏览器状态 |
| `{浏览器} 中没有可用的 zhihu.com Cookie` | 该浏览器从未访问过知乎，或 Cookie 已过期 | 换用其他浏览器，或手动传入 `--cookie` |
| `读取 {浏览器} Cookie 失败: can't find cookies file` | 浏览器未安装，或使用了非默认 Profile | 用 `--cookies-db` 指定数据库路径 |
| `Safari Cookie 读取只支持 macOS` | 在非 macOS 平台指定了 `--browser safari` | 改用其他浏览器 |
| HTTP 401、403 | Cookie 失效或触发风控 | 重新登录知乎；提高 `--delay-ms` |
| HTTP 429 | 请求频率过高 | 提高 `--delay-ms`，例如 `--delay-ms 2000` |
| `--limit 必须在 1..=100 之间` | 分页大小超出允许范围 | 调整 `--limit`，或使用默认值 20 |
