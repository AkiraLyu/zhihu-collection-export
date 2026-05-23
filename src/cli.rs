use std::path::PathBuf;

use clap::{Parser, ValueEnum};

const DEFAULT_LIMIT: u32 = 20;

#[derive(Parser, Debug)]
#[command(author, version, about = "Export a Zhihu collection to Markdown")]
pub(crate) struct Cli {
    /// Zhihu collection URL, for example https://www.zhihu.com/collection/997879559
    pub(crate) collection: Option<String>,

    /// Output root directory. A collection subdirectory is created inside it.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,

    /// Browser to read zhihu.com cookies from
    #[arg(long, value_enum, default_value_t = BrowserChoice::Auto)]
    pub(crate) browser: BrowserChoice,

    /// Raw Cookie header. When set, browser cookie extraction is skipped.
    #[arg(long)]
    pub(crate) cookie: Option<String>,

    /// Show cookie availability without printing cookie values.
    #[arg(long)]
    pub(crate) diagnose_cookies: bool,

    /// Continue even when no z_c0 login cookie is found.
    #[arg(long)]
    pub(crate) allow_anonymous: bool,

    /// Absolute browser cookie DB path for custom profiles.
    #[arg(long)]
    pub(crate) cookies_db: Option<PathBuf>,

    /// Chromium Local State path, mainly needed for custom profiles on Windows.
    #[arg(long)]
    pub(crate) key_file: Option<PathBuf>,

    /// Also write links.txt with one collected item URL per line.
    #[arg(long)]
    pub(crate) export_links: bool,

    /// API page size. Zhihu currently works well with 20.
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub(crate) limit: u32,

    /// Delay between page requests.
    #[arg(long, default_value_t = 800)]
    pub(crate) delay_ms: u64,

    /// Retry count for transient HTTP errors.
    #[arg(long, default_value_t = 3)]
    pub(crate) retries: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum BrowserChoice {
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
