//! Parsing of sitemaps (<https://www.sitemaps.org/protocol.html>).
//!
//! Supports XML `<urlset>` and `<sitemapindex>` documents, plain-text
//! sitemaps (one URL per line), and gzip-compressed variants of both.

use flate2::read::GzDecoder;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;

/// Largest sitemap accepted after decompression (the protocol limit is 50 MiB).
pub const MAX_SITEMAP_BYTES: u64 = 50 * 1024 * 1024;

/// What a sitemap document lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sitemap {
    /// Page URLs (`<urlset>` or a plain-text sitemap).
    Urls(Vec<String>),
    /// URLs of further sitemaps (`<sitemapindex>`).
    Index(Vec<String>),
}

/// Parses a sitemap body, decompressing it first if it is gzipped.
pub fn parse(body: &[u8]) -> Result<Sitemap, String> {
    let body = decompress(body)?;
    let text = String::from_utf8_lossy(&body);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with('<') {
        parse_xml(text)
    } else {
        Ok(Sitemap::Urls(
            text.lines()
                .map(str::trim)
                .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
                .map(str::to_string)
                .collect(),
        ))
    }
}

/// Gunzips `body` if it starts with the gzip magic bytes, capping the output size.
fn decompress(body: &[u8]) -> Result<Vec<u8>, String> {
    if !body.starts_with(&[0x1f, 0x8b]) {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    GzDecoder::new(body)
        .take(MAX_SITEMAP_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("invalid gzip data: {}", e))?;
    if out.len() as u64 > MAX_SITEMAP_BYTES {
        return Err("sitemap exceeds 50 MiB after decompression".into());
    }
    Ok(out)
}

fn parse_xml(xml: &str) -> Result<Sitemap, String> {
    let mut reader = Reader::from_str(xml);
    let mut is_index = None;
    let mut in_loc = false;
    let mut current = String::new();
    let mut locs = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                let name = name.as_ref();
                if is_index.is_none() {
                    is_index = match name {
                        "urlset" => Some(false),
                        "sitemapindex" => Some(true),
                        _ => {
                            return Err("not a sitemap (expected <urlset> or <sitemapindex>)".into())
                        }
                    };
                } else if name == "loc" {
                    in_loc = true;
                    current.clear();
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == "loc" => {
                in_loc = false;
                let loc = current.trim();
                if !loc.is_empty() {
                    locs.push(loc.to_string());
                }
            }
            Ok(Event::Text(t)) if in_loc => current.push_str(&t.xml10_content()),
            Ok(Event::CData(t)) if in_loc => current.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) if in_loc => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    current.push(c);
                } else if let Some(s) = resolve_predefined_entity(&r) {
                    current.push_str(s);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("invalid XML: {}", e)),
        }
    }

    match is_index {
        Some(true) => Ok(Sitemap::Index(locs)),
        Some(false) => Ok(Sitemap::Urls(locs)),
        None => Err("empty sitemap".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn parses_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
              <url><loc>https://example.com/</loc><lastmod>2024-01-01</lastmod></url>
              <url><loc> https://example.com/a?x=1&amp;y=2 </loc></url>
              <url><loc><![CDATA[https://example.com/b]]></loc></url>
            </urlset>"#;
        assert_eq!(
            parse(xml.as_bytes()).unwrap(),
            Sitemap::Urls(vec![
                "https://example.com/".into(),
                "https://example.com/a?x=1&y=2".into(),
                "https://example.com/b".into(),
            ])
        );
    }

    #[test]
    fn parses_sitemap_index() {
        let xml = r#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
              <sitemap><loc>https://example.com/s1.xml</loc></sitemap>
              <sitemap><loc>https://example.com/s2.xml.gz</loc></sitemap>
            </sitemapindex>"#;
        assert_eq!(
            parse(xml.as_bytes()).unwrap(),
            Sitemap::Index(vec![
                "https://example.com/s1.xml".into(),
                "https://example.com/s2.xml.gz".into(),
            ])
        );
    }

    #[test]
    fn parses_prefixed_namespace() {
        let xml = r#"<sm:urlset xmlns:sm="http://www.sitemaps.org/schemas/sitemap/0.9">
              <sm:url><sm:loc>https://example.com/p</sm:loc></sm:url>
            </sm:urlset>"#;
        assert_eq!(
            parse(xml.as_bytes()).unwrap(),
            Sitemap::Urls(vec!["https://example.com/p".into()])
        );
    }

    #[test]
    fn parses_plain_text() {
        let txt = "https://example.com/a\n\n  https://example.com/b  \nnot a url\n";
        assert_eq!(
            parse(txt.as_bytes()).unwrap(),
            Sitemap::Urls(vec![
                "https://example.com/a".into(),
                "https://example.com/b".into()
            ])
        );
    }

    #[test]
    fn parses_gzip() {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(b"<urlset><url><loc>https://example.com/z</loc></url></urlset>")
            .unwrap();
        assert_eq!(
            parse(&gz.finish().unwrap()).unwrap(),
            Sitemap::Urls(vec!["https://example.com/z".into()])
        );
    }

    #[test]
    fn rejects_html() {
        assert!(parse(b"<html><body>Not found</body></html>").is_err());
    }
}
