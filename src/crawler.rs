use reqwest::{Client, Error};
use std::collections::HashSet;
use std::io::{stdout, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use url::Url;
use futures::stream::{FuturesUnordered, StreamExt};
use crate::extractor::{extract_links, normalize_url};

async fn fetch_url(client: &Client, url: &str) -> Result<String, Error> {
    let response = client.get(url).send().await?;
    let body = response.text().await?;
    Ok(body)
}

pub async fn crawl(
    client: Client,
    url: String,
    depth: usize,
    visited: Arc<Mutex<HashSet<String>>>,
    delay: Duration,
    scope_domain: Option<String>,
    user_agent: String,
    semaphore: Arc<Semaphore>,
    _quiet: bool,
) {
    if depth == 0 {
        return;
    }

    let normalized_url = normalize_url(&url);
    {
        let mut visited_guard = visited.lock().unwrap();
        if visited_guard.contains(&normalized_url) {
            return;
        }
        visited_guard.insert(normalized_url.clone());
    }

    let _permit = semaphore.acquire().await.unwrap();

    println!("{}", normalized_url);
    stdout().flush().unwrap();

    match fetch_url(&client, &url).await {
        Ok(body) => {
            let links = extract_links(&body, &url);
            let mut futures = FuturesUnordered::new();
            for link in links {
                if let Ok(parsed_link) = Url::parse(&link) {
                    if parsed_link.scheme() != "http" && parsed_link.scheme() != "https" {
                        println!("Skipping non-HTTP URL: {}", link);
                        continue;
                    }
                } else {
                    println!("Skipping invalid URL: {}", link);
                    continue;
                }

                if let Some(scope) = &scope_domain {
                    if let Ok(parsed_url) = Url::parse(&link) {
                        if let Some(host) = parsed_url.host_str() {
                            let is_in_scope = if scope.starts_with('.') {
                                host.ends_with(scope)
                            } else {
                                host == scope || host.ends_with(&format!(".{}", scope))
                            };
                            if !is_in_scope {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                let new_depth = if depth > 0 { depth - 1 } else { 0 };
                futures.push(crawl(
                    client.clone(),
                    link.to_string(),
                    new_depth,
                    visited.clone(),
                    delay,
                    scope_domain.clone(),
                    user_agent.clone(),
                    semaphore.clone(),
                    false,
                ));
            }
            while let Some(_) = futures.next().await {}
        }
        Err(e) => eprintln!("Error crawling {}: {:?}", normalize_url(&url), e),
    }
    sleep(delay).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use std::sync::{Arc, Mutex};
    use std::collections::HashSet;
    use std::time::Duration;
    use tokio::sync::Semaphore;

    #[tokio::test]
    async fn test_crawl_basic() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server.mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="/page2">Page 2</a></body></html>"#)
            .create_async().await;
        let _m2 = server.mock("GET", "/page2")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="/">Home</a></body></html>"#)
            .create_async().await;
        let start_url = server.url();
        let max_depth = 2;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url).unwrap().host_str().map(|s| s.to_string());
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
        ).await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        assert!(visited_links.contains(&format!("{}/page2", norm_start_url.trim_end_matches('/'))));
    }

    #[tokio::test]
    async fn test_crawl_scope() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server.mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="https://other.com/page">Other</a></body></html>"#)
            .create_async().await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url).unwrap().host_str().map(|s| s.to_string());
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
        ).await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(!visited_links.iter().any(|u| u.contains("other.com")));
        assert!(visited_links.contains(&norm_start_url));
    }

    #[tokio::test]
    async fn test_crawl_non_http_links() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server.mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(r#"<html><body><a href="mailto:test@example.com">Mail</a></body></html>"#)
            .create_async().await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url).unwrap().host_str().map(|s| s.to_string());
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
        ).await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        assert_eq!(visited_links.len(), 1);
    }

    #[tokio::test]
    async fn test_crawl_concurrency() {
        let mut server = mockito::Server::new_async().await;
        for i in 0..5 {
            server.mock("GET", format!("/page{}", i).as_str())
                .with_status(200)
                .with_header("content-type", "text/html")
                .with_body("<html></html>")
                .create_async().await;
        }
        let _mroot = server.mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body((0..5).map(|i| format!("<a href='/page{}'>Page</a>", i)).collect::<String>())
            .create_async().await;
        let start_url = server.url();
        let max_depth = 2;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url).unwrap().host_str().map(|s| s.to_string());
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
        ).await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
        for i in 0..5 {
            assert!(visited_links.contains(&format!("{}/page{}", norm_start_url.trim_end_matches('/'), i)));
        }
    }

    #[tokio::test]
    async fn test_crawl_error_handling() {
        let mut server = mockito::Server::new_async().await;
        let _m1 = server.mock("GET", "/")
            .with_status(404)
            .with_header("content-type", "text/html")
            .with_body("Not found")
            .create_async().await;
        let start_url = server.url();
        let max_depth = 1;
        let visited = Arc::new(Mutex::new(HashSet::new()));
        let client = Client::new();
        let delay = Duration::from_millis(10);
        let scope_domain = Url::parse(&start_url).unwrap().host_str().map(|s| s.to_string());
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
        ).await;
        let visited_links = visited.lock().unwrap();
        let norm_start_url = normalize_url(&start_url);
        assert!(visited_links.contains(&norm_start_url));
    }
}