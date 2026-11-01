use crawl::crawler::crawl;
use crawl::extractor::normalize_url;
use reqwest::Client;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

#[tokio::test]
async fn test_crawl_with_circular_links_and_depth_limit() {
    let mut server = mockito::Server::new_async().await;

    // Create a web of interconnected pages
    let _m_home = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(
            "<html><body><a href='/about'>About</a><a href='/contact'>Contact</a></body></html>",
        )
        .create_async()
        .await;

    let _m_about = server
        .mock("GET", "/about")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/'>Home</a><a href='/team'>Team</a></body></html>")
        .create_async()
        .await;

    let _m_contact = server
        .mock("GET", "/contact")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/'>Home</a></body></html>")
        .create_async()
        .await;

    let _m_team = server
        .mock("GET", "/team")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/about'>About</a></body></html>")
        .create_async()
        .await;

    let start_url = server.url();
    let visited = Arc::new(Mutex::new(HashSet::new()));
    let client = Client::new();

    crawl(
        client,
        start_url.clone(),
        3, // depth of 3 should allow visiting all pages
        visited.clone(),
        Duration::from_millis(5),
        None, // no scope restriction
        "integration-test-agent".to_string(),
        Arc::new(Semaphore::new(5)),
        false, // not quiet
    )
    .await;

    let visited_links = visited.lock().unwrap();
    let base = normalize_url(&start_url);

    // Verify all pages were visited despite circular links
    assert!(visited_links.contains(&base));
    assert!(visited_links.contains(&format!("{}/about", base.trim_end_matches('/'))));
    assert!(visited_links.contains(&format!("{}/contact", base.trim_end_matches('/'))));
    assert!(visited_links.contains(&format!("{}/team", base.trim_end_matches('/'))));
    assert_eq!(
        visited_links.len(),
        4,
        "Should visit exactly 4 unique pages"
    );
}

#[tokio::test]
async fn test_depth_limit_enforcement() {
    let mut server = mockito::Server::new_async().await;

    let _m1 = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/level1'>Level 1</a></body></html>")
        .create_async()
        .await;

    let _m2 = server
        .mock("GET", "/level1")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/level2'>Level 2</a></body></html>")
        .create_async()
        .await;

    let _m3 = server
        .mock("GET", "/level2")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body><a href='/level3'>Level 3</a></body></html>")
        .create_async()
        .await;

    let start_url = server.url();
    let visited = Arc::new(Mutex::new(HashSet::new()));
    let client = Client::new();

    crawl(
        client,
        start_url.clone(),
        2, // depth of 2 should only visit root and level1
        visited.clone(),
        Duration::from_millis(5),
        None,
        "test-agent".to_string(),
        Arc::new(Semaphore::new(10)),
        true, // quiet
    )
    .await;

    let visited_links = visited.lock().unwrap();
    let base = normalize_url(&start_url);

    // Should visit root and level1, but not level2
    assert!(visited_links.contains(&base));
    assert!(visited_links.contains(&format!("{}/level1", base.trim_end_matches('/'))));
    assert!(!visited_links.contains(&format!("{}/level2", base.trim_end_matches('/'))));
}

#[tokio::test]
async fn test_mixed_protocols_and_schemes() {
    let mut server = mockito::Server::new_async().await;

    let body = r#"
        <html><body>
            <a href='/valid'>Valid</a>
            <a href='mailto:test@example.com'>Email</a>
            <a href='javascript:void(0)'>JS</a>
            <a href='ftp://ftp.example.com'>FTP</a>
            <a href='tel:+1234567890'>Phone</a>
        </body></html>
    "#;

    let _m1 = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(body)
        .create_async()
        .await;

    let _m2 = server
        .mock("GET", "/valid")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body>Valid page</body></html>")
        .create_async()
        .await;

    let start_url = server.url();
    let visited = Arc::new(Mutex::new(HashSet::new()));
    let client = Client::new();

    crawl(
        client,
        start_url.clone(),
        2,
        visited.clone(),
        Duration::from_millis(5),
        None,
        "test-agent".to_string(),
        Arc::new(Semaphore::new(10)),
        true, // quiet
    )
    .await;

    let visited_links = visited.lock().unwrap();
    let base = normalize_url(&start_url);

    // Should only visit HTTP(S) links
    assert!(visited_links.contains(&base));
    assert!(visited_links.contains(&format!("{}/valid", base.trim_end_matches('/'))));
    assert_eq!(visited_links.len(), 2, "Should only visit HTTP links");
}

#[tokio::test]
async fn test_scope_filtering_integration() {
    let mut server = mockito::Server::new_async().await;

    let body = r#"
        <html><body>
            <a href='/page1'>Internal 1</a>
            <a href='/page2'>Internal 2</a>
            <a href='https://external.com/page'>External</a>
            <a href='https://sub.external.com/page'>Sub External</a>
        </body></html>
    "#;

    let _m1 = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(body)
        .create_async()
        .await;

    let _m2 = server
        .mock("GET", "/page1")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body>Page 1</body></html>")
        .create_async()
        .await;

    let _m3 = server
        .mock("GET", "/page2")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body>Page 2</body></html>")
        .create_async()
        .await;

    let start_url = server.url();
    let parsed_url = url::Url::parse(&start_url).unwrap();
    let scope = parsed_url.host_str().unwrap().to_string();

    let visited = Arc::new(Mutex::new(HashSet::<String>::new()));
    let client = Client::new();

    crawl(
        client,
        start_url.clone(),
        2,
        visited.clone(),
        Duration::from_millis(5),
        Some(scope),
        "test-agent".to_string(),
        Arc::new(Semaphore::new(10)),
        true, // quiet
    )
    .await;

    let visited_links = visited.lock().unwrap();
    let base = normalize_url(&start_url);

    // Should only visit in-scope links
    assert!(visited_links.contains(&base));
    assert!(visited_links.contains(&format!("{}/page1", base.trim_end_matches('/'))));
    assert!(visited_links.contains(&format!("{}/page2", base.trim_end_matches('/'))));
    assert!(!visited_links.iter().any(|url| url.contains("external.com")));
    assert_eq!(visited_links.len(), 3, "Should only visit in-scope pages");
}

#[tokio::test]
async fn test_http_status_codes_handling() {
    let mut server = mockito::Server::new_async().await;

    let body = r#"
        <html><body>
            <a href='/ok'>OK Page</a>
            <a href='/notfound'>Not Found</a>
            <a href='/error'>Server Error</a>
        </body></html>
    "#;

    let _m_root = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(body)
        .create_async()
        .await;

    let _m_ok = server
        .mock("GET", "/ok")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body>OK</body></html>")
        .create_async()
        .await;

    let _m_404 = server
        .mock("GET", "/notfound")
        .with_status(404)
        .with_header("content-type", "text/html")
        .with_body("Not Found")
        .create_async()
        .await;

    let _m_500 = server
        .mock("GET", "/error")
        .with_status(500)
        .with_header("content-type", "text/html")
        .with_body("Internal Server Error")
        .create_async()
        .await;

    let start_url = server.url();
    let visited = Arc::new(Mutex::new(HashSet::new()));
    let client = Client::new();

    crawl(
        client,
        start_url.clone(),
        2,
        visited.clone(),
        Duration::from_millis(5),
        None,
        "test-agent".to_string(),
        Arc::new(Semaphore::new(10)),
        true, // quiet
    )
    .await;

    let visited_links = visited.lock().unwrap();
    let base = normalize_url(&start_url);

    // All pages should be marked as visited, regardless of status code
    assert!(visited_links.contains(&base));
    assert!(visited_links.contains(&format!("{}/ok", base.trim_end_matches('/'))));
    assert!(visited_links.contains(&format!("{}/notfound", base.trim_end_matches('/'))));
    assert!(visited_links.contains(&format!("{}/error", base.trim_end_matches('/'))));
}

#[tokio::test]
async fn test_deep_crawl_with_low_concurrency_does_not_hang() {
    let mut server = mockito::Server::new_async().await;
    let mut mocks = Vec::new();
    for (path, next) in [("/", "/a"), ("/a", "/b"), ("/b", "/c"), ("/c", "/d")] {
        mocks.push(
            server
                .mock("GET", path)
                .with_status(200)
                .with_header("content-type", "text/html")
                .with_body(format!("<a href='{}'>next</a>", next))
                .create_async()
                .await,
        );
    }

    let start_url = server.url();
    let visited = Arc::new(Mutex::new(HashSet::new()));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        crawl(
            Client::new(),
            start_url,
            4,
            visited.clone(),
            Duration::from_millis(1),
            None,
            "test-agent".to_string(),
            Arc::new(Semaphore::new(1)),
            true,
        ),
    )
    .await;

    assert!(result.is_ok(), "crawl deadlocked with concurrency < depth");
    assert_eq!(visited.lock().unwrap().len(), 4);
}
