use kuchikiki::{NodeData, NodeRef, traits::TendrilSink};

use crate::zhihu::normalize_zhihu_url;

pub(crate) fn html_to_markdown(html: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_zhihu_html_to_markdown() {
        let html = r#"<p>你好 <strong>世界</strong></p><blockquote><p>引用</p></blockquote><p><img data-original="//pic.example/a.jpg"></p>"#;
        let markdown = html_to_markdown(html);
        assert!(markdown.contains("你好 **世界**"));
        assert!(markdown.contains("> 引用"));
        assert!(markdown.contains("![图片](https://pic.example/a.jpg)"));
    }
}
