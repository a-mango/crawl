use crate::extractor::{extract_links, normalize_url};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use texting_robots::Robot;
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::sleep;
use url::Url;

/// Longest wait between retries, whether from backoff or a `Retry-After` header.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// Settings for a crawl.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    /// Maximum depth; the start page is depth 1.
    pub max_depth: usize,
    /// Minimum delay after each request.
    pub delay: Duration,
    /// Only follow links whose host is this domain or a subdomain of it.
    /// A leading dot (e.g. `.com`) matches any host with that suffix.
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
            quiet: false,
        }
    }
}

/// Crawls from `start_url` and returns the set of visited (normalized) URLs.
/// Each URL is printed to stdout as it is visited.
pub async fn crawl(
    start_url: &str,
    config: CrawlConfig,
) -> Result<HashSet<String>, reqwest::Error> {
    let client = Client::builder()
        .user_agent(&config.user_agent)
        .timeout(config.timeout)
        .build()?;
    let crawler = Crawler {
        client,
        semaphore: Semaphore::new(config.concurrency.max(1)),
        config,
        visited: Mutex::new(HashSet::new()),
        robots: Mutex::new(HashMap::new()),
    };
    crawler
        .visit(start_url.to_string(), crawler.config.max_depth)
        .await;
    Ok(crawler.visited.into_inner().unwrap())
}

enum RobotsPolicy {
    AllowAll,
    DisallowAll,
    Rules(Box<Robot>),
}

struct Crawler {
    client: Client,
    config: CrawlConfig,
    visited: Mutex<HashSet<String>>,
    semaphore: Semaphore,
    /// robots.txt policy per origin, fetched at most once.
    robots: Mutex<HashMap<String, Arc<OnceCell<RobotsPolicy>>>>,
}

type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

impl Crawler {
    fn log(&self, msg: impl std::fmt::Display) {
        if !self.config.quiet {
            eprintln!("{}", msg);
        }
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

            if !self.visited.lock().unwrap().insert(normalized_url.clone()) {
                return;
            }
            println!("{}", normalized_url);

            // Hold a permit only while fetching (and during the politeness delay).
            // It must be released before recursing, otherwise parents waiting on
            // children exhaust the permits and the crawl deadlocks.
            let result = {
                let _permit = self.semaphore.acquire().await.unwrap();
                let result = self.fetch_html(&url).await;
                sleep(self.config.delay.max(crawl_delay.unwrap_or_default())).await;
                result
            };

            let body = match result {
                Ok(Some(body)) => body,
                Ok(None) => return,
                Err(e) => {
                    self.log(format_args!("Error crawling {}: {}", normalized_url, e));
                    return;
                }
            };
            if depth == 1 {
                return;
            }

            let mut children = FuturesUnordered::new();
            for link in extract_links(&body, &url) {
                let Ok(parsed) = Url::parse(&link) else {
                    self.log(format_args!("Skipping invalid URL: {}", link));
                    continue;
                };
                if parsed.scheme() != "http" && parsed.scheme() != "https" {
                    self.log(format_args!("Skipping non-HTTP URL: {}", link));
                    continue;
                }
                if let Some(scope) = &self.config.scope {
                    match parsed.host_str() {
                        Some(host) if in_scope(host, scope) => {}
                        _ => continue,
                    }
                }
                children.push(self.visit(link, depth - 1));
            }
            while children.next().await.is_some() {}
        })
    }

    /// Fetches a page and returns its body, or `None` if it is not HTML.
    /// Non-2xx responses (after retries) are returned as errors.
    async fn fetch_html(&self, url: &str) -> Result<Option<String>, reqwest::Error> {
        let response = self.get_with_retry(url).await?.error_for_status()?;
        let is_html = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|ct| ct.contains("html"));
        if !is_html {
            return Ok(None);
        }
        Ok(Some(response.text().await?))
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

    /// Returns whether robots.txt allows `url`, and the site's Crawl-delay.
    async fn robots_check(&self, url: &Url) -> (bool, Option<Duration>) {
        if !self.config.respect_robots {
            return (true, None);
        }
        let origin = url.origin().ascii_serialization();
        let cell = self
            .robots
            .lock()
            .unwrap()
            .entry(origin.clone())
            .or_default()
            .clone();
        let policy = cell.get_or_init(|| self.fetch_robots(&origin)).await;
        match policy {
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
                    "Could not fetch {} ({}); not crawling {}",
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
                "{} returned HTTP {}; not crawling {}",
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
    fn backoff_doubles_and_is_capped() {
        let crawler = Crawler {
            client: Client::new(),
            config: CrawlConfig::default(),
            visited: Mutex::new(HashSet::new()),
            semaphore: Semaphore::new(1),
            robots: Mutex::new(HashMap::new()),
        };
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
