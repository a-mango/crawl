use crawl::crawler::{crawl, CrawlConfig, CrawlReport};
use crawl::extractor::normalize_url;
use mockito::{Matcher, Server, ServerGuard};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Fast settings for tests; robots.txt is off unless a test opts in.
fn config(max_depth: usize) -> CrawlConfig {
    CrawlConfig {
        max_depth,
        delay: Duration::from_millis(1),
        retry_base_delay: Duration::from_millis(1),
        respect_robots: false,
        quiet: true,
        ..CrawlConfig::default()
    }
}

async fn page(server: &mut ServerGuard, path: &str, body: &str) -> mockito::Mock {
    server
        .mock("GET", path)
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(body)
        .create_async()
        .await
}

fn url(server: &ServerGuard, path: &str) -> String {
    normalize_url(&format!("{}{}", server.url(), path))
}

async fn run_report(server: &ServerGuard, config: CrawlConfig) -> CrawlReport {
    tokio::time::timeout(Duration::from_secs(10), crawl(&server.url(), config))
        .await
        .expect("crawl timed out")
        .expect("crawl failed")
}

async fn run(server: &ServerGuard, config: CrawlConfig) -> HashSet<String> {
    run_report(server, config).await.urls()
}

async fn robots(server: &mut ServerGuard, body: &str) -> mockito::Mock {
    server
        .mock("GET", "/robots.txt")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await
}

#[tokio::test]
async fn follows_links_and_handles_cycles() {
    let mut server = Server::new_async().await;
    let _m = [
        page(
            &mut server,
            "/",
            "<a href='/about'>A</a><a href='/contact'>C</a>",
        )
        .await,
        page(
            &mut server,
            "/about",
            "<a href='/'>H</a><a href='/team'>T</a>",
        )
        .await,
        page(&mut server, "/contact", "<a href='/'>H</a>").await,
        page(&mut server, "/team", "<a href='/about'>A</a>").await,
    ];

    let visited = run(&server, config(3)).await;

    let expected: HashSet<_> = ["/", "/about", "/contact", "/team"]
        .iter()
        .map(|p| url(&server, p))
        .collect();
    assert_eq!(visited, expected);
}

#[tokio::test]
async fn respects_depth_limit() {
    let mut server = Server::new_async().await;
    let _m = [
        page(&mut server, "/", "<a href='/level1'>1</a>").await,
        page(&mut server, "/level1", "<a href='/level2'>2</a>").await,
        page(&mut server, "/level2", "<a href='/level3'>3</a>").await,
    ];

    let visited = run(&server, config(2)).await;

    assert!(visited.contains(&url(&server, "/level1")));
    assert!(!visited.contains(&url(&server, "/level2")));
}

#[tokio::test]
async fn skips_non_http_schemes() {
    let mut server = Server::new_async().await;
    let _m = [
        page(
            &mut server,
            "/",
            "<a href='/valid'>V</a><a href='mailto:a@b.c'>M</a><a href='javascript:void(0)'>J</a>\
             <a href='ftp://ftp.example.com'>F</a><a href='tel:+123'>T</a>",
        )
        .await,
        page(&mut server, "/valid", "ok").await,
    ];

    let visited = run(&server, config(2)).await;

    assert_eq!(visited.len(), 2);
}

#[tokio::test]
async fn scope_excludes_other_hosts() {
    let mut server = Server::new_async().await;
    let _m = [
        page(
            &mut server,
            "/",
            "<a href='/page1'>1</a><a href='https://external.com/x'>E</a>\
             <a href='https://sub.external.com/x'>S</a>",
        )
        .await,
        page(&mut server, "/page1", "ok").await,
    ];
    let host = url::Url::parse(&server.url())
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();

    let visited = run(
        &server,
        CrawlConfig {
            scope: Some(host),
            ..config(2)
        },
    )
    .await;

    assert_eq!(visited.len(), 2);
    assert!(!visited.iter().any(|u| u.contains("external.com")));
}

#[tokio::test]
async fn error_pages_are_visited_but_not_parsed() {
    let mut server = Server::new_async().await;
    let _root = page(&mut server, "/", "<a href='/missing'>M</a>").await;
    let _missing = server
        .mock("GET", "/missing")
        .with_status(404)
        .with_header("content-type", "text/html")
        .with_body("<a href='/hidden'>H</a>")
        .create_async()
        .await;

    let visited = run(&server, config(3)).await;

    assert!(visited.contains(&url(&server, "/missing")));
    assert!(!visited.contains(&url(&server, "/hidden")));
}

#[tokio::test]
async fn deep_crawl_with_low_concurrency_does_not_hang() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(404)
        .create_async()
        .await;
    let _m = [
        page(&mut server, "/", "<a href='/a'>a</a>").await,
        page(&mut server, "/a", "<a href='/b'>b</a>").await,
        page(&mut server, "/b", "<a href='/c'>c</a>").await,
        page(&mut server, "/c", "end").await,
    ];

    let visited = run(
        &server,
        CrawlConfig {
            concurrency: 1,
            respect_robots: true, // robots.txt fetch also needs a permit
            ..config(4)
        },
    )
    .await;

    assert_eq!(visited.len(), 4);
}

#[tokio::test]
async fn sends_configured_user_agent() {
    let mut server = Server::new_async().await;
    let ua = server
        .mock("GET", "/")
        .match_header("user-agent", "TestBot/9.9")
        .with_status(200)
        .with_body("ok")
        .create_async()
        .await;

    run(
        &server,
        CrawlConfig {
            user_agent: "TestBot/9.9".into(),
            ..config(1)
        },
    )
    .await;

    ua.assert_async().await;
}

#[tokio::test]
async fn retries_server_errors_then_succeeds() {
    let mut server = Server::new_async().await;
    let failing = server
        .mock("GET", "/")
        .with_status(503)
        .expect(2)
        .create_async()
        .await;
    let ok = page(&mut server, "/", "<a href='/next'>n</a>").await;
    let _next = page(&mut server, "/next", "ok").await;

    let visited = run(&server, config(2)).await;

    failing.assert_async().await;
    ok.assert_async().await;
    assert!(visited.contains(&url(&server, "/next")));
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let mut server = Server::new_async().await;
    let failing = server
        .mock("GET", "/")
        .with_status(500)
        .expect(3) // first attempt + 2 retries
        .create_async()
        .await;

    run(
        &server,
        CrawlConfig {
            max_retries: 2,
            ..config(1)
        },
    )
    .await;

    failing.assert_async().await;
}

#[tokio::test]
async fn does_not_retry_client_errors() {
    let mut server = Server::new_async().await;
    let missing = server
        .mock("GET", "/")
        .with_status(404)
        .expect(1)
        .create_async()
        .await;

    run(&server, config(1)).await;

    missing.assert_async().await;
}

#[tokio::test]
async fn honours_retry_after_on_429() {
    let mut server = Server::new_async().await;
    let limited = server
        .mock("GET", "/")
        .with_status(429)
        .with_header("retry-after", "1")
        .expect(1)
        .create_async()
        .await;
    let _ok = page(&mut server, "/", "ok").await;

    let start = Instant::now();
    run(&server, config(1)).await;

    limited.assert_async().await;
    assert!(start.elapsed() >= Duration::from_secs(1));
}

#[tokio::test]
async fn robots_txt_disallowed_paths_are_skipped() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(200)
        .with_body("User-agent: *\nDisallow: /private\n")
        .create_async()
        .await;
    let _m = [
        page(
            &mut server,
            "/",
            "<a href='/public'>p</a><a href='/private/x'>x</a>",
        )
        .await,
        page(&mut server, "/public", "ok").await,
    ];
    let private = server
        .mock("GET", Matcher::Regex("^/private".into()))
        .expect(0)
        .create_async()
        .await;

    let visited = run(
        &server,
        CrawlConfig {
            respect_robots: true,
            ..config(2)
        },
    )
    .await;

    private.assert_async().await;
    assert!(visited.contains(&url(&server, "/public")));
    assert!(!visited.contains(&url(&server, "/private/x")));
}

#[tokio::test]
async fn robots_txt_rules_match_our_user_agent() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(200)
        .with_body("User-agent: TestBot\nDisallow: /\n\nUser-agent: *\nAllow: /\n")
        .create_async()
        .await;
    let root = page(&mut server, "/", "ok").await.expect(0);

    let visited = run(
        &server,
        CrawlConfig {
            respect_robots: true,
            user_agent: "TestBot/1.0".into(),
            ..config(1)
        },
    )
    .await;

    root.assert_async().await;
    assert!(visited.is_empty());
}

#[tokio::test]
async fn missing_robots_txt_allows_everything() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(404)
        .create_async()
        .await;
    let _root = page(&mut server, "/", "ok").await;

    let visited = run(
        &server,
        CrawlConfig {
            respect_robots: true,
            ..config(1)
        },
    )
    .await;

    assert_eq!(visited.len(), 1);
}

#[tokio::test]
async fn unreachable_robots_txt_disallows_everything() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(503)
        .create_async()
        .await;
    let root = page(&mut server, "/", "ok").await.expect(0);

    let visited = run(
        &server,
        CrawlConfig {
            respect_robots: true,
            ..config(1)
        },
    )
    .await;

    root.assert_async().await;
    assert!(visited.is_empty());
}

#[tokio::test]
async fn robots_txt_is_fetched_once_per_site() {
    let mut server = Server::new_async().await;
    let robots = server
        .mock("GET", "/robots.txt")
        .with_status(200)
        .with_body("User-agent: *\nAllow: /\n")
        .expect(1)
        .create_async()
        .await;
    let _m = [
        page(
            &mut server,
            "/",
            "<a href='/a'>a</a><a href='/b'>b</a><a href='/c'>c</a>",
        )
        .await,
        page(&mut server, "/a", "ok").await,
        page(&mut server, "/b", "ok").await,
        page(&mut server, "/c", "ok").await,
    ];

    let visited = run(
        &server,
        CrawlConfig {
            respect_robots: true,
            ..config(2)
        },
    )
    .await;

    robots.assert_async().await;
    assert_eq!(visited.len(), 4);
}

#[tokio::test]
async fn records_status_depth_and_links_per_page() {
    let mut server = Server::new_async().await;
    let _root = page(
        &mut server,
        "/",
        "<a href='/ok'>o</a><a href='/gone'>g</a><a href='mailto:x@y.z'>m</a>\
         <a href='https://elsewhere.test/'>e</a>",
    )
    .await;
    let _ok = page(&mut server, "/ok", "fine").await;
    let _gone = server
        .mock("GET", "/gone")
        .with_status(410)
        .create_async()
        .await;
    let host = url::Url::parse(&server.url())
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();

    let report = run_report(
        &server,
        CrawlConfig {
            scope: Some(host),
            ..config(2)
        },
    )
    .await;

    let get = |path: &str| {
        report
            .pages
            .iter()
            .find(|p| p.url == url(&server, path))
            .unwrap_or_else(|| panic!("{} not visited", path))
    };
    let root = get("/");
    assert_eq!((root.depth, root.status), (0, Some(200)));
    // Out-of-scope links are recorded but not followed; mailto is dropped.
    let mut expected = vec![
        "https://elsewhere.test/".to_string(),
        url(&server, "/gone"),
        url(&server, "/ok"),
    ];
    expected.sort();
    assert_eq!(root.links, expected);
    assert_eq!((get("/ok").depth, get("/ok").status), (1, Some(200)));
    assert_eq!(get("/gone").status, Some(410));
    assert_eq!(report.pages.len(), 3);
}

#[tokio::test]
async fn relative_links_resolve_against_redirect_target() {
    let mut server = Server::new_async().await;
    let _root = server
        .mock("GET", "/")
        .with_status(301)
        .with_header("location", "/docs/")
        .create_async()
        .await;
    let _docs = page(&mut server, "/docs/", "<a href='intro'>i</a>").await;
    let _intro = page(&mut server, "/docs/intro", "ok").await;

    let visited = run(&server, config(2)).await;

    assert!(visited.contains(&url(&server, "/docs/intro")));
}

#[tokio::test]
async fn max_pages_stops_the_crawl() {
    let mut server = Server::new_async().await;
    let links: String = (0..10)
        .map(|i| format!("<a href='/p{}'>p</a>", i))
        .collect();
    let _root = page(&mut server, "/", &links).await;
    let _pages = server
        .mock("GET", Matcher::Regex("^/p[0-9]$".into()))
        .with_status(200)
        .with_body("ok")
        .create_async()
        .await;

    let visited = run(
        &server,
        CrawlConfig {
            max_pages: Some(4),
            ..config(2)
        },
    )
    .await;

    assert_eq!(visited.len(), 4);
    assert!(visited.contains(&url(&server, "/")));
}

#[tokio::test]
async fn sitemaps_from_robots_txt_seed_the_crawl() {
    let mut server = Server::new_async().await;
    let base = server.url();
    let _robots = robots(
        &mut server,
        &format!("User-agent: *\nAllow: /\nSitemap: {}/index.xml\n", base),
    )
    .await;
    let _index = server
        .mock("GET", "/index.xml")
        .with_status(200)
        .with_body(format!(
            "<sitemapindex><sitemap><loc>{0}/pages.xml</loc></sitemap>\
             <sitemap><loc>{0}/more.txt</loc></sitemap></sitemapindex>",
            base
        ))
        .create_async()
        .await;
    let _pages = server
        .mock("GET", "/pages.xml")
        .with_status(200)
        .with_body(format!(
            "<urlset><url><loc>{0}/orphan</loc></url>\
             <url><loc>https://other.test/x</loc></url></urlset>",
            base
        ))
        .create_async()
        .await;
    let _more = server
        .mock("GET", "/more.txt")
        .with_status(200)
        .with_body(format!("{}/orphan2\n", base))
        .create_async()
        .await;
    let _m = [
        page(&mut server, "/", "no links").await,
        page(&mut server, "/orphan", "ok").await,
        page(&mut server, "/orphan2", "ok").await,
    ];
    let host = url::Url::parse(&base)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();

    let visited = run(
        &server,
        CrawlConfig {
            use_sitemaps: true,
            respect_robots: true,
            scope: Some(host),
            ..config(1)
        },
    )
    .await;

    let expected: HashSet<_> = ["/", "/orphan", "/orphan2"]
        .iter()
        .map(|p| url(&server, p))
        .collect();
    assert_eq!(
        visited, expected,
        "out-of-scope sitemap entries are skipped"
    );
}

#[tokio::test]
async fn sitemap_falls_back_to_sitemap_xml() {
    let mut server = Server::new_async().await;
    let _robots = server
        .mock("GET", "/robots.txt")
        .with_status(404)
        .create_async()
        .await;
    let _sitemap = server
        .mock("GET", "/sitemap.xml")
        .with_status(200)
        .with_body(format!(
            "<urlset><url><loc>{}/hidden</loc></url></urlset>",
            server.url()
        ))
        .create_async()
        .await;
    let _m = [
        page(&mut server, "/", "no links").await,
        page(&mut server, "/hidden", "ok").await,
    ];

    // Robots ignored: robots.txt is still read for Sitemap lines.
    let visited = run(
        &server,
        CrawlConfig {
            use_sitemaps: true,
            ..config(1)
        },
    )
    .await;

    assert!(visited.contains(&url(&server, "/hidden")));
}

#[tokio::test]
async fn sitemap_entries_still_obey_robots_txt() {
    let mut server = Server::new_async().await;
    let base = server.url();
    let _robots = robots(
        &mut server,
        &format!(
            "User-agent: *\nDisallow: /private\nSitemap: {}/s.xml\n",
            base
        ),
    )
    .await;
    let _sitemap = server
        .mock("GET", "/s.xml")
        .with_status(200)
        .with_body(format!(
            "<urlset><url><loc>{0}/private/a</loc></url><url><loc>{0}/public</loc></url></urlset>",
            base
        ))
        .create_async()
        .await;
    let _m = [
        page(&mut server, "/", "x").await,
        page(&mut server, "/public", "x").await,
    ];
    let private = server
        .mock("GET", "/private/a")
        .expect(0)
        .create_async()
        .await;

    let visited = run(
        &server,
        CrawlConfig {
            use_sitemaps: true,
            respect_robots: true,
            ..config(1)
        },
    )
    .await;

    private.assert_async().await;
    assert!(visited.contains(&url(&server, "/public")));
}
