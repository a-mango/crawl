use crate::extractor::{extract_links, normalize_url};
use crate::sitemap::{self, Sitemap};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use texting_robots::Robot;
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::sleep;
use url::{Host, Url};

/// Longest wait between retries, whether from backoff or a `Retry-After` header.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// Most sitemap files fetched per crawl (index files included).
const MAX_SITEMAP_FILES: usize = 100;

/// Settings for a crawl.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    /// Maximum depth; the start page is depth 1.
    pub max_depth: usize,
    /// Minimum delay after each request.
    pub delay: Duration,
    /// Only follow links whose host is this domain or a subdomain of it.
    /// A leading dot (e.g. `.com`) matches any host with that suffix.
    /// See [`site_domain`] to derive it from a URL.
    pub scope: Option<String>,
    /// Maximum number of requests in flight.
    pub concurrency: usize,
    /// User-Agent header, also used to match robots.txt rules.
    pub user_agent: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// How many times to retry a failed request (network errors, 429, 5xx).
    pub max_retries: u32,
    /// Initial backoff before the first retry; doubles on each attempt.
    pub retry_base_delay: Duration,
    /// Honour robots.txt rules and Crawl-delay.
    pub respect_robots: bool,
    /// Also start from every URL listed in the start site's sitemaps
    /// (found through robots.txt, or `/sitemap.xml`).
    pub use_sitemaps: bool,
    /// Stop after visiting this many pages.
    pub max_pages: Option<usize>,
    /// Suppress diagnostics on stderr.
    pub quiet: bool,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            delay: Duration::from_millis(1000),
            scope: None,
            concurrency: 10,
            user_agent: concat!("rust-crawler/", env!("CARGO_PKG_VERSION")).to_string(),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_base_delay: Duration::from_millis(500),
            respect_robots: true,
            use_sitemaps: false,
            max_pages: None,
            quiet: false,
        }
    }
}

/// A visited page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Normalized URL (no query string or fragment).
    pub url: String,
    /// Distance from the start URL; sitemap entries are also at depth 0.
    pub depth: usize,
    /// Final HTTP status, if a response was received.
    pub status: Option<u16>,
    /// `Content-Type` header of the response.
    pub content_type: Option<String>,
    /// Network or decoding error, if the page could not be fetched.
    pub error: Option<String>,
    /// Normalized http(s) links found on the page (in or out of scope).
    pub links: Vec<String>,
    /// When the response was received (or the request failed).
    pub fetched_at: SystemTime,
}

/// The outcome of a crawl.
#[derive(Debug, Clone, Default)]
pub struct CrawlReport {
    /// Visited pages, in the order they finished.
    pub pages: Vec<Page>,
}

impl CrawlReport {
    /// The set of visited URLs.
    pub fn urls(&self) -> HashSet<String> {
        self.pages.iter().map(|p| p.url.clone()).collect()
    }
}

/// Crawls from `start_url` and returns a report of every visited page.
/// Each URL is printed to stdout as it is visited.
pub async fn crawl(start_url: &str, config: CrawlConfig) -> Result<CrawlReport, reqwest::Error> {
    let crawler = Crawler::new(config)?;
    let max_depth = crawler.config.max_depth;

    let mut seeds = vec![start_url.to_string()];
    if crawler.config.use_sitemaps && max_depth > 0 {
        if let Ok(start) = Url::parse(start_url) {
            let found = crawler.sitemap_urls(&start).await;
            seeds.extend(
                found
                    .into_iter()
                    .filter(|u| Url::parse(u).is_ok_and(|u| crawler.should_follow(&u))),
            );
        }
    }

    let mut visits: FuturesUnordered<_> = seeds
        .into_iter()
        .map(|url| crawler.visit(url, max_depth))
        .collect();
    while visits.next().await.is_some() {}
    drop(visits);

    Ok(CrawlReport {
        pages: crawler.pages.into_inner().unwrap(),
    })
}

enum RobotsPolicy {
    AllowAll,
    DisallowAll,
    Rules(Box<Robot>),
}

/// A fetched response, reduced to what the crawler keeps.
struct Fetched {
    status: u16,
    content_type: Option<String>,
    /// URL after redirects; relative links resolve against it.
    final_url: String,
    /// Body, only for successful HTML responses.
    html: Option<String>,
}

struct Crawler {
    client: Client,
    config: CrawlConfig,
    visited: Mutex<HashSet<String>>,
    pages: Mutex<Vec<Page>>,
    semaphore: Semaphore,
    /// robots.txt policy per origin, fetched at most once.
    robots: Mutex<HashMap<String, Arc<OnceCell<RobotsPolicy>>>>,
    page_limit_logged: AtomicBool,
}

type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

impl Crawler {
    fn new(config: CrawlConfig) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.timeout)
            .build()?;
        Ok(Self {
            client,
            semaphore: Semaphore::new(config.concurrency.max(1)),
            config,
            visited: Mutex::new(HashSet::new()),
            pages: Mutex::new(Vec::new()),
            robots: Mutex::new(HashMap::new()),
            page_limit_logged: AtomicBool::new(false),
        })
    }

    fn log(&self, msg: impl std::fmt::Display) {
        if !self.config.quiet {
            eprintln!("{}", msg);
        }
    }

    /// Whether a link should be followed: http(s) and within scope.
    fn should_follow(&self, url: &Url) -> bool {
        if url.scheme() != "http" && url.scheme() != "https" {
            return false;
        }
        match (&self.config.scope, url.host_str()) {
            (None, _) => true,
            (Some(scope), Some(host)) => in_scope(host, scope),
            (Some(_), None) => false,
        }
    }

    /// Marks a URL as visited. Returns false if it already was, or if the
    /// page limit has been reached.
    fn claim(&self, normalized_url: &str) -> bool {
        let mut visited = self.visited.lock().unwrap();
        if let Some(max) = self.config.max_pages {
            if visited.len() >= max {
                if !self.page_limit_logged.swap(true, Ordering::Relaxed) {
                    self.log(format_args!("Reached the limit of {} pages", max));
                }
                return false;
            }
        }
        visited.insert(normalized_url.to_string())
    }

    fn visit(&self, url: String, depth: usize) -> BoxFuture<'_> {
        Box::pin(async move {
            if depth == 0 {
                return;
            }
            let Ok(parsed) = Url::parse(&url) else {
                self.log(format_args!("Skipping invalid URL: {}", url));
                return;
            };
            let normalized_url = normalize_url(&url);
            if self.visited.lock().unwrap().contains(&normalized_url) {
                return;
            }

            // Checked before taking a permit: fetching robots.txt needs one.
            let (allowed, crawl_delay) = self.robots_check(&parsed).await;
            if !allowed {
                self.log(format_args!("Skipping (robots.txt): {}", normalized_url));
                return;
            }

            if !self.claim(&normalized_url) {
                return;
            }
            println!("{}", normalized_url);

            // Hold a permit only while fetching (and during the politeness delay).
            // It must be released before recursing, otherwise parents waiting on
            // children exhaust the permits and the crawl deadlocks.
            let result = {
                let _permit = self.semaphore.acquire().await.unwrap();
                let result = self.fetch(&url).await;
                sleep(self.config.delay.max(crawl_delay.unwrap_or_default())).await;
                result
            };

            let mut page = Page {
                url: normalized_url.clone(),
                depth: self.config.max_depth - depth,
                status: None,
                content_type: None,
                error: None,
                links: Vec::new(),
                fetched_at: SystemTime::now(),
            };
            let mut children = Vec::new();
            match result {
                Ok(fetched) => {
                    page.status = Some(fetched.status);
                    page.content_type = fetched.content_type;
                    if let Some(html) = fetched.html {
                        let mut links = BTreeSet::new();
                        for link in extract_links(&html, &fetched.final_url) {
                            let Ok(parsed) = Url::parse(&link) else {
                                self.log(format_args!("Skipping invalid URL: {}", link));
                                continue;
                            };
                            // mailto:, tel:, javascript: etc. are expected; skip silently.
                            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                                continue;
                            }
                            links.insert(normalize_url(&link));
                            if self.should_follow(&parsed) {
                                children.push(link);
                            }
                        }
                        page.links = links.into_iter().collect();
                    }
                }
                Err(e) => {
                    self.log(format_args!("Error crawling {}: {}", normalized_url, e));
                    page.error = Some(e.to_string());
                }
            }
            self.pages.lock().unwrap().push(page);

            if depth == 1 {
                return;
            }
            let mut children: FuturesUnordered<_> = children
                .into_iter()
                .map(|link| self.visit(link, depth - 1))
                .collect();
            while children.next().await.is_some() {}
        })
    }

    /// Fetches a page (with retries). The body is only read for successful
    /// HTML responses; error statuses are returned as `Ok` with no body.
    async fn fetch(&self, url: &str) -> Result<Fetched, reqwest::Error> {
        let response = self.get_with_retry(url).await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let final_url = response.url().to_string();
        let is_html = content_type.as_deref().is_none_or(|ct| ct.contains("html"));
        let html = if status.is_success() && is_html {
            Some(response.text().await?)
        } else {
            if !status.is_success() {
                self.log(format_args!("HTTP {} for {}", status.as_u16(), url));
            }
            None
        };
        Ok(Fetched {
            status: status.as_u16(),
            content_type,
            final_url,
            html,
        })
    }

    /// GETs `url`, retrying network errors, 429 and 5xx responses with
    /// exponential backoff. A `Retry-After` header (in seconds) takes precedence.
    async fn get_with_retry(&self, url: &str) -> Result<Response, reqwest::Error> {
        let mut attempt = 0;
        loop {
            let can_retry = attempt < self.config.max_retries;
            let wait = match self.client.get(url).send().await {
                Ok(resp) if can_retry && is_retryable_status(resp.status()) => {
                    let wait = retry_after(&resp).unwrap_or_else(|| self.backoff(attempt));
                    self.log(format_args!(
                        "Retrying {} in {:.1?} (HTTP {})",
                        url,
                        wait,
                        resp.status().as_u16()
                    ));
                    wait
                }
                Err(e) if can_retry && (e.is_timeout() || e.is_connect()) => {
                    let wait = self.backoff(attempt);
                    self.log(format_args!("Retrying {} in {:.1?} ({})", url, wait, e));
                    wait
                }
                result => return result,
            };
            sleep(wait).await;
            attempt += 1;
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.config
            .retry_base_delay
            .saturating_mul(2u32.saturating_pow(attempt))
            .min(MAX_RETRY_WAIT)
    }

    /// The robots.txt policy for an origin, fetched on first use.
    async fn robots_policy(&self, origin: &str) -> Arc<OnceCell<RobotsPolicy>> {
        let cell = self
            .robots
            .lock()
            .unwrap()
            .entry(origin.to_string())
            .or_default()
            .clone();
        cell.get_or_init(|| self.fetch_robots(origin)).await;
        cell
    }

    /// Returns whether robots.txt allows `url`, and the site's Crawl-delay.
    async fn robots_check(&self, url: &Url) -> (bool, Option<Duration>) {
        if !self.config.respect_robots {
            return (true, None);
        }
        let cell = self
            .robots_policy(&url.origin().ascii_serialization())
            .await;
        match cell.get().expect("initialised by robots_policy") {
            RobotsPolicy::AllowAll => (true, None),
            RobotsPolicy::DisallowAll => (false, None),
            RobotsPolicy::Rules(robot) => (
                robot.allowed(url.as_str()),
                robot
                    .delay
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .map(|d| Duration::from_secs_f32(d).min(MAX_RETRY_WAIT)),
            ),
        }
    }

    /// Fetches and parses robots.txt for an origin, following RFC 9309:
    /// 4xx means no restrictions, 5xx or unreachable means disallow everything.
    async fn fetch_robots(&self, origin: &str) -> RobotsPolicy {
        let robots_url = format!("{}/robots.txt", origin);
        let response = {
            let _permit = self.semaphore.acquire().await.unwrap();
            self.get_with_retry(&robots_url).await
        };
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                self.log(format_args!(
                    "Could not fetch {} ({}); treating {} as disallowed",
                    robots_url, e, origin
                ));
                return RobotsPolicy::DisallowAll;
            }
        };
        let status = response.status();
        if status.is_client_error() {
            return RobotsPolicy::AllowAll;
        }
        if !status.is_success() {
            self.log(format_args!(
                "{} returned HTTP {}; treating {} as disallowed",
                robots_url,
                status.as_u16(),
                origin
            ));
            return RobotsPolicy::DisallowAll;
        }
        let agent = self.config.user_agent.split('/').next().unwrap_or_default();
        match response.bytes().await {
            Ok(body) => match Robot::new(agent, &body) {
                Ok(robot) => RobotsPolicy::Rules(Box::new(robot)),
                Err(_) => RobotsPolicy::AllowAll,
            },
            Err(_) => RobotsPolicy::DisallowAll,
        }
    }

    /// Collects page URLs from the start site's sitemaps. Sitemaps are taken
    /// from robots.txt `Sitemap:` lines, falling back to `/sitemap.xml`;
    /// sitemap index files are followed.
    async fn sitemap_urls(&self, start: &Url) -> Vec<String> {
        let origin = start.origin().ascii_serialization();
        let robots = self.robots_policy(&origin).await;
        let mut queue: VecDeque<String> = match robots.get().expect("initialised by robots_policy")
        {
            RobotsPolicy::Rules(robot) if !robot.sitemaps.is_empty() => {
                robot.sitemaps.iter().cloned().collect()
            }
            _ => VecDeque::from([format!("{}/sitemap.xml", origin)]),
        };

        let mut seen = HashSet::new();
        let mut urls = Vec::new();
        while let Some(sitemap_url) = queue.pop_front() {
            if !seen.insert(sitemap_url.clone()) {
                continue;
            }
            if seen.len() > MAX_SITEMAP_FILES {
                self.log(format_args!(
                    "Stopping after {} sitemap files",
                    MAX_SITEMAP_FILES
                ));
                break;
            }
            match self.fetch_sitemap(&sitemap_url).await {
                Ok(Sitemap::Urls(found)) => urls.extend(found),
                Ok(Sitemap::Index(children)) => queue.extend(children),
                Err(e) => self.log(format_args!("Skipping sitemap {}: {}", sitemap_url, e)),
            }
        }
        self.log(format_args!(
            "Found {} URLs in {} sitemap file(s)",
            urls.len(),
            seen.len().min(MAX_SITEMAP_FILES)
        ));
        urls
    }

    async fn fetch_sitemap(&self, url: &str) -> Result<Sitemap, String> {
        let _permit = self.semaphore.acquire().await.unwrap();
        let result = async {
            let response = self
                .get_with_retry(url)
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?;
            let body = response.bytes().await.map_err(|e| e.to_string())?;
            sitemap::parse(&body)
        }
        .await;
        sleep(self.config.delay).await;
        result
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_WAIT))
}

/// Returns the registrable domain of a URL's host, using the Public Suffix
/// List: `www.example.co.uk` gives `example.co.uk`, `user.github.io` stays
/// as is. IP addresses and hosts without a known suffix (e.g. `localhost`)
/// are returned unchanged. Use it as `CrawlConfig::scope` to crawl a site
/// together with all of its subdomains.
pub fn site_domain(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(host) => {
            let host = host.trim_end_matches('.');
            Some(psl::domain_str(host).unwrap_or(host).to_string())
        }
        _ => url.host_str().map(str::to_string),
    }
}

fn in_scope(host: &str, scope: &str) -> bool {
    if scope.starts_with('.') {
        host.ends_with(scope)
    } else {
        host == scope || host.ends_with(&format!(".{}", scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matches_domain_and_subdomains() {
        assert!(in_scope("example.com", "example.com"));
        assert!(in_scope("www.example.com", "example.com"));
        assert!(!in_scope("notexample.com", "example.com"));
        assert!(in_scope("foo.com", ".com"));
        assert!(!in_scope("foo.org", ".com"));
    }

    #[test]
    fn site_domain_strips_subdomains() {
        let domain = |u: &str| site_domain(&Url::parse(u).unwrap());
        assert_eq!(domain("https://example.com/"), Some("example.com".into()));
        assert_eq!(
            domain("https://www.example.com/a"),
            Some("example.com".into())
        );
        assert_eq!(
            domain("https://a.b.example.co.uk/"),
            Some("example.co.uk".into())
        );
        assert_eq!(
            domain("https://user.github.io/"),
            Some("user.github.io".into())
        );
        assert_eq!(domain("http://localhost:8080/"), Some("localhost".into()));
        assert_eq!(domain("http://127.0.0.1:8080/"), Some("127.0.0.1".into()));
        assert_eq!(domain("http://[::1]/"), Some("[::1]".into()));
        assert_eq!(domain("mailto:a@b.c"), None);
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let crawler = Crawler::new(CrawlConfig::default()).unwrap();
        assert_eq!(crawler.backoff(0), Duration::from_millis(500));
        assert_eq!(crawler.backoff(1), Duration::from_millis(1000));
        assert_eq!(crawler.backoff(2), Duration::from_millis(2000));
        assert_eq!(crawler.backoff(30), MAX_RETRY_WAIT);
    }

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::OK));
    }
}
