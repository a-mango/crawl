use crate::extractor::{extract_links, normalize_url};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use url::Url;

/// Fetches a page and returns its body, or `None` if it is not HTML.
/// Non-2xx responses are returned as errors.
async fn fetch_html(client: &Client, url: &str) -> Result<Option<String>, reqwest::Error> {
    let response = client.get(url).send().await?.error_for_status()?;
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

fn in_scope(host: &str, scope: &str) -> bool {
    if scope.starts_with('.') {
        host.ends_with(scope)
    } else {
        host == scope || host.ends_with(&format!(".{}", scope))
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn crawl(
    client: Client,
    url: String,
    depth: usize,
    visited: Arc<Mutex<HashSet<String>>>,
    delay: Duration,
    scope_domain: Option<String>,
    user_agent: String,
    semaphore: Arc<Semaphore>,
    quiet: bool,
) {
    if depth == 0 {
        return;
    }

    let normalized_url = normalize_url(&url);
    if !visited.lock().unwrap().insert(normalized_url.clone()) {
        return;
    }

    println!("{}", normalized_url);

    // Hold a permit only while fetching (and during the politeness delay).
    // It must be released before recursing, otherwise parents waiting on
    // children exhaust the permits and the crawl deadlocks.
    let result = {
        let _permit = semaphore.acquire().await.unwrap();
        let result = fetch_html(&client, &url).await;
        sleep(delay).await;
        result
    };

    let body = match result {
        Ok(Some(body)) => body,
        Ok(None) => return,
        Err(e) => {
            if !quiet {
                eprintln!("Error crawling {}: {}", normalized_url, e);
            }
            return;
        }
    };

    if depth == 1 {
        return;
    }

    let mut futures = FuturesUnordered::new();
    for link in extract_links(&body, &url) {
        let parsed = match Url::parse(&link) {
            Ok(parsed) => parsed,
            Err(_) => {
                if !quiet {
                    eprintln!("Skipping invalid URL: {}", link);
                }
                continue;
            }
        };
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            if !quiet {
                eprintln!("Skipping non-HTTP URL: {}", link);
            }
            continue;
        }
        if let Some(scope) = &scope_domain {
            match parsed.host_str() {
                Some(host) if in_scope(host, scope) => {}
                _ => continue,
            }
        }
        futures.push(crawl(
            client.clone(),
            link,
            depth - 1,
            visited.clone(),
            delay,
            scope_domain.clone(),
            user_agent.clone(),
            semaphore.clone(),
            quiet,
        ));
    }
    while futures.next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::Semaphore;

    #[tokio::test]
    async fn test_crawl_basic() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="/page2">Page 2</a></body></html>"#)
            .create_async()
            .await;
        let _m2 = server
            .mock("GET", "/page2")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="/">Home</a></body></html>"#)
            .create_async()
            .await;
        let start_url = server.url();
        let max_depth = 2;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url)
            .unwrap()
            .host_str()
            .map(|s| s.to_string());
        let user_agent = "test-agent".to_string();
        let semaphore = Arc::new(Semaphore::new(10));
        crawl(
            client,
            start_url.clone(),
            max_depth,
            visited.clone(),
            delay,
            scope_domain,
            user_agent,
            semaphore,
            false, // not quiet
        )
        .await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        assert!(visited_links.contains(&format!("{}/page2", norm_start_url.trim_end_matches('/'))));
    }

    #[tokio::test]
    async fn test_crawl_scope() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="https://other.com/page">Other</a></body></html>"#)
            .create_async()
            .await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url)
            .unwrap()
            .host_str()
            .map(|s| s.to_string());
        let user_agent = "test-agent".to_string();
        let semaphore = Arc::new(Semaphore::new(10));
        crawl(
            client,
            start_url.clone(),
            max_depth,
            visited.clone(),
            delay,
            scope_domain,
            user_agent,
            semaphore,
            true, // quiet
        )
        .await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(!visited_links.iter().any(|u| u.contains("other.com")));
        assert!(visited_links.contains(&norm_start_url));
    }

    #[tokio::test]
    async fn test_crawl_non_http_links() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="mailto:test@example.com">Mail</a></body></html>"#)
            .create_async()
            .await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url)
            .unwrap()
            .host_str()
            .map(|s| s.to_string());
        let user_agent = "test-agent".to_string();
        let semaphore = Arc::new(Semaphore::new(10));
        crawl(
            client,
            start_url.clone(),
            max_depth,
            visited.clone(),
            delay,
            scope_domain,
            user_agent,
            semaphore,
            true, // quiet
        )
        .await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        assert_eq!(visited_links.len(), 1);
    }

    #[tokio::test]
    async fn test_crawl_concurrency() {
        let mut server = mockito::Server::new_async().await;
        for i in 0..5 {
            server
                .mock("GET", format!("/page{}", i).as_str())
                .with_status(200)
                .with_header("content-type", "text/html")
                .with_body("<html></html>")
                .create_async()
                .await;
        }
        let _mroot = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(
                (0..5)
                    .map(|i| format!("<a href='/page{}'>Page</a>", i))
                    .collect::<String>(),
            )
            .create_async()
            .await;
        let start_url = server.url();
        let max_depth = 2;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url)
            .unwrap()
            .host_str()
            .map(|s| s.to_string());
        let user_agent = "test-agent".to_string();
        let semaphore = Arc::new(Semaphore::new(2)); // Only 2 concurrent
        crawl(
            client,
            start_url.clone(),
            max_depth,
            visited.clone(),
            delay,
            scope_domain,
            user_agent,
            semaphore,
            false, // not quiet
        )
        .await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        for i in 0..5 {
            assert!(visited_links.contains(&format!(
                "{}/page{}",
                norm_start_url.trim_end_matches('/'),
                i
            )));
        }
    }

    #[tokio::test]
    async fn test_crawl_error_handling() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server
            .mock("GET", "/")
            .with_status(404)
            .with_header("content-type", "text/html")
            .with_body("Not found")
            .create_async()
            .await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url)
            .unwrap()
            .host_str()
            .map(|s| s.to_string());
        let user_agent = "test-agent".to_string();
        let semaphore = Arc::new(Semaphore::new(10));
        crawl(
            client,
            start_url.clone(),
            max_depth,
            visited.clone(),
            delay,
            scope_domain,
            user_agent,
            semaphore,
            true, // quiet
        )
        .await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
    }
}
