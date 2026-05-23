use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail};
use rookie::enums::Cookie as BrowserCookie;

use crate::cli::{BrowserChoice, Cli};

#[derive(Debug)]
pub(crate) struct CookieHeader {
    pub(crate) source: String,
    pub(crate) value: String,
    pub(crate) count: usize,
    pub(crate) has_login_cookie: bool,
}

pub(crate) fn resolve_cookie_header(cli: &Cli) -> Result<CookieHeader> {
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

pub(crate) fn diagnose_cookies(cli: &Cli) -> Result<()> {
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

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
