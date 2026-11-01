//! Saving crawl results to SQLite.
//!
//! Each call to [`save_sqlite`] appends one crawl to the database, so a single
//! file can hold the history of many crawls. Timestamps are Unix milliseconds;
//! use `datetime(col / 1000, 'unixepoch')` to read them in SQL.

use crate::crawler::CrawlReport;
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS crawls (
    id              INTEGER PRIMARY KEY,
    start_url       TEXT    NOT NULL,
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER NOT NULL,
    page_count      INTEGER NOT NULL,
    crawler_version TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS pages (
    crawl_id     INTEGER NOT NULL REFERENCES crawls(id) ON DELETE CASCADE,
    url          TEXT    NOT NULL,
    depth        INTEGER NOT NULL,
    status       INTEGER,
    content_type TEXT,
    error        TEXT,
    fetched_at   INTEGER NOT NULL,
    PRIMARY KEY (crawl_id, url)
);
CREATE TABLE IF NOT EXISTS links (
    crawl_id INTEGER NOT NULL REFERENCES crawls(id) ON DELETE CASCADE,
    source   TEXT    NOT NULL,
    target   TEXT    NOT NULL,
    PRIMARY KEY (crawl_id, source, target)
);
CREATE INDEX IF NOT EXISTS links_by_target ON links (crawl_id, target);
";

/// Returns true if `path` has an extension that should be written as SQLite.
pub fn is_sqlite_path(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("db" | "sqlite" | "sqlite3"))
}

fn millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Opens (or creates) the database at `path` and creates the schema if needed.
pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "database schema version {} is newer than supported version {}",
            version, SCHEMA_VERSION
        )));
    }
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
}

/// Appends a crawl to the database at `path` and returns its `crawls.id`.
pub fn save_sqlite(
    path: impl AsRef<Path>,
    start_url: &str,
    started_at: SystemTime,
    finished_at: SystemTime,
    report: &CrawlReport,
) -> rusqlite::Result<i64> {
    let mut conn = open(path)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO crawls (start_url, started_at, finished_at, page_count, crawler_version)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            start_url,
            millis(started_at),
            millis(finished_at),
            report.pages.len() as i64,
            env!("CARGO_PKG_VERSION"),
        ],
    )?;
    let crawl_id = tx.last_insert_rowid();
    {
        let mut page_stmt = tx.prepare(
            "INSERT OR IGNORE INTO pages
                 (crawl_id, url, depth, status, content_type, error, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut link_stmt = tx.prepare(
            "INSERT OR IGNORE INTO links (crawl_id, source, target) VALUES (?1, ?2, ?3)",
        )?;
        for page in &report.pages {
            page_stmt.execute(params![
                crawl_id,
                page.url,
                page.depth as i64,
                page.status,
                page.content_type,
                page.error,
                millis(page.fetched_at),
            ])?;
            for target in &page.links {
                link_stmt.execute(params![crawl_id, page.url, target])?;
            }
        }
    }
    tx.commit()?;
    Ok(crawl_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crawler::Page;

    fn page(url: &str, status: Option<u16>, links: &[&str]) -> Page {
        Page {
            url: url.into(),
            depth: 0,
            status,
            content_type: Some("text/html".into()),
            error: None,
            links: links.iter().map(|l| l.to_string()).collect(),
            fetched_at: SystemTime::now(),
        }
    }

    #[test]
    fn detects_sqlite_extensions() {
        assert!(is_sqlite_path("out.db"));
        assert!(is_sqlite_path("/data/out.SQLite"));
        assert!(is_sqlite_path("out.sqlite3"));
        assert!(!is_sqlite_path("out.json"));
        assert!(!is_sqlite_path("db"));
    }

    #[test]
    fn saves_pages_and_links_and_appends_crawls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crawl.db");
        let report = CrawlReport {
            pages: vec![
                page(
                    "https://a.test/",
                    Some(200),
                    &["https://a.test/x", "https://b.test/"],
                ),
                page("https://a.test/x", Some(404), &[]),
            ],
        };
        let now = SystemTime::now();

        let first = save_sqlite(&path, "https://a.test/", now, now, &report).unwrap();
        let second = save_sqlite(&path, "https://a.test/", now, now, &report).unwrap();
        assert_ne!(first, second);

        let conn = open(&path).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM crawls"), 2);
        assert_eq!(count("SELECT COUNT(*) FROM pages"), 4);
        assert_eq!(count("SELECT COUNT(*) FROM links"), 4);
        assert_eq!(count("SELECT page_count FROM crawls WHERE id = 1"), 2);
        let status: i64 = conn
            .query_row(
                "SELECT status FROM pages WHERE crawl_id = ?1 AND url = 'https://a.test/x'",
                [first],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, 404);
    }
}
