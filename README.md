# Crawl

```txt
=======================================
                                    888
                                    888
                                    888
 .d8888b888d888 8888b. 888  888  888888
d88P"   888P"      "88b888  888  888888
888     888    .d888888888  888  888888
Y88b.   888    888  888Y88b 888 d88P888
 "Y8888P888    "Y888888 "Y8888888P" 888
=======================================
```

A fast, concurrent web crawler written in Rust 🦀

## Features

- 🚀 **Concurrent crawling** with configurable parallelism
- 🎯 **Domain scoping** to limit crawls to specific domains or TLDs
- 🔍 **Depth-limited crawling** to control how deep to traverse
- 🛡 **Custom User-Agent** support
- 📊 **Duplicate detection** to avoid revisiting pages
- 📈 **Crawl statistics** (total pages, time elapsed, average time per page)
- 🔇 **Quiet mode** for piping output (automatic detection or via flag)
- 💾 **File output** in JSON, plain text, or SQLite (pages, statuses and the link graph)
- 🗺 **Sitemap discovery** via robots.txt or `/sitemap.xml`, including index and gzipped sitemaps
- 🤖 **robots.txt** rules and `Crawl-delay` respected, with retries and backoff for flaky servers

## Installation

### From source

```bash
git clone git@github.com:a-mango/crawl.git
cd crawl
cargo build --release
```

The binary will be available at `target/release/crawl`. You can move it to a directory in your PATH for easier access:

```bash
mv target/release/crawl $HOME/.local/bin/
```

## Usage

Basic usage:

```bash
crawl -s https://example.com
```

### Options

- `-s, --start-url <URL>` - **Required**. The URL to start crawling from
- `-d, --depth <NUM>` - Maximum depth to crawl (default: 2)
- `--scope <DOMAIN>` - Only follow links on this domain and its subdomains (e.g. `example.com`), or on any host with this suffix if it starts with a dot (e.g. `.com`)
- `--subdomains` - Stay on the start URL's site, including all of its subdomains (cannot be combined with `--scope`)
- `--delay <MS>` - Delay between requests in milliseconds (default: 1000)
- `--user-agent <STRING>` - Custom User-Agent string (default: "rust-crawler/0.2.0")
- `--concurrency <NUM>` - Maximum number of concurrent requests (default: 10)
- `--timeout <SECS>` - Per-request timeout in seconds (default: 30)
- `--max-retries <NUM>` - Retries for network errors, timeouts, 429 and 5xx responses (default: 3)
- `--ignore-robots` - Ignore robots.txt rules and `Crawl-delay`
- `--sitemap` - Also start from every URL listed in the site's sitemaps
- `--max-pages <NUM>` - Stop after visiting this many pages
- `-o, --output <FILE>` - Output file: `.json`, `.txt`, or `.db`/`.sqlite`/`.sqlite3` for SQLite
- `-q, --quiet` - Quiet mode - only output URLs (useful for piping)

### Examples

Crawl a site with depth 3:

```bash
crawl -s https://example.com -d 3
```

Crawl only within a specific domain:

```bash
crawl -s https://example.com --scope example.com
```

Crawl a site and all of its subdomains (here `www.example.com`, `blog.example.com`, …).
The site is derived from the start URL using the [Public Suffix List](https://publicsuffix.org/),
so `www.example.co.uk` scopes to `example.co.uk`:

```bash
crawl -s https://www.example.com --subdomains
```

Without `--scope` or `--subdomains`, the crawler follows links to any host.

Save results to a JSON file:

```bash
crawl -s https://example.com -o results.json
```

Save results to a text file:

```bash
crawl -s https://example.com -o results.txt
```

Crawl all `.com` sites with custom concurrency:

```bash
crawl -s https://example.com --scope .com --concurrency 20
```

Crawl with a custom delay and user-agent:

```bash
crawl -s https://example.com --delay 2000 --user-agent "CustomBot/1.0"
```

**Piping to other programs** (quiet mode activates automatically):

```bash
# Filter URLs containing "api"
crawl -s https://example.com | grep "/api/"

# Get first 10 URLs
crawl -s https://example.com | head -n 10
```

### Environment variables

Every option can also be set through an environment variable. Command-line flags take precedence.

| Variable              | Flag              |
|-----------------------|-------------------|
| `CRAWL_START_URL`     | `--start-url`     |
| `CRAWL_DEPTH`         | `--depth`         |
| `CRAWL_SCOPE`         | `--scope`         |
| `CRAWL_SUBDOMAINS`    | `--subdomains`    |
| `CRAWL_DELAY_MS`      | `--delay`         |
| `CRAWL_USER_AGENT`    | `--user-agent`    |
| `CRAWL_CONCURRENCY`   | `--concurrency`   |
| `CRAWL_OUTPUT`        | `--output`        |
| `CRAWL_QUIET`         | `--quiet`         |
| `CRAWL_TIMEOUT_SECS`  | `--timeout`       |
| `CRAWL_MAX_RETRIES`   | `--max-retries`   |
| `CRAWL_IGNORE_ROBOTS` | `--ignore-robots` |
| `CRAWL_SITEMAP`       | `--sitemap`       |
| `CRAWL_MAX_PAGES`     | `--max-pages`     |

See [`.env.example`](.env.example) for a template.

## Docker

Build the image and run a crawl:

```bash
docker build -t crawl .
docker run --rm crawl -s https://example.com -d 2
```

With Docker Compose, results written to `/data` inside the container land in `./output`:

```bash
docker compose run --rm \
  -e CRAWL_START_URL=https://example.com \
  -e CRAWL_OUTPUT=/data/results.json \
  crawl
```

The Compose service runs as UID/GID 1000 so output files are owned by your user. Set `CRAWL_UID` and `CRAWL_GID` if yours differ.

## Output

### Normal Mode (Terminal)

The crawler prints each discovered URL to stdout as it crawls, with an ASCII banner and statistics at the end:

```
=======================================
                                    888
                                    888
                                    888
 .d8888b888d888 8888b. 888  888  888888
d88P"   888P"      "88b888  888  888888
888     888    .d888888888  888  888888
Y88b.   888    888  888Y88b 888 d88P888
 "Y8888P888    "Y888888 "Y8888888P" 888
=======================================

Starting crawl: https://example.com
Configuration:
  Max depth:    2
  Delay:        1000ms
  User-Agent:   rust-crawler/0.2.0
  Concurrency:  10
  Retries:      3
  robots.txt:   respected
========================================

https://example.com
https://example.com/about
https://example.com/contact

========================================
Crawl complete!
Statistics:
  Total pages:  3
  Time elapsed: 2.45s
  Avg per page: 816.67ms
```

### Quiet Mode (Piping)

When output is piped to another program or when using the `-q/--quiet` flag, only URLs are printed to stdout (one per line). Diagnostics still go to stderr unless `-q` is given:

```
https://example.com
https://example.com/about
https://example.com/contact
```

This makes it perfect for Unix-style command chaining and processing with standard tools like `grep`, `sort`, `wc`, etc.

When using `-o/--output`, results are saved in the specified format:

- **JSON**: Structured format with total count and sorted URL list
- **Text**: One URL per line, sorted alphabetically
- **SQLite** (`.db`, `.sqlite`, `.sqlite3`): every page with its status, content
  type, depth and errors, plus every link between pages. Each run is appended as
  a new crawl, so one database can hold a history of crawls.

### SQLite schema

| Table    | Columns                                                                                  |
|----------|------------------------------------------------------------------------------------------|
| `crawls` | `id`, `start_url`, `started_at`, `finished_at`, `page_count`, `crawler_version`          |
| `pages`  | `crawl_id`, `url`, `depth`, `status`, `content_type`, `error`, `fetched_at`              |
| `links`  | `crawl_id`, `source`, `target` (every http(s) link found, followed or not)               |

Timestamps are Unix milliseconds. Some example queries:

```sql
-- Broken pages in the latest crawl, and which pages link to them
SELECT p.url, p.status, l.source
FROM pages p JOIN links l ON l.crawl_id = p.crawl_id AND l.target = p.url
WHERE p.crawl_id = (SELECT MAX(id) FROM crawls) AND (p.status >= 400 OR p.error IS NOT NULL);

-- Most linked-to pages
SELECT target, COUNT(*) AS inbound FROM links WHERE crawl_id = 1
GROUP BY target ORDER BY inbound DESC LIMIT 10;

-- When each crawl ran
SELECT id, start_url, datetime(started_at / 1000, 'unixepoch') FROM crawls;
```

Non-HTTP schemes (mailto:, ftp:, etc.) are skipped. Diagnostics (fetch errors, retries, robots.txt decisions) go to stderr and are silenced by `-q`, so stdout only ever contains URLs. Links are only extracted from successful (2xx) HTML responses.

## Sitemaps

With `--sitemap`, the crawler reads the start site's sitemaps before crawling
and uses every listed URL as an additional starting point (at depth 0). Sitemaps
are taken from the `Sitemap:` lines in robots.txt, falling back to
`/sitemap.xml`. XML, plain-text and gzipped sitemaps are supported, and sitemap
index files are followed (up to 100 sitemap files). Sitemap entries still go
through `--scope`/`--subdomains` and robots.txt. Large sites can list tens of
thousands of URLs, so combine it with `--max-pages`:

```bash
crawl -s https://blog.example.com --sitemap --max-pages 500 -o blog.db
```

## Politeness

- **robots.txt** is fetched once per site and its rules are matched against the
  User-Agent's product token (the part before `/`). Following
  [RFC 9309](https://www.rfc-editor.org/rfc/rfc9309), a missing robots.txt (4xx)
  allows everything, while a server error or unreachable robots.txt blocks the
  whole site. `Crawl-delay` is honoured when it is longer than `--delay`.
  Use `--ignore-robots` only on sites you own or have permission to crawl.
- **Retries** use exponential backoff starting at 500ms (capped at 60s).
  A `Retry-After` header on 429/503 responses takes precedence.

## Library usage

```rust
use crawl::crawler::{crawl, CrawlConfig};

let report = crawl("https://example.com", CrawlConfig {
    max_depth: 3,
    scope: Some("example.com".into()),
    ..CrawlConfig::default()
}).await?;

for page in &report.pages {
    println!("{} {:?} ({} links)", page.url, page.status, page.links.len());
}

// Optionally persist it
crawl::storage::save_sqlite("crawl.db", "https://example.com", started, finished, &report)?;
```

## Architecture

The crawler is built with:

- **reqwest** - Async HTTP client
- **tokio** - Async runtime
- **scraper** - HTML parsing and link extraction
- **clap** - Command-line argument parsing
- **texting_robots** - robots.txt parsing
- **psl** - Public Suffix List lookups for `--subdomains`
- **quick-xml** / **flate2** - Sitemap parsing and decompression
- **rusqlite** - SQLite output (SQLite is bundled, no system library needed)

The codebase is modular:

- `crawler.rs` - Core crawling logic with concurrency control
- `extractor.rs` - HTML parsing and link extraction
- `sitemap.rs` - Sitemap parsing
- `storage.rs` - SQLite output
- `main.rs` - CLI interface

## Testing

Run the test suite:

```bash
cargo test
```

The project includes:

- Unit tests for link extraction and URL normalization
- Integration tests for crawling scenarios (circular links, depth limits, scope filtering, etc.)

## Limitations

- No resume/checkpoint functionality

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This software is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Roadmap

- [x] Add retry logic with exponential backoff
- [x] Respect robots.txt
- [x] Support for sitemap.xml
- [ ] Resume crawling from checkpoint
- [ ] Better progress indicators
- [x] Handle rate limiting (429 responses)
- [ ] Add CSV output format
- [ ] Add option to download crawled content
- [x] Store crawled links as a graph structure
- [x] Store crawl results in a database
