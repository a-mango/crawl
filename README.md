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
- 💾 **File output** in JSON or plain text format

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
- `--scope <DOMAIN>` - Domain to scope crawling to (e.g., `example.com`, `.com`)
- `--delay <MS>` - Delay between requests in milliseconds (default: 1000)
- `--user-agent <STRING>` - Custom User-Agent string (default: "rust-crawler/0.1.0")
- `--concurrency <NUM>` - Maximum number of concurrent requests (default: 10)
- `-o, --output <FILE>` - Output file path (supports .json or .txt extensions)
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
  User-Agent:   rust-crawler/0.1.0
  Concurrency:  10
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

When output is piped to another program or when using the `-q/--quiet` flag, only URLs are printed (one per line):

```
https://example.com
https://example.com/about
https://example.com/contact
```

This makes it perfect for Unix-style command chaining and processing with standard tools like `grep`, `sort`, `wc`, etc.

When using `-o/--output`, results are saved in the specified format:

- **JSON**: Structured format with total count and sorted URL list
- **Text**: One URL per line, sorted alphabetically

Non-HTTP schemes (mailto:, ftp:, etc.) are skipped and logged in normal mode.

## Architecture

The crawler is built with:

- **reqwest** - Async HTTP client
- **tokio** - Async runtime
- **scraper** - HTML parsing and link extraction
- **clap** - Command-line argument parsing

The codebase is modular:

- `crawler.rs` - Core crawling logic with concurrency control
- `extractor.rs` - HTML parsing and link extraction
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

- No robots.txt support
- No sitemap.xml support
- No retry logic for failed requests
- No resume/checkpoint functionality

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This software is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Roadmap

- [ ] Add retry logic with exponential backoff
- [ ] Respect robots.txt
- [ ] Support for sitemap.xml
- [ ] Resume crawling from checkpoint
- [ ] Better progress indicators
- [ ] Handle rate limiting (429 responses)
- [ ] Add CSV output format
- [ ] Add option to download crawled content
- [ ] Store crawled links as a graph structure
