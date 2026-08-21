use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::Value;
use tokio::time::sleep;

use crate::{
    cli::Cli,
    markdown::html_to_markdown,
    zhihu::{Author, CollectionItem, Content, ZHIHU_HOST, fetch_page, normalize_zhihu_url},
};

pub(crate) struct ExportedCollection {
    title: String,
    collection_id: String,
    total: Option<u64>,
    items: Vec<ExportedItem>,
}

struct ExportedItem {
    index: usize,
    title: String,
    url: Option<String>,
    file_stem: String,
    markdown: String,
}

pub(crate) async fn export_collection(
    client: &Client,
    collection_id: &str,
    title: &str,
    cli: &Cli,
) -> Result<ExportedCollection> {
    if cli.limit == 0 || cli.limit > 100 {
        bail!("--limit 必须在 1..=100 之间");
    }

    let mut offset = 0;
    let mut total = None;
    let mut processed = 0usize;
    let mut items = Vec::new();

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
            items.push(render_item(processed, item));
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

    let width = item_number_width(items.len());
    for item in &mut items {
        item.file_stem = item_file_stem(item.index, &item.title, width);
    }

    Ok(ExportedCollection {
        title: title.trim().to_string(),
        collection_id: collection_id.to_string(),
        total,
        items,
    })
}

fn render_item(index: usize, item: CollectionItem) -> ExportedItem {
    let Some(content) = item.content else {
        let title = "[内容不可用]".to_string();
        return ExportedItem {
            index,
            title: title.clone(),
            url: None,
            file_stem: String::new(),
            markdown: format!("# {title}\n\n该收藏条目已删除、不可见，或接口没有返回内容。\n"),
        };
    };

    let title = item_title(&content);
    let item_url = item_url(&content);
    let kind = content.kind.as_deref().unwrap_or("unknown");

    let mut output = String::new();
    output.push_str(&format!("# {}\n\n", title));
    output.push_str(&format!("- 序号: {}\n", index));
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
        return ExportedItem {
            index,
            title,
            url: item_url,
            file_stem: String::new(),
            markdown: output,
        };
    }

    if let Some(markdown) = content.content.as_ref().and_then(content_to_markdown) {
        output.push_str(&markdown);
        if !markdown.ends_with('\n') {
            output.push('\n');
        }
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

    ExportedItem {
        index,
        title,
        url: item_url,
        file_stem: String::new(),
        markdown: output,
    }
}

fn content_to_markdown(content: &Value) -> Option<String> {
    match content {
        Value::String(html) if !html.trim().is_empty() => Some(html_to_markdown(html)),
        Value::Array(blocks) => {
            let rendered = blocks
                .iter()
                .filter_map(render_content_block)
                .collect::<Vec<_>>()
                .join("\n\n");
            if rendered.trim().is_empty() {
                None
            } else {
                Some(rendered)
            }
        }
        _ => None,
    }
}

fn render_content_block(block: &Value) -> Option<String> {
    let block = block.as_object()?;
    let kind = block.get("type").and_then(Value::as_str);

    if let Some(html) = block
        .get("content")
        .or_else(|| block.get("own_text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|html| !html.is_empty())
    {
        let markdown = html_to_markdown(html);
        if !markdown.trim().is_empty() {
            return Some(markdown.trim().to_string());
        }
    }

    let url = block
        .get("url")
        .or_else(|| block.get("original_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())?;
    let url = normalize_zhihu_url(url);
    let title = block
        .get("title")
        .or_else(|| block.get("data_draft_title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty());

    if matches!(kind, Some("image")) {
        Some(format!("![{}]({})", title.unwrap_or("图片"), url))
    } else if let Some(title) = title {
        Some(format!("[{}]({})", title, url))
    } else {
        Some(url)
    }
}

fn item_title(content: &Content) -> String {
    content
        .title
        .as_deref()
        .or(content.excerpt_title.as_deref())
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

pub(crate) fn collection_output_dir(
    output_root: Option<&Path>,
    collection_id: &str,
    title: Option<&str>,
) -> PathBuf {
    let folder_name = title
        .map(sanitize_filename)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| collection_id.to_string());

    match output_root {
        Some(root) => root.join(folder_name),
        None => PathBuf::from(folder_name),
    }
}

pub(crate) fn write_collection(
    output_dir: &Path,
    collection: &ExportedCollection,
    export_links: bool,
) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;
    fs::write(output_dir.join("00_index.md"), render_index(collection))
        .with_context(|| format!("写入索引失败: {}", output_dir.join("00_index.md").display()))?;
    if export_links {
        fs::write(output_dir.join("links.txt"), render_links_txt(collection)).with_context(
            || {
                format!(
                    "写入链接列表失败: {}",
                    output_dir.join("links.txt").display()
                )
            },
        )?;
    }

    for item in &collection.items {
        let path = output_dir.join(format!("{}.md", item.file_stem));
        fs::write(&path, &item.markdown)
            .with_context(|| format!("写入条目失败: {}", path.display()))?;
    }

    Ok(())
}

fn render_index(collection: &ExportedCollection) -> String {
    let mut output = String::new();
    output.push_str(&format!("# {}\n\n", collection.title));
    output.push_str(&format!(
        "- 收藏夹链接: https://www.zhihu.com/collection/{}\n",
        collection.collection_id
    ));
    if let Some(total) = collection.total {
        output.push_str(&format!("- 接口报告条目数: {}\n", total));
    }
    output.push_str(&format!("- 导出条目数: {}\n\n", collection.items.len()));
    output.push_str("## 目录\n\n");

    for item in &collection.items {
        output.push_str(&format!(
            "{}. [[{}|{}]]\n",
            item.index,
            item.file_stem,
            obsidian_alias(&item.title)
        ));
    }

    output
}

fn render_links_txt(collection: &ExportedCollection) -> String {
    let mut output = collection
        .items
        .iter()
        .filter_map(|item| item.url.as_deref())
        .filter(|url| !url.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn item_number_width(item_count: usize) -> usize {
    item_count.max(1).to_string().len().max(2)
}

fn item_file_stem(index: usize, title: &str, width: usize) -> String {
    format!(
        "{:0width$}_{}",
        index,
        sanitize_filename(title),
        width = width
    )
}

fn obsidian_alias(title: &str) -> String {
    let alias = title
        .trim()
        .replace('|', "｜")
        .replace('[', "［")
        .replace(']', "］");
    if alias.is_empty() {
        "无标题".to_string()
    } else {
        alias
    }
}

fn sanitize_filename(input: &str) -> String {
    let mut output = String::new();
    for ch in input.chars() {
        if matches!(
            ch,
            '/' | '\\'
                | ':'
                | '*'
                | '?'
                | '"'
                | '<'
                | '>'
                | '|'
                | '['
                | ']'
                | '#'
                | '^'
                | '\r'
                | '\n'
                | '\t'
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zhihu::Question;

    #[test]
    fn renders_answer_url_from_ids() {
        let content = Content {
            id: Some(Value::from(456)),
            kind: Some("answer".to_string()),
            url: None,
            title: None,
            excerpt_title: None,
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

    #[test]
    fn chooses_collection_folder_from_title_or_id() {
        assert_eq!(
            collection_output_dir(Some(Path::new("exports")), "997879559", Some("Linux/OS"))
                .to_string_lossy(),
            "exports/Linux_OS"
        );
        assert_eq!(
            collection_output_dir(Some(Path::new("exports")), "997879559", None).to_string_lossy(),
            "exports/997879559"
        );
    }

    #[test]
    fn renders_obsidian_index_links() {
        let collection = ExportedCollection {
            title: "收藏夹".to_string(),
            collection_id: "123".to_string(),
            total: Some(1),
            items: vec![ExportedItem {
                index: 1,
                title: "A [B] | C".to_string(),
                url: Some("https://zhuanlan.zhihu.com/p/1".to_string()),
                file_stem: "01_A _B_ _ C".to_string(),
                markdown: "# A".to_string(),
            }],
        };
        let index = render_index(&collection);
        assert!(index.contains("[[01_A _B_ _ C|A ［B］ ｜ C]]"));
    }

    #[test]
    fn renders_plain_links_txt() {
        let collection = ExportedCollection {
            title: "收藏夹".to_string(),
            collection_id: "123".to_string(),
            total: Some(3),
            items: vec![
                ExportedItem {
                    index: 1,
                    title: "A".to_string(),
                    url: Some("https://zhuanlan.zhihu.com/p/1".to_string()),
                    file_stem: "01_A".to_string(),
                    markdown: "# A".to_string(),
                },
                ExportedItem {
                    index: 2,
                    title: "B".to_string(),
                    url: None,
                    file_stem: "02_B".to_string(),
                    markdown: "# B".to_string(),
                },
                ExportedItem {
                    index: 3,
                    title: "C".to_string(),
                    url: Some("https://www.zhihu.com/question/1/answer/2".to_string()),
                    file_stem: "03_C".to_string(),
                    markdown: "# C".to_string(),
                },
            ],
        };

        assert_eq!(
            render_links_txt(&collection),
            "https://zhuanlan.zhihu.com/p/1
https://www.zhihu.com/question/1/answer/2
"
        );
    }

    #[test]
    fn renders_pin_structured_content() {
        let item = render_item(
            1,
            CollectionItem {
                content: Some(Content {
                    id: Some(Value::from("123")),
                    kind: Some("pin".to_string()),
                    url: Some("https://www.zhihu.com/pin/123".to_string()),
                    title: None,
                    excerpt_title: Some("想法标题".to_string()),
                    content: Some(serde_json::json!([
                        {"type": "text", "content": "<p>想法<strong>正文</strong></p>"},
                        {"type": "link_card", "url": "https://www.zhihu.com/question/1"}
                    ])),
                    excerpt: None,
                    question: None,
                    author: None,
                    created_time: Some(10),
                    updated_time: Some(20),
                }),
            },
        );

        assert_eq!(item.title, "想法标题");
        assert!(item.markdown.contains("想法**正文**"));
        assert!(item.markdown.contains("https://www.zhihu.com/question/1"));
        assert!(!item.markdown.contains("接口未返回正文"));
    }
}
