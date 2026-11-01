# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- robots.txt support (RFC 9309): rules are fetched once per site and matched
  against the User-Agent; `Crawl-delay` is honoured. Opt out with `--ignore-robots`
- Retries with exponential backoff for network errors, timeouts, 429 and 5xx
  responses, honouring `Retry-After` (`--max-retries`, default 3)
- Per-request timeout (`--timeout`, default 30s)
- Invalid start URLs are rejected with a clear error instead of a panic

### Changed
- **Breaking (library):** `crawler::crawl` now takes a start URL and a
  `CrawlConfig` and returns the visited URLs, instead of nine positional
  arguments and a shared `visited` set
- The `user_agent` setting is now applied by the library itself (it was
  previously ignored by `crawl` and only worked through the CLI's client)

## [0.2.0] - 2026-11-01

### Added
- Configuration through environment variables (`CRAWL_*`), with `.env.example`
- Docker image (multi-stage, non-root) and Docker Compose service

### Fixed
- Crawls could hang when the link depth exceeded `--concurrency`: a page held its
  concurrency slot while waiting on its child pages
- Quiet/piped output was polluted by "Skipping …" messages; diagnostics now go to
  stderr and are suppressed in quiet mode, including for nested pages
- Links were extracted from error pages (4xx/5xx); these are now only recorded as visited
- Non-HTML responses (images, PDFs, …) were downloaded in full; they are now skipped
- Docker build failed at the dependency caching step
- Docker Compose could not write results to `./output` (UID mismatch, SELinux labels)

### Changed
- Default User-Agent now tracks the crate version (`rust-crawler/0.2.0`)
- Replaced the unmaintained `atty` crate with `std::io::IsTerminal`
- Minimum supported Rust version is now 1.82

## [0.1.0] - 2025-10-09

### Added
- Initial release
- Concurrent web crawling with configurable parallelism
- Depth-limited crawling
- Domain scoping support (exact domain or TLD matching)
- Custom User-Agent support
- Configurable delay between requests
- URL normalization (removes query params and fragments)
- Automatic duplicate detection
- Non-HTTP scheme filtering (skips mailto:, ftp:, etc.)
- Comprehensive test suite (unit and integration tests)
- Modular architecture (crawler, extractor, main modules)
- File output support (JSON and plain text formats)
- Crawl statistics (total pages, time elapsed, average time per page)
- Clean formatted console output with ASCII banner
- Quiet mode for piping output to other programs (automatic detection or via `-q` flag)
- TTY detection for automatic Unix-style command chaining

### Known Issues
- No robots.txt support
- No retry logic for failed requests

[Unreleased]: https://github.com/a-mango/crawl/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/a-mango/crawl/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/a-mango/crawl/releases/tag/v0.1.0
