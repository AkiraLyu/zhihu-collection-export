use anyhow::Result;
use kuchikiki::{NodeData, NodeRef, traits::TendrilSink};

use crate::{images::ImageExporter, zhihu::normalize_zhihu_url};

pub(crate) async fn html_to_markdown(html: &str, images: &mut ImageExporter) -> Result<String> {
    if html.trim().is_empty() {
        return Ok(String::new());
    }

    let document = kuchikiki::parse_html().one(format!("<!doctype html><body>{html}</body>"));
    let body = document
        .select_first("body")
        .ok()
        .map(|node| node.as_node().clone())
        .unwrap_or(document);
    for node in body.select("img").expect("valid selector") {
        if node.as_node().ancestors().any(|ancestor| {
            matches!(
                element_tag(&ancestor).as_deref(),
                Some("script" | "style" | "noscript" | "pre" | "code")
            )
        }) {
            continue;
        }
        if let Some(source) = image_source(&node) {
            images.prepare(&source).await?;
        }
    }
    let markdown = convert_children(&body, images);
    Ok(cleanup_markdown(&markdown))
}

fn convert_children(node: &NodeRef, images: &ImageExporter) -> String {
    node.children()
        .map(|child| convert_node(&child, images))
        .collect::<Vec<_>>()
        .join("")
}

fn convert_node(node: &NodeRef, images: &ImageExporter) -> String {
    match node.data() {
        NodeData::Text(text) => text.borrow().to_string(),
        NodeData::Element(element) => {
            let tag = element.name.local.to_string();
            let content = convert_children(node, images);

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
                "img" => image(element, images),
                "ul" => unordered_list(node, images),
                "ol" => ordered_list(node, images),
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

fn image_source(element: &kuchikiki::ElementData) -> Option<String> {
    let attrs = element.attributes.borrow();
    ["data-original", "data-actualsrc", "src"]
        .into_iter()
        .filter_map(|name| attrs.get(name))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(normalize_zhihu_url)
}

fn image(element: &kuchikiki::ElementData, images: &ImageExporter) -> String {
    let Some(src) = image_source(element) else {
        return String::new();
    };
    let attrs = element.attributes.borrow();
    let alt = attrs.get("alt").unwrap_or("图片").trim();
    format!("{}\n\n", markdown_image(alt, images.resolve(&src)))
}

pub(crate) fn markdown_image(alt: &str, source: &str) -> String {
    let alt = cleanup_inline(alt)
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let mut destination = String::with_capacity(source.len());
    for ch in source.chars() {
        match ch {
            '(' => destination.push_str("%28"),
            ')' => destination.push_str("%29"),
            ' ' => destination.push_str("%20"),
            '\n' => destination.push_str("%0A"),
            '\r' => destination.push_str("%0D"),
            '\t' => destination.push_str("%09"),
            '<' => destination.push_str("%3C"),
            '>' => destination.push_str("%3E"),
            '\\' => destination.push_str("%5C"),
            _ => destination.push(ch),
        }
    }
    format!(
        "![{}]({destination})",
        if alt.is_empty() { "图片" } else { &alt }
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

fn unordered_list(node: &NodeRef, images: &ImageExporter) -> String {
    let mut output = String::new();
    for child in node.children() {
        if element_tag(&child).as_deref() == Some("li") {
            let content = cleanup_inline(&convert_children(&child, images));
            if !content.is_empty() {
                output.push_str(&format!("* {content}\n"));
            }
        } else {
            output.push_str(&convert_node(&child, images));
        }
    }
    if output.is_empty() {
        output
    } else {
        output.push('\n');
        output
    }
}

fn ordered_list(node: &NodeRef, images: &ImageExporter) -> String {
    let mut output = String::new();
    let mut index = 1;
    for child in node.children() {
        if element_tag(&child).as_deref() == Some("li") {
            let content = cleanup_inline(&convert_children(&child, images));
            if !content.is_empty() {
                output.push_str(&format!("{index}. {content}\n"));
                index += 1;
            }
        } else {
            output.push_str(&convert_node(&child, images));
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{
        cli::ImageMode,
        test_support::{ImageServer, png, response},
    };

    #[tokio::test]
    async fn converts_basic_zhihu_html_to_markdown() {
        let html = r#"<p>你好 <strong>世界</strong></p><blockquote><p>引用</p></blockquote><p><img data-original="//pic.example/a.jpg"></p>"#;
        let mut images = ImageExporter::new(ImageMode::Remote, Path::new("."), 0, 0).unwrap();
        let markdown = html_to_markdown(html, &mut images).await.unwrap();
        assert!(markdown.contains("你好 **世界**"));
        assert!(markdown.contains("> 引用"));
        assert!(markdown.contains("![图片](https://pic.example/a.jpg)"));
    }

    #[tokio::test]
    async fn handles_lazy_sources_and_escapes_image_syntax() {
        let mut images = ImageExporter::new(ImageMode::Remote, Path::new("."), 0, 0).unwrap();
        let markdown = html_to_markdown(
            r#"<img data-original=" " data-actualsrc="//pic.example/a (b).png" src="wrong.png" alt="A [B]"><img data-actualsrc="" src="/c.png"><img src="data:image/png;base64,AAAA"><img src=" ">"#,
            &mut images,
        ).await.unwrap();
        assert!(markdown.contains(r"![A \[B\]](https://pic.example/a%20%28b%29.png)"));
        assert!(markdown.contains("![图片](https://www.zhihu.com/c.png)"));
        assert!(markdown.contains("![图片](data:image/png;base64,AAAA)"));
        assert_eq!(markdown.matches("![").count(), 3);
        assert!(!markdown.contains("wrong.png"));
    }

    #[tokio::test]
    async fn keeps_failed_image_links_while_exporting_other_images() {
        let server = ImageServer::start(vec![
            response(404, &[], b"missing"),
            response(200, &[("Content-Type", "image/png")], &png()),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Base64, directory.path(), 0, 0).unwrap();
        let missing = server.url("/missing");
        let found = server.url("/found");
        let ignored = server.url("/ignored");
        let html = format!(
            "<img src=\"{missing}\" alt=\"丢失\"><img src=\"{found}\" alt=\"成功\"><code><img src=\"{ignored}\"></code><pre><img src=\"{ignored}\"></pre><a href=\"{ignored}\">链接</a><img src=\"{missing}\">"
        );
        let markdown = html_to_markdown(&html, &mut images).await.unwrap();
        assert!(markdown.contains(&format!("![丢失]({missing})")));
        assert!(markdown.contains("![成功](data:image/png;base64,"));
        assert_eq!(server.requests().len(), 2);
    }
}
