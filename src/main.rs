use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use kuchikiki::{NodeData, NodeRef, traits::TendrilSink};
use regex::Regex;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use rookie::enums::Cookie as BrowserCookie;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::sleep;
use url::Url;

const ZHIHU_HOST: &str = "https://www.zhihu.com";
const DEFAULT_LIMIT: u32 = 20;

#[derive(Parser, Debug)]
#[command(author, version, about = "Export a Zhihu collection to Markdown")]
struct Cli {
    /// Zhihu collection URL, for example https://www.zhihu.com/collection/997879559
    collection: Option<String>,

    /// Output Markdown file or output directory
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Browser to read zhihu.com cookies from
    #[arg(long, value_enum, default_value_t = BrowserChoice::Auto)]
    browser: BrowserChoice,

    /// Raw Cookie header. When set, browser cookie extraction is skipped.
    #[arg(long)]
    cookie: Option<String>,

    /// Show cookie availability without printing cookie values.
    #[arg(long)]
    diagnose_cookies: bool,

    /// Continue even when no z_c0 login cookie is found.
    #[arg(long)]
    allow_anonymous: bool,

    /// Absolute browser cookie DB path for custom profiles.
    #[arg(long)]
    cookies_db: Option<PathBuf>,

    /// Chromium Local State path, mainly needed for custom profiles on Windows.
    #[arg(long)]
    key_file: Option<PathBuf>,

    /// API page size. Zhihu currently works well with 20.
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    limit: u32,

    /// Delay between page requests.
    #[arg(long, default_value_t = 800)]
    delay_ms: u64,

    /// Retry count for transient HTTP errors.
    #[arg(long, default_value_t = 3)]
    retries: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BrowserChoice {
    Auto,
    Chrome,
    Chromium,
    Edge,
    Brave,
    Firefox,
    LibreWolf,
    Vivaldi,
    Opera,
    OperaGx,
    Arc,
    Zen,
    Safari,
}

#[derive(Debug)]
struct CookieHeader {
    source: String,
    value: String,
    count: usize,
    has_login_cookie: bool,
}

#[derive(Debug, Deserialize)]
struct CollectionPage {
    #[serde(default)]
    data: Vec<CollectionItem>,
    paging: Option<Paging>,
}

#[derive(Debug, Deserialize)]
struct Paging {
    totals: Option<u64>,
    is_end: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CollectionItem {
    content: Option<Content>,
}

#[derive(Debug, Deserialize)]
struct Content {
    id: Option<Value>,
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
    title: Option<String>,
    content: Option<String>,
    excerpt: Option<String>,
    question: Option<Question>,
    author: Option<Author>,
    created_time: Option<i64>,
    updated_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Question {
    id: Option<Value>,
    title: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Author {
    name: Option<String>,
    headline: Option<String>,
    url_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CollectionInfo {
    title: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    if cli.diagnose_cookies {
        return diagnose_cookies(&cli);
    }

    let collection = cli
        .collection
        .as_deref()
        .ok_or_else(|| anyhow!("缺少收藏夹链接。用法: zhihu-collection-export <收藏夹链接>"))?;
    let collection_id = parse_collection_id(collection)?;
    let cookie_header = resolve_cookie_header(&cli)?;
    let client = zhihu_client(&cookie_header.value, &collection_id)?;

    eprintln!(
        "使用 {} 的 zhihu.com Cookie（{} 个，{}登录 Cookie）",
        cookie_header.source,
        cookie_header.count,
        if cookie_header.has_login_cookie {
            "包含 "
        } else {
            "未发现 "
        }
    );

    let title = fetch_collection_title(&client, &collection_id)
        .await
        .unwrap_or_else(|| format!("知乎收藏夹 {}", collection_id));

    let markdown = export_collection(&client, &collection_id, &title, &cli).await?;
    let output = output_path(cli.output.as_deref(), &collection_id, &title)?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建输出目录失败: {}", parent.display()))?;
    }
    fs::write(&output, markdown).with_context(|| format!("写入文件失败: {}", output.display()))?;

    eprintln!("导出完成: {}", output.display());
    Ok(())
}

async fn export_collection(
    client: &Client,
    collection_id: &str,
    title: &str,
    cli: &Cli,
) -> Result<String> {
    if cli.limit == 0 || cli.limit > 100 {
        bail!("--limit 必须在 1..=100 之间");
    }

    let mut offset = 0;
    let mut total = None;
    let mut processed = 0usize;
    let mut sections = Vec::new();

    loop {
        let page = fetch_page(
            client,
            collection_id,
            offset,
            cli.limit,
            cli.retries,
            cli.delay_ms,
        )
        .await
        .with_context(|| format!("请求收藏夹分页失败: offset={offset}"))?;

        if total.is_none() {
            total = page.paging.as_ref().and_then(|paging| paging.totals);
            if let Some(total) = total {
                eprintln!("收藏夹共 {} 条，开始分页导出", total);
            } else {
                eprintln!("未拿到总数，按分页结束标记导出");
            }
        }

        if page.data.is_empty() {
            break;
        }

        for item in page.data {
            processed += 1;
            sections.push(render_item(processed, item));
        }

        if let Some(total) = total {
            eprintln!("已处理 {}/{}", processed, total);
        } else {
            eprintln!("已处理 {}", processed);
        }

        let is_end = page
            .paging
            .as_ref()
            .and_then(|paging| paging.is_end)
            .unwrap_or(false);
        if is_end || total.is_some_and(|total| processed as u64 >= total) {
            break;
        }

        offset += cli.limit;
        sleep(Duration::from_millis(cli.delay_ms)).await;
    }

    let mut output = String::new();
    output.push_str(&format!("# {}\n\n", title.trim()));
    output.push_str(&format!(
        "- 收藏夹链接: https://www.zhihu.com/collection/{}\n",
        collection_id
    ));
    output.push_str(&format!("- 导出条目数: {}\n\n", processed));
    output.push_str("---\n\n");
    output.push_str(&sections.join("\n---\n\n"));
    output.push('\n');
    Ok(output)
}

async fn fetch_page(
    client: &Client,
    collection_id: &str,
    offset: u32,
    limit: u32,
    retries: u32,
    delay_ms: u64,
) -> Result<CollectionPage> {
    let url = format!(
        "{ZHIHU_HOST}/api/v4/collections/{collection_id}/items?offset={offset}&limit={limit}"
    );
    fetch_json(client, &url, retries, delay_ms).await
}

async fn fetch_collection_title(client: &Client, collection_id: &str) -> Option<String> {
    let url = format!("{ZHIHU_HOST}/api/v4/collections/{collection_id}");
    let info = fetch_json::<CollectionInfo>(client, &url, 1, 300)
        .await
        .ok()?;
    let title = info.title?.trim().to_string();
    if title.is_empty() { None } else { Some(title) }
}

async fn fetch_json<T>(client: &Client, url: &str, retries: u32, delay_ms: u64) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let mut last_error = None;

    for attempt in 0..=retries {
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if status.is_success() {
                    return serde_json::from_str(&text).with_context(|| {
                        format!("解析 JSON 失败，响应前 200 字符: {}", excerpt(&text, 200))
                    });
                }

                let error = anyhow!(
                    "HTTP {}，响应前 200 字符: {}",
                    status.as_u16(),
                    excerpt(&text, 200)
                );
                if should_retry(status) && attempt < retries {
                    last_error = Some(error);
                } else {
                    return Err(error);
                }
            }
            Err(error) => {
                if attempt < retries {
                    last_error = Some(anyhow!(error));
                } else {
                    return Err(error).context("请求失败");
                }
            }
        }

        let backoff = delay_ms.saturating_mul((attempt + 1) as u64).max(300);
        sleep(Duration::from_millis(backoff)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow!("请求失败")))
}

fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn zhihu_client(cookie_header: &str, collection_id: &str) -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    headers.insert(
        REFERER,
        HeaderValue::from_str(&format!("{ZHIHU_HOST}/collection/{collection_id}"))?,
    );
    headers.insert(COOKIE, HeaderValue::from_str(cookie_header)?);

    Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .build()
        .context("创建 HTTP client 失败")
}

fn resolve_cookie_header(cli: &Cli) -> Result<CookieHeader> {
    let header = if let Some(raw_cookie) = cli.cookie.as_deref() {
        let cookie = raw_cookie.trim();
        if cookie.is_empty() {
            bail!("--cookie 不能为空");
        }
        CookieHeader {
            source: "--cookie".to_string(),
            value: cookie.to_string(),
            count: cookie.split(';').filter(|part| part.contains('=')).count(),
            has_login_cookie: cookie.contains("z_c0="),
        }
    } else if let Some(cookies_db) = cli.cookies_db.as_ref() {
        let cookies_db = absolute_path(cookies_db)?;
        let key_file = cli
            .key_file
            .as_ref()
            .map(|path| absolute_path(path))
            .transpose()?
            .map(|path| path.to_string_lossy().to_string());
        let domains = zhihu_domains();
        let cookies =
            rookie::any_browser(&cookies_db.to_string_lossy(), domains, key_file.as_deref())
                .map_err(|error| anyhow!("读取自定义 Cookie DB 失败: {error}"))?;
        build_cookie_header("custom cookie DB", cookies)?
    } else {
        if cli.key_file.is_some() {
            bail!("--key-file 只能和 --cookies-db 一起使用");
        }

        match cli.browser {
            BrowserChoice::Auto => best_auto_cookie_header(zhihu_domains(), !cli.allow_anonymous)?,
            browser => load_browser_cookie_header(browser, zhihu_domains())?,
        }
    };

    ensure_login_cookie(&header, cli.allow_anonymous)?;
    Ok(header)
}

fn best_auto_cookie_header(
    domains: Option<Vec<String>>,
    require_login: bool,
) -> Result<CookieHeader> {
    let mut candidates = Vec::new();
    for browser in auto_browser_order() {
        if let Ok(candidate) = load_browser_cookie_header(browser, domains.clone()) {
            candidates.push(candidate);
        }
    }

    candidates.sort_by(|left, right| {
        right
            .has_login_cookie
            .cmp(&left.has_login_cookie)
            .then_with(|| right.count.cmp(&left.count))
    });

    if require_login {
        let diagnostics = cookie_diagnostics(&candidates);
        return candidates
            .into_iter()
            .find(|candidate| candidate.has_login_cookie)
            .ok_or_else(|| {
                anyhow!(
                    "没有找到包含 z_c0 的知乎登录 Cookie。\n{}\n请先确认浏览器已登录知乎，或使用 --cookie 手动传入 Cookie header。也可以运行 --diagnose-cookies 查看各浏览器状态。",
                    diagnostics
                )
            });
    }

    candidates.into_iter().next().ok_or_else(|| {
        anyhow!("没有找到 zhihu.com Cookie。请先在浏览器登录知乎，或使用 --cookie 手动传入 Cookie header。")
    })
}

fn ensure_login_cookie(header: &CookieHeader, allow_anonymous: bool) -> Result<()> {
    if allow_anonymous || header.has_login_cookie {
        return Ok(());
    }

    bail!(
        "{} 中没有 z_c0 登录 Cookie。知乎收藏夹接口通常需要登录态；请确认这个浏览器已登录知乎，或用 --cookie 手动传入包含 z_c0 的 Cookie header。需要强行匿名尝试时加 --allow-anonymous。",
        header.source
    )
}

fn zhihu_domains() -> Option<Vec<String>> {
    Some(vec!["zhihu.com".to_string()])
}

fn diagnose_cookies(cli: &Cli) -> Result<()> {
    if let Some(raw_cookie) = cli.cookie.as_deref() {
        let cookie = raw_cookie.trim();
        if cookie.is_empty() {
            bail!("--cookie 不能为空");
        }
        let header = CookieHeader {
            source: "--cookie".to_string(),
            value: String::new(),
            count: cookie.split(';').filter(|part| part.contains('=')).count(),
            has_login_cookie: cookie.contains("z_c0="),
        };
        print_cookie_diagnostic(&header);
        return Ok(());
    }

    if let Some(cookies_db) = cli.cookies_db.as_ref() {
        let cookies_db = absolute_path(cookies_db)?;
        let key_file = cli
            .key_file
            .as_ref()
            .map(|path| absolute_path(path))
            .transpose()?
            .map(|path| path.to_string_lossy().to_string());
        let cookies = rookie::any_browser(
            &cookies_db.to_string_lossy(),
            zhihu_domains(),
            key_file.as_deref(),
        )
        .map_err(|error| anyhow!("读取自定义 Cookie DB 失败: {error}"))?;
        let header = build_cookie_header("custom cookie DB", cookies)?;
        print_cookie_diagnostic(&header);
        return Ok(());
    }

    if cli.key_file.is_some() {
        bail!("--key-file 只能和 --cookies-db 一起使用");
    }

    let browsers = if cli.browser == BrowserChoice::Auto {
        auto_browser_order()
    } else {
        vec![cli.browser]
    };

    let mut any_success = false;
    for browser in browsers {
        match load_browser_cookie_header(browser, zhihu_domains()) {
            Ok(header) => {
                any_success = true;
                print_cookie_diagnostic(&header);
            }
            Err(error) => {
                println!("{:<36} 读取失败: {}", format!("{browser:?}"), error);
            }
        }
    }

    if !any_success {
        bail!("没有从任何浏览器读到 zhihu.com Cookie");
    }

    Ok(())
}

fn print_cookie_diagnostic(header: &CookieHeader) {
    println!(
        "{:<36} zhihu.com Cookie: {:>2}, z_c0: {}",
        header.source,
        header.count,
        if header.has_login_cookie { "yes" } else { "no" }
    );
}

fn cookie_diagnostics(candidates: &[CookieHeader]) -> String {
    if candidates.is_empty() {
        return "已尝试的浏览器都没有读到 zhihu.com Cookie。".to_string();
    }

    let details = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}: {} 个 Cookie, z_c0={}",
                candidate.source,
                candidate.count,
                if candidate.has_login_cookie {
                    "yes"
                } else {
                    "no"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("已读取: {details}")
}

fn auto_browser_order() -> Vec<BrowserChoice> {
    vec![
        BrowserChoice::Chrome,
        BrowserChoice::Edge,
        BrowserChoice::Chromium,
        BrowserChoice::Brave,
        BrowserChoice::Firefox,
        BrowserChoice::LibreWolf,
        BrowserChoice::Vivaldi,
        BrowserChoice::Opera,
        BrowserChoice::OperaGx,
        BrowserChoice::Arc,
        BrowserChoice::Zen,
        BrowserChoice::Safari,
    ]
}

fn load_browser_cookie_header(
    browser: BrowserChoice,
    domains: Option<Vec<String>>,
) -> Result<CookieHeader> {
    let (source, cookies) = load_browser_cookies(browser, domains)?;
    build_cookie_header(&source, cookies)
}

fn load_browser_cookies(
    browser: BrowserChoice,
    domains: Option<Vec<String>>,
) -> Result<(String, Vec<BrowserCookie>)> {
    let source = format!("{browser:?}");
    let cookies = match browser {
        BrowserChoice::Auto => unreachable!("auto is handled separately"),
        BrowserChoice::Chrome => rookie::chrome(domains),
        BrowserChoice::Chromium => rookie::chromium(domains),
        BrowserChoice::Edge => rookie::edge(domains),
        BrowserChoice::Brave => rookie::brave(domains),
        BrowserChoice::Firefox => return load_firefox_cookies(domains),
        BrowserChoice::LibreWolf => rookie::librewolf(domains),
        BrowserChoice::Vivaldi => rookie::vivaldi(domains),
        BrowserChoice::Opera => rookie::opera(domains),
        BrowserChoice::OperaGx => rookie::opera_gx(domains),
        BrowserChoice::Arc => rookie::arc(domains),
        BrowserChoice::Zen => rookie::zen(domains),
        BrowserChoice::Safari => {
            #[cfg(target_os = "macos")]
            {
                rookie::safari(domains)
            }
            #[cfg(not(target_os = "macos"))]
            {
                bail!("Safari Cookie 读取只支持 macOS");
            }
        }
    }
    .map_err(|error| anyhow!("读取 {browser:?} Cookie 失败: {error}"))?;

    Ok((source, cookies))
}

fn load_firefox_cookies(domains: Option<Vec<String>>) -> Result<(String, Vec<BrowserCookie>)> {
    match rookie::firefox(domains.clone()) {
        Ok(cookies) => return Ok(("Firefox".to_string(), cookies)),
        Err(primary_error) => {
            let candidates = firefox_cookie_db_candidates();
            if candidates.is_empty() {
                return Err(anyhow!(
                    "读取 Firefox Cookie 失败: {primary_error}; 未发现 fallback cookies.sqlite"
                ));
            }

            let mut errors = Vec::new();
            for path in candidates {
                match rookie::firefox_based(path.clone(), domains.clone()) {
                    Ok(cookies) => {
                        return Ok((format!("Firefox ({})", path.display()), cookies));
                    }
                    Err(error) => errors.push(format!("{}: {}", path.display(), error)),
                }
            }

            Err(anyhow!(
                "读取 Firefox Cookie 失败: {primary_error}; fallback 也失败: {}",
                errors.join(" | ")
            ))
        }
    }
}

fn firefox_cookie_db_candidates() -> Vec<PathBuf> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };

    let bases = [
        home.join(".config/mozilla/firefox"),
        home.join(".mozilla/firefox"),
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ];

    let mut seen = HashSet::new();
    let mut dbs = Vec::new();
    for base in bases {
        collect_firefox_cookie_dbs(&base, &mut seen, &mut dbs);
    }
    dbs
}

fn collect_firefox_cookie_dbs(base: &Path, seen: &mut HashSet<PathBuf>, dbs: &mut Vec<PathBuf>) {
    if !base.exists() {
        return;
    }

    let profiles_ini = base.join("profiles.ini");
    if let Ok(text) = fs::read_to_string(&profiles_ini) {
        for line in text.lines().map(str::trim) {
            let Some(profile_path) = line.strip_prefix("Path=") else {
                continue;
            };
            let profile_path = PathBuf::from(profile_path.trim());
            let profile_path = if profile_path.is_absolute() {
                profile_path
            } else {
                base.join(profile_path)
            };
            push_cookie_db(profile_path.join("cookies.sqlite"), seen, dbs);
        }
    }

    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                push_cookie_db(path.join("cookies.sqlite"), seen, dbs);
            }
        }
    }
}

fn push_cookie_db(path: PathBuf, seen: &mut HashSet<PathBuf>, dbs: &mut Vec<PathBuf>) {
    if path.exists() && seen.insert(path.clone()) {
        dbs.push(path);
    }
}

fn build_cookie_header(source: &str, cookies: Vec<BrowserCookie>) -> Result<CookieHeader> {
    let now = unix_now();
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    let mut has_login_cookie = false;

    let mut cookies = cookies;
    cookies.sort_by(|left, right| {
        right
            .domain
            .trim_start_matches('.')
            .len()
            .cmp(&left.domain.trim_start_matches('.').len())
            .then_with(|| right.path.len().cmp(&left.path.len()))
    });

    for cookie in cookies {
        if !cookie.domain.contains("zhihu.com") || cookie.name.is_empty() {
            continue;
        }
        if cookie.expires.is_some_and(|expires| expires <= now) {
            continue;
        }
        if !seen.insert(cookie.name.clone()) {
            continue;
        }
        if cookie.name == "z_c0" {
            has_login_cookie = true;
        }
        pairs.push(format!("{}={}", cookie.name, cookie.value));
    }

    if pairs.is_empty() {
        bail!("{source} 中没有可用的 zhihu.com Cookie");
    }

    Ok(CookieHeader {
        source: source.to_string(),
        value: pairs.join("; "),
        count: pairs.len(),
        has_login_cookie,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_collection_id(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }

    if let Ok(url) = Url::parse(trimmed) {
        for pair in url
            .path_segments()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .windows(2)
        {
            if pair[0] == "collection" && pair[1].chars().all(|ch| ch.is_ascii_digit()) {
                return Ok(pair[1].to_string());
            }
        }
    }

    let re = Regex::new(r"/collection/(\d+)").expect("valid regex");
    re.captures(trimmed)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
        .ok_or_else(|| anyhow!("无法从参数中解析收藏夹 ID: {input}"))
}

fn render_item(index: usize, item: CollectionItem) -> String {
    let Some(content) = item.content else {
        return format!(
            "## {}. [内容不可用]\n\n该收藏条目已删除、不可见，或接口没有返回内容。\n",
            index
        );
    };

    let title = item_title(&content);
    let item_url = item_url(&content);
    let kind = content.kind.as_deref().unwrap_or("unknown");

    let mut output = String::new();
    output.push_str(&format!("## {}. {}\n\n", index, title));
    output.push_str(&format!("- 类型: {}\n", kind));
    if let Some(url) = item_url.as_deref() {
        output.push_str(&format!("- 原文链接: {}\n", url));
    }
    if let Some(author) = content
        .author
        .as_ref()
        .and_then(|author| author.name.as_deref())
    {
        output.push_str(&format!("- 作者: {}\n", author.trim()));
    }
    if let Some(author_url) = content.author.as_ref().and_then(author_url) {
        output.push_str(&format!("- 作者主页: {}\n", author_url));
    }
    if let Some(headline) = content
        .author
        .as_ref()
        .and_then(|author| author.headline.as_deref())
        .map(str::trim)
        .filter(|headline| !headline.is_empty())
    {
        output.push_str(&format!("- 作者简介: {}\n", headline));
    }
    if let Some(created_time) = content.created_time {
        output.push_str(&format!("- 创建时间: {}\n", created_time));
    }
    if let Some(updated_time) = content.updated_time {
        output.push_str(&format!("- 更新时间: {}\n", updated_time));
    }
    output.push('\n');

    if kind == "zvideo" {
        output.push_str("视频条目通常不包含正文，已保留标题和链接。\n");
        return output;
    }

    if let Some(html) = content
        .content
        .as_deref()
        .filter(|html| !html.trim().is_empty())
    {
        output.push_str(&html_to_markdown(html));
        output.push('\n');
    } else if let Some(excerpt) = content
        .excerpt
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        output.push_str(excerpt.trim());
        output.push('\n');
    } else {
        output.push_str("接口未返回正文。\n");
    }

    output
}

fn item_title(content: &Content) -> String {
    content
        .title
        .as_deref()
        .or_else(|| {
            content
                .question
                .as_ref()
                .and_then(|question| question.title.as_deref())
        })
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("无标题")
        .to_string()
}

fn author_url(author: &Author) -> Option<String> {
    let token = author.url_token.as_deref()?.trim();
    if token.is_empty() {
        None
    } else {
        Some(format!("{ZHIHU_HOST}/people/{token}"))
    }
}

fn item_url(content: &Content) -> Option<String> {
    let kind = content.kind.as_deref();
    match kind {
        Some("answer") => {
            let question_id = content
                .question
                .as_ref()
                .and_then(|question| question.id.as_ref())
                .and_then(value_to_string);
            let answer_id = content.id.as_ref().and_then(value_to_string);
            if let (Some(question_id), Some(answer_id)) = (question_id, answer_id) {
                return Some(format!(
                    "{ZHIHU_HOST}/question/{question_id}/answer/{answer_id}"
                ));
            }
        }
        Some("article") => {
            if let Some(id) = content.id.as_ref().and_then(value_to_string) {
                return Some(format!("https://zhuanlan.zhihu.com/p/{id}"));
            }
        }
        Some("zvideo") => {
            if let Some(id) = content.id.as_ref().and_then(value_to_string) {
                return Some(format!("{ZHIHU_HOST}/zvideo/{id}"));
            }
        }
        _ => {}
    }

    content
        .url
        .as_deref()
        .or_else(|| {
            content
                .question
                .as_ref()
                .and_then(|question| question.url.as_deref())
        })
        .map(normalize_zhihu_url)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }
}

fn normalize_zhihu_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.starts_with('/') {
        format!("{ZHIHU_HOST}{raw}")
    } else {
        raw.to_string()
    }
}

fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }

    let document = kuchikiki::parse_html().one(format!("<!doctype html><body>{html}</body>"));
    let body = document
        .select_first("body")
        .ok()
        .map(|node| node.as_node().clone())
        .unwrap_or(document);
    let markdown = convert_children(&body);
    cleanup_markdown(&markdown)
}

fn convert_children(node: &NodeRef) -> String {
    node.children()
        .map(|child| convert_node(&child))
        .collect::<Vec<_>>()
        .join("")
}

fn convert_node(node: &NodeRef) -> String {
    match node.data() {
        NodeData::Text(text) => text.borrow().to_string(),
        NodeData::Element(element) => {
            let tag = element.name.local.to_string();
            let content = convert_children(node);

            match tag.as_str() {
                "script" | "style" | "noscript" => String::new(),
                "p" => paragraph(&content),
                "br" => "\n".to_string(),
                "hr" => "\n---\n\n".to_string(),
                "strong" | "b" => wrap_inline("**", &content),
                "em" | "i" => wrap_inline("*", &content),
                "s" | "del" => wrap_inline("~~", &content),
                "code" => format!("`{}`", content.trim().replace('`', "\\`")),
                "pre" => format!("\n```\n{}\n```\n\n", node.text_contents().trim()),
                "blockquote" => blockquote(&content),
                "a" => link(element, &content),
                "img" => image(element),
                "ul" => unordered_list(node),
                "ol" => ordered_list(node),
                "li" => format!("* {}\n", cleanup_inline(&content)),
                "h1" => heading(1, &content),
                "h2" => heading(2, &content),
                "h3" => heading(3, &content),
                "h4" => heading(4, &content),
                "h5" => heading(5, &content),
                "h6" => heading(6, &content),
                "figure" | "figcaption" | "div" | "span" | "main" | "body" | "html" => content,
                _ => content,
            }
        }
        _ => String::new(),
    }
}

fn paragraph(content: &str) -> String {
    let content = content.trim();
    if content.is_empty() {
        String::new()
    } else {
        format!("{content}\n\n")
    }
}

fn heading(level: usize, content: &str) -> String {
    let content = cleanup_inline(content);
    if content.is_empty() {
        String::new()
    } else {
        format!("{} {}\n\n", "#".repeat(level), content)
    }
}

fn wrap_inline(marker: &str, content: &str) -> String {
    let content = content.trim();
    if content.is_empty() {
        String::new()
    } else {
        format!("{marker}{content}{marker}")
    }
}

fn link(element: &kuchikiki::ElementData, content: &str) -> String {
    let attrs = element.attributes.borrow();
    let Some(href) = attrs.get("href").map(normalize_zhihu_url) else {
        return content.to_string();
    };
    let label = cleanup_inline(content);
    if label.is_empty() || label == href {
        format!("<{}>", href)
    } else {
        format!("[{}]({})", label, href)
    }
}

fn image(element: &kuchikiki::ElementData) -> String {
    let attrs = element.attributes.borrow();
    let src = attrs
        .get("data-original")
        .or_else(|| attrs.get("data-actualsrc"))
        .or_else(|| attrs.get("src"))
        .map(normalize_zhihu_url);
    let Some(src) = src else {
        return String::new();
    };
    let alt = attrs.get("alt").unwrap_or("图片").trim();
    format!(
        "![{}]({})\n\n",
        if alt.is_empty() { "图片" } else { alt },
        src
    )
}

fn blockquote(content: &str) -> String {
    let content = cleanup_markdown(content);
    if content.is_empty() {
        return String::new();
    }
    let quoted = content
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                ">".to_string()
            } else {
                format!("> {}", line.trim_end())
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{quoted}\n\n")
}

fn unordered_list(node: &NodeRef) -> String {
    let mut output = String::new();
    for child in node.children() {
        if element_tag(&child).as_deref() == Some("li") {
            let content = cleanup_inline(&convert_children(&child));
            if !content.is_empty() {
                output.push_str(&format!("* {content}\n"));
            }
        } else {
            output.push_str(&convert_node(&child));
        }
    }
    if output.is_empty() {
        output
    } else {
        output.push('\n');
        output
    }
}

fn ordered_list(node: &NodeRef) -> String {
    let mut output = String::new();
    let mut index = 1;
    for child in node.children() {
        if element_tag(&child).as_deref() == Some("li") {
            let content = cleanup_inline(&convert_children(&child));
            if !content.is_empty() {
                output.push_str(&format!("{index}. {content}\n"));
                index += 1;
            }
        } else {
            output.push_str(&convert_node(&child));
        }
    }
    if output.is_empty() {
        output
    } else {
        output.push('\n');
        output
    }
}

fn element_tag(node: &NodeRef) -> Option<String> {
    match node.data() {
        NodeData::Element(element) => Some(element.name.local.to_string()),
        _ => None,
    }
}

fn cleanup_inline(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn cleanup_markdown(input: &str) -> String {
    let mut output = String::new();
    let mut blank_lines = 0;

    for line in input.replace('\u{200b}', "").lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            blank_lines += 1;
            if blank_lines <= 1 && !output.is_empty() {
                output.push('\n');
            }
        } else {
            blank_lines = 0;
            output.push_str(trimmed_end);
            output.push('\n');
        }
    }

    output.trim().to_string()
}

fn output_path(output: Option<&Path>, collection_id: &str, title: &str) -> Result<PathBuf> {
    let file_name = format!("{}_{}.md", sanitize_filename(title), collection_id);
    match output {
        Some(path) if path.extension().is_some_and(|extension| extension == "md") => {
            Ok(path.to_path_buf())
        }
        Some(path) => Ok(path.join(file_name)),
        None => Ok(PathBuf::from(file_name)),
    }
}

fn sanitize_filename(input: &str) -> String {
    let mut output = String::new();
    for ch in input.chars() {
        if matches!(
            ch,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\r' | '\n' | '\t'
        ) {
            output.push('_');
        } else {
            output.push(ch);
        }
    }
    let output = output.trim().trim_matches('.').to_string();
    if output.is_empty() {
        "zhihu_collection".to_string()
    } else {
        output.chars().take(80).collect()
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn excerpt(input: &str, max_chars: usize) -> String {
    let cleaned = input.replace(['\n', '\r', '\t'], " ");
    let mut output = cleaned.chars().take(max_chars).collect::<String>();
    if cleaned.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_collection_id_from_url() {
        assert_eq!(
            parse_collection_id("https://www.zhihu.com/collection/997879559").unwrap(),
            "997879559"
        );
        assert_eq!(parse_collection_id("997879559").unwrap(), "997879559");
    }

    #[test]
    fn converts_basic_zhihu_html_to_markdown() {
        let html = r#"<p>你好 <strong>世界</strong></p><blockquote><p>引用</p></blockquote><p><img data-original="//pic.example/a.jpg"></p>"#;
        let markdown = html_to_markdown(html);
        assert!(markdown.contains("你好 **世界**"));
        assert!(markdown.contains("> 引用"));
        assert!(markdown.contains("![图片](https://pic.example/a.jpg)"));
    }

    #[test]
    fn renders_answer_url_from_ids() {
        let content = Content {
            id: Some(Value::from(456)),
            kind: Some("answer".to_string()),
            url: None,
            title: None,
            content: None,
            excerpt: None,
            question: Some(Question {
                id: Some(Value::from(123)),
                title: Some("问题".to_string()),
                url: None,
            }),
            author: None,
            created_time: None,
            updated_time: None,
        };
        assert_eq!(
            item_url(&content).unwrap(),
            "https://www.zhihu.com/question/123/answer/456"
        );
    }
}
