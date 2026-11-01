use scraper::{Html, Selector};
use std::collections::HashSet;
use url::Url;

pub fn normalize_url(url_str: &str) -> String {
    if let Ok(mut url) = Url::parse(url_str) {
        url.set_query(None);
        url.set_fragment(None);
        url.to_string()
    } else {
        url_str.to_string()
    }
}

pub fn extract_links(html: &str, base_url: &str) -> HashSet<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a").unwrap();
    let mut links = HashSet::new();
    let base = Url::parse(base_url).ok();
    for element in document.select(&selector) {
        if let Some(link) = element.value().attr("href") {
            if let Some(base) = &base {
                if let Ok(abs_link) = base.join(link) {
                    links.insert(abs_link.to_string());
                }
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_extract_links_absolute_and_relative() {
        let html = "
            <html>
                <body>
                    <a href='https://example.com/page1'>Page 1</a>
                    <a href='/page2'>Page 2</a>
                    <a href='#fragment'>Fragment</a>
                </body>
            </html>";
        let base_url = "https://example.com";
        let links = extract_links(html, base_url);
        let mut expected_links = HashSet::new();
        expected_links.insert("https://example.com/page1".to_string());
        expected_links.insert("https://example.com/page2".to_string());
        expected_links.insert("https://example.com/#fragment".to_string());
        assert_eq!(links, expected_links);
    }

    #[test]
    fn test_normalize_url_removes_query_and_fragment() {
        let url = "https://example.com/page?foo=bar#frag";
        let normalized = normalize_url(url);
        assert_eq!(normalized, "https://example.com/page");
    }

    #[test]
    fn test_normalize_url_invalid_url() {
        let url = "not a url";
        let normalized = normalize_url(url);
        assert_eq!(normalized, url);
    }

    #[test]
    fn test_extract_links_empty() {
        let html = "<html></html>";
        let base_url = "https://example.com";
        let links = extract_links(html, base_url);
        assert!(links.is_empty());
    }
}
