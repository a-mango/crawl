mod crawler;
mod extractor;

use crate::crawler::crawl;
use atty::Stream;
use clap::Parser;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Semaphore;
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The URL to start crawling from
    #[arg(short, long, env = "CRAWL_START_URL")]
    start_url: String,

    /// The maximum depth to crawl
    #[arg(short, long, env = "CRAWL_DEPTH", default_value_t = 2)]
    depth: usize,

    /// The domain to scope crawling to
    #[arg(long, env = "CRAWL_SCOPE")]
    scope: Option<String>,

    /// Delay between requests in milliseconds
    #[arg(long, env = "CRAWL_DELAY_MS", default_value_t = 1000)]
    delay: u64,

    /// Custom User-Agent
    #[arg(long, env = "CRAWL_USER_AGENT", default_value = "rust-crawler/0.1.0")]
    user_agent: String,

    /// Maximum number of concurrent requests
    #[arg(long, env = "CRAWL_CONCURRENCY", default_value_t = 10)]
    concurrency: usize,

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
    let start_url = Url::parse(&cli.start_url).expect("Failed to parse start URL");
    let max_depth = cli.depth;
    let delay = Duration::from_millis(cli.delay);
    let user_agent = cli.user_agent;
    let visited = Arc::new(Mutex::new(HashSet::new()));
    let scope_domain = cli.scope.clone();
    let semaphore = Arc::new(Semaphore::new(cli.concurrency));
    let client = reqwest::Client::builder()
        .user_agent(&user_agent)
        .build()
        .unwrap();

    // Check if stdout is a TTY or being piped, or if quiet mode is explicitly requested
    let is_tty = atty::is(Stream::Stdout);
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
        if let Some(scope) = &cli.scope {
            println!("  Scope:        {}", scope);
        }
        println!("========================================\n");
    }

    let start_time = Instant::now();

    crawl(
        client,
        start_url.to_string(),
        max_depth,
        visited.clone(),
        delay,
        scope_domain,
        user_agent,
        semaphore,
        quiet_mode,
    )
    .await;

    let elapsed = start_time.elapsed();
    let visited_urls = visited.lock().unwrap();
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
