use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, CONTENT_TYPE, REFERER},
};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use url::Url;

use crate::{
    cli::ImageMode,
    zhihu::{BROWSER_USER_AGENT, ZHIHU_HOST},
};

const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;

pub(crate) struct ImageExporter {
    mode: ImageMode,
    output_dir: PathBuf,
    client: Option<Client>,
    retries: u32,
    delay_ms: u64,
    // Cache failures too, so a broken image is only requested/warned about once.
    replacements: HashMap<String, Option<String>>,
}

impl ImageExporter {
    pub(crate) fn new(
        mode: ImageMode,
        output_dir: &Path,
        retries: u32,
        delay_ms: u64,
    ) -> Result<Self> {
        let client = if mode == ImageMode::Remote {
            None
        } else {
            // Never reuse the API client: its default Cookie header would be sent
            // to arbitrary image hosts, including hosts reached through redirects.
            Some(
                Client::builder()
                    .user_agent(BROWSER_USER_AGENT)
                    // Keep the page referer when an image redirects to a CDN.
                    .referer(false)
                    .timeout(Duration::from_secs(30))
                    .build()
                    .context("创建图片 HTTP client 失败")?,
            )
        };
        Ok(Self {
            mode,
            output_dir: output_dir.to_path_buf(),
            client,
            retries,
            delay_ms,
            replacements: HashMap::new(),
        })
    }

    pub(crate) async fn prepare(&mut self, source: &str) -> Result<()> {
        let Some(client) = &self.client else {
            return Ok(());
        };
        if self.replacements.contains_key(source) {
            return Ok(());
        }
        let Ok(url) = Url::parse(source) else {
            return Ok(());
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Ok(());
        }

        let image = match download_image(client, url, self.retries, self.delay_ms).await {
            Ok(image) => image,
            Err(error) => {
                eprintln!("图片下载失败，保留外链: {source} ({error:#})");
                self.replacements.insert(source.to_string(), None);
                return Ok(());
            }
        };

        let replacement = match self.mode {
            ImageMode::Remote => unreachable!("remote mode has no image client"),
            ImageMode::Local => {
                let digest = Sha256::digest(&image.bytes);
                let relative_path = format!("images/{digest:x}.{}", image.extension);
                let path = self.output_dir.join(&relative_path);
                fs::create_dir_all(self.output_dir.join("images")).context("创建图片子目录失败")?;
                fs::write(&path, &image.bytes)
                    .with_context(|| format!("保存图片失败: {}", path.display()))?;
                relative_path
            }
            ImageMode::Base64 => format!(
                "data:{};base64,{}",
                image.media_type,
                STANDARD.encode(&image.bytes)
            ),
        };
        self.replacements
            .insert(source.to_string(), Some(replacement));
        Ok(())
    }

    pub(crate) fn resolve<'a>(&'a self, source: &'a str) -> &'a str {
        self.replacements
            .get(source)
            .and_then(|replacement| replacement.as_deref())
            .unwrap_or(source)
    }
}

struct DownloadedImage {
    bytes: Vec<u8>,
    media_type: String,
    extension: &'static str,
}

async fn download_image(
    client: &Client,
    url: Url,
    retries: u32,
    delay_ms: u64,
) -> Result<DownloadedImage> {
    for attempt in 0..=retries {
        let response = client
            .get(url.clone())
            .header(REFERER, ZHIHU_HOST)
            .header(ACCEPT, "image/*")
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return read_image(response).await;
                }
                if attempt == retries
                    || !(status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                {
                    bail!("HTTP {}", status.as_u16());
                }
            }
            Err(error) if attempt == retries => return Err(error).context("请求图片失败"),
            Err(_) => {}
        }
        let backoff = delay_ms.saturating_mul(u64::from(attempt) + 1).max(300);
        sleep(Duration::from_millis(backoff)).await;
    }
    unreachable!("the final attempt always returns")
}

async fn read_image(mut response: reqwest::Response) -> Result<DownloadedImage> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        bail!("图片超过 50 MiB 上限");
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("读取图片失败")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            bail!("图片超过 50 MiB 上限");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        bail!("图片响应为空");
    }
    let media_type = if content_type.is_empty() || content_type == "application/octet-stream" {
        sniff_image_type(&bytes)
            .context("响应未提供图片 Content-Type，且无法识别图片格式")?
            .to_string()
    } else if content_type.starts_with("image/")
        && content_type[6..]
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '+' | '-'))
        && content_type.len() > 6
    {
        content_type
    } else {
        bail!("响应不是图片: {content_type}");
    };
    let extension = match media_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/bmp" | "image/x-ms-bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        _ => "img",
    };
    Ok(DownloadedImage {
        bytes,
        media_type,
        extension,
    })
}

fn sniff_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("image/tiff")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/vnd.microsoft.icon")
    } else if bytes.get(4..8) == Some(b"ftyp")
        && matches!(bytes.get(8..12), Some(b"avif" | b"avis"))
    {
        Some("image/avif")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{ImageServer, SVG, png, response};

    #[tokio::test]
    async fn saves_images_with_relative_paths_and_reuses_duplicates() {
        let server = ImageServer::start(vec![
            response(200, &[("Content-Type", "image/png")], &png()),
            response(200, &[("Content-Type", "image/svg+xml")], SVG),
            response(200, &[("Content-Type", "image/png")], &png()),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Local, directory.path(), 0, 0).unwrap();
        let first = server.url("/photo.png?version=1");
        let second = server.url("/photo.png?version=2");
        let duplicate = server.url("/another/photo.png");
        for source in [&first, &first, &second, &duplicate] {
            images.prepare(source).await.unwrap();
        }
        let first_path = images.resolve(&first);
        let second_path = images.resolve(&second);
        assert!(first_path.starts_with("images/"));
        assert!(first_path.ends_with(".png"));
        assert!(second_path.ends_with(".svg"));
        assert_ne!(first_path, second_path);
        assert_eq!(images.resolve(&duplicate), first_path);
        assert_eq!(fs::read(directory.path().join(first_path)).unwrap(), png());
        assert_eq!(fs::read(directory.path().join(second_path)).unwrap(), SVG);
        assert_eq!(
            fs::read_dir(directory.path().join("images"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test]
    async fn embeds_exact_bytes_with_mime_type_without_creating_files() {
        let server = ImageServer::start(vec![
            response(
                200,
                &[("Content-Type", "IMAGE/SVG+XML; charset=utf-8")],
                SVG,
            ),
            response(200, &[("Content-Type", "application/octet-stream")], &png()),
            response(200, &[], &png()),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Base64, directory.path(), 0, 0).unwrap();
        for (path, mime, expected) in [
            ("/vector", "image/svg+xml", SVG.to_vec()),
            ("/binary", "image/png", png()),
            ("/no-header", "image/png", png()),
        ] {
            let source = server.url(path);
            images.prepare(&source).await.unwrap();
            images.prepare(&source).await.unwrap();
            let encoded = images
                .resolve(&source)
                .strip_prefix(&format!("data:{mime};base64,"))
                .unwrap();
            assert_eq!(STANDARD.decode(encoded).unwrap(), expected);
        }
        for source in [
            "data:image/png;base64,AAAA",
            "images/existing.png",
            "file:///tmp/image.png",
        ] {
            images.prepare(source).await.unwrap();
            assert_eq!(images.resolve(source), source);
        }
        assert_eq!(server.requests().len(), 3);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn keeps_failed_urls_and_does_not_repeat_failed_requests() {
        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_IMAGE_BYTES + 1
        ).into_bytes();
        let server = ImageServer::start(vec![
            response(404, &[], b"missing"),
            response(200, &[("Content-Type", "text/html")], b"<html>Login</html>"),
            response(200, &[("Content-Type", "image/png")], b""),
            oversized,
            response(200, &[], b"not an image"),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Local, directory.path(), 3, 0).unwrap();
        for path in ["/missing", "/login", "/empty", "/too-large", "/unknown"] {
            let source = server.url(path);
            images.prepare(&source).await.unwrap();
            images.prepare(&source).await.unwrap();
            assert_eq!(images.resolve(&source), source);
        }
        assert_eq!(server.requests().len(), 5);
        assert!(!directory.path().join("images").exists());
    }

    #[tokio::test]
    async fn retries_transient_http_errors() {
        let server = ImageServer::start(vec![
            response(503, &[], b"unavailable"),
            response(429, &[], b"slow down"),
            response(200, &[("Content-Type", "image/png")], &png()),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Base64, directory.path(), 2, 0).unwrap();
        let source = server.url("/retry");
        images.prepare(&source).await.unwrap();
        assert!(
            images
                .resolve(&source)
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test]
    async fn image_requests_and_redirects_have_no_cookies() {
        let destination = ImageServer::start(vec![response(
            200,
            &[("Content-Type", "image/png")],
            &png(),
        )]);
        let source = ImageServer::start(vec![response(
            302,
            &[
                ("Location", &destination.url("/image")),
                ("Set-Cookie", "secret=value"),
            ],
            b"",
        )]);
        let directory = tempfile::tempdir().unwrap();
        let mut images = ImageExporter::new(ImageMode::Base64, directory.path(), 0, 0).unwrap();
        let url = source.url("/redirect");
        images.prepare(&url).await.unwrap();
        assert!(images.resolve(&url).starts_with("data:image/png;base64,"));
        for server in [&source, &destination] {
            let requests = server.requests();
            assert_eq!(requests.len(), 1);
            let request = requests[0].to_ascii_lowercase();
            assert!(!request.contains("\r\ncookie:"));
            assert!(!request.contains("\r\nauthorization:"));
            assert!(request.contains("\r\nreferer: https://www.zhihu.com"));
        }
    }

    #[tokio::test]
    async fn reports_local_write_errors() {
        let server = ImageServer::start(vec![response(
            200,
            &[("Content-Type", "image/png")],
            &png(),
        )]);
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("images"), "not a directory").unwrap();
        let mut images = ImageExporter::new(ImageMode::Local, directory.path(), 0, 0).unwrap();
        let source = server.url("/image");
        let error = images.prepare(&source).await.unwrap_err();
        assert!(error.to_string().contains("创建图片子目录失败"));
        assert_eq!(images.resolve(&source), source);
    }
}
