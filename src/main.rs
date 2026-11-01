use clap::{CommandFactory, Parser};
use crawl::crawler::{crawl, site_domain, CrawlConfig};
use crawl::extractor;
use std::collections::HashSet;
use std::io::IsTerminal;
use std::time::{Duration, Instant};
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The URL to start crawling from
    #[arg(short, long, env = "CRAWL_START_URL")]
    start_url: Url,

    /// The maximum depth to crawl
    #[arg(short, long, env = "CRAWL_DEPTH", default_value_t = 2)]
    depth: usize,

    /// Only follow links on this domain and its subdomains (e.g. `example.com`),
    /// or on any host with this suffix if it starts with a dot (e.g. `.com`)
    #[arg(long, env = "CRAWL_SCOPE")]
    scope: Option<String>,

    /// Stay on the start URL's site, including all of its subdomains
    /// (e.g. starting at www.example.com also crawls blog.example.com)
    #[arg(long, env = "CRAWL_SUBDOMAINS")]
    subdomains: bool,

    /// Delay between requests in milliseconds
    #[arg(long, env = "CRAWL_DELAY_MS", default_value_t = 1000)]
    delay: u64,

    /// Custom User-Agent
    #[arg(long, env = "CRAWL_USER_AGENT", default_value = concat!("rust-crawler/", env!("CARGO_PKG_VERSION")))]
    user_agent: String,

    /// Maximum number of concurrent requests
    #[arg(long, env = "CRAWL_CONCURRENCY", default_value_t = 10)]
    concurrency: usize,

    /// Per-request timeout in seconds
    #[arg(long, env = "CRAWL_TIMEOUT_SECS", default_value_t = 30)]
    timeout: u64,

    /// Number of retries for failed requests (network errors, 429, 5xx)
    #[arg(long, env = "CRAWL_MAX_RETRIES", default_value_t = 3)]
    max_retries: u32,

    /// Ignore robots.txt rules and Crawl-delay
    #[arg(long, env = "CRAWL_IGNORE_ROBOTS")]
    ignore_robots: bool,

    /// Output file path (optional, outputs to stdout if not specified)
    #[arg(short, long, env = "CRAWL_OUTPUT")]
    output: Option<String>,

    /// Quiet mode - only output URLs (useful for piping)
    #[arg(short, long, env = "CRAWL_QUIET")]
    quiet: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let start_url = cli.start_url;
    let max_depth = cli.depth;
    let user_agent = cli.user_agent;
    // Checked by hand: clap's `conflicts_with` would also fire for
    // CRAWL_SUBDOMAINS=false, which is what .env.example sets.
    if cli.subdomains && cli.scope.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        Cli::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "--scope cannot be used with --subdomains",
            )
            .exit();
    }
    let scope = if cli.subdomains {
        site_domain(&start_url)
    } else {
        cli.scope
            .as_deref()
            .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
    };

    // Check if stdout is a TTY or being piped, or if quiet mode is explicitly requested
    let is_tty = std::io::stdout().is_terminal();
    let quiet_mode = cli.quiet || !is_tty;

    if !quiet_mode {
        // ASCII banner
        println!(
            "
=======================================
                                    888
                                    888
                                    888
 .d8888b888d888 8888b. 888  888  888888
d88P\"   888P\"      \"88b888  888  888888
888     888    .d888888888  888  888888
Y88b.   888    888  888Y88b 888 d88P888
 \"Y8888P888    \"Y888888 \"Y8888888P\" 888
=======================================
"
        );

        println!(
            "Starting crawl: {}",
            extractor::normalize_url(start_url.as_str())
        );
        println!("Configuration:");
        println!("  Max depth:    {}", max_depth);
        println!("  Delay:        {}ms", cli.delay);
        println!("  User-Agent:   {}", user_agent);
        println!("  Concurrency:  {}", cli.concurrency);
        if let Some(scope) = &scope {
            println!("  Scope:        {}", scope);
        }
        println!("  Retries:      {}", cli.max_retries);
        println!(
            "  robots.txt:   {}",
            if cli.ignore_robots {
                "ignored"
            } else {
                "respected"
            }
        );
        println!("========================================\n");
    }

    let start_time = Instant::now();

    let config = CrawlConfig {
        max_depth,
        delay: Duration::from_millis(cli.delay),
        scope,
        concurrency: cli.concurrency,
        user_agent: user_agent.clone(),
        timeout: Duration::from_secs(cli.timeout),
        max_retries: cli.max_retries,
        respect_robots: !cli.ignore_robots,
        quiet: quiet_mode,
        ..CrawlConfig::default()
    };

    let visited_urls = match crawl(start_url.as_str(), config).await {
        Ok(urls) => urls,
        Err(e) => {
            eprintln!("Error: failed to start crawl: {}", e);
            std::process::exit(1);
        }
    };

    let elapsed = start_time.elapsed();
    let total_pages = visited_urls.len();

    if !quiet_mode {
        println!("\n========================================");
        println!("Crawl complete!");
        println!("Statistics:");
        println!("  Total pages:  {}", total_pages);
        println!("  Time elapsed: {:.2?}", elapsed);
        if total_pages > 0 {
            println!("  Avg per page: {:.2?}", elapsed / total_pages as u32);
        }
    }

    // Save to file if output path specified
    if let Some(output_path) = cli.output {
        match save_results(&output_path, &visited_urls) {
            Ok(_) => {
                if !quiet_mode {
                    println!("\nResults saved to: {}", output_path);
                }
            }
            Err(e) => eprintln!("\nError: Failed to save results: {}", e),
        }
    }
}

fn save_results(path: &str, urls: &HashSet<String>) -> std::io::Result<()> {
    use std::fs::File;
    use std::io::Write;

    let mut file = File::create(path)?;

    // Determine format based on extension
    if path.ends_with(".json") {
        // JSON format
        let mut sorted_urls: Vec<_> = urls.iter().collect();
        sorted_urls.sort();
        writeln!(file, "{{")?;
        writeln!(file, "  \"total_urls\": {},", urls.len())?;
        writeln!(file, "  \"urls\": [")?;
        for (i, url) in sorted_urls.iter().enumerate() {
            if i < sorted_urls.len() - 1 {
                writeln!(file, "    \"{}\",", url)?;
            } else {
                writeln!(file, "    \"{}\"", url)?;
            }
        }
        writeln!(file, "  ]")?;
        writeln!(file, "}}")?;
    } else {
        // Plain text format (one URL per line)
        let mut sorted_urls: Vec<_> = urls.iter().collect();
        sorted_urls.sort();
        for url in sorted_urls {
            writeln!(file, "{}", url)?;
        }
    }

    Ok(())
}
