# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0-alpha] - 2025-10-09

### Added
- Initial alpha release
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
