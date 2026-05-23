mod cli;
mod cookies;
mod export;
mod markdown;
mod zhihu;

use anyhow::{Context, Result, anyhow};
use clap::Parser;

use crate::{
    cli::Cli,
    cookies::{diagnose_cookies, resolve_cookie_header},
    export::{collection_output_dir, export_collection, write_collection},
    zhihu::{fetch_collection_title, parse_collection_id, zhihu_client},
};

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

    let fetched_title = fetch_collection_title(&client, &collection_id).await;
    let title = fetched_title
        .clone()
        .unwrap_or_else(|| collection_id.to_string());

    let exported = export_collection(&client, &collection_id, &title, &cli).await?;
    let output_dir = collection_output_dir(
        cli.output.as_deref(),
        &collection_id,
        fetched_title.as_deref(),
    );
    write_collection(&output_dir, &exported, cli.export_links)
        .with_context(|| format!("写入导出目录失败: {}", output_dir.display()))?;

    eprintln!("导出完成: {}", output_dir.display());
    Ok(())
}
