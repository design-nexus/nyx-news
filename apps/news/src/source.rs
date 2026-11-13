//! Publication name and date for a story. The address stays off the page.

use gtk::glib;

pub fn byline(url: &str, feed: &str, published_at: Option<i64>) -> String {
    let name = name(url, feed);
    let when = when(published_at);
    if when.is_empty() { name } else { format!("{name} · {when}") }
}

pub fn name(url: &str, feed: &str) -> String {
    let host = url.split("//").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let known = if host.contains("nytimes.com") {
        "The New York Times"
    } else if host.contains("bbc.") || host.contains("bbci.co.uk") {
        "BBC"
    } else if host.contains("theguardian.com") {
        "The Guardian"
    } else if host.contains("npr.org") {
        "NPR"
    } else if host.contains("espn.com") {
        "ESPN"
    } else if host.contains("aljazeera.com") {
        "Al Jazeera"
    } else if host.contains("nbcnews.com") {
        "NBC News"
    } else if host.contains("cbsnews.com") {
        "CBS News"
    } else if host.contains("abcnews.go.com") {
        "ABC News"
    } else if host.contains("wired.com") {
        "Wired"
    } else if host.contains("theverge.com") {
        "The Verge"
    } else if host.contains("arstechnica.com") {
        "Ars Technica"
    } else if host.contains("techcrunch.com") {
        "TechCrunch"
    } else if host.contains("technologyreview.com") {
        "MIT Technology Review"
    } else if host.contains("nature.com") {
        "Nature"
    } else if host.contains("science.org") {
        "Science"
    } else if host.contains("scientificamerican.com") {
        "Scientific American"
    } else if host.contains("newscientist.com") {
        "New Scientist"
    } else if host.contains("nasa.gov") {
        "NASA"
    } else if host.contains("statnews.com") {
        "STAT"
    } else if host.contains("variety.com") {
        "Variety"
    } else if host.contains("pitchfork.com") {
        "Pitchfork"
    } else if host.contains("bloomberg.com") {
        "Bloomberg"
    } else if host.contains("cnbc.com") {
        "CNBC"
    } else if host.contains("wsj.com") || host.contains("dj.com") {
        "The Wall Street Journal"
    } else if host.contains("thehill.com") {
        "The Hill"
    } else {
        ""
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let feed = feed.trim();
    let feed = feed.split_once('|').map(|(_, rest)| rest.trim()).unwrap_or(feed);
    let feed = feed.split_once(" - ").map(|(head, _)| head.trim()).unwrap_or(feed);
    let feed = feed.strip_prefix("RSS:").map(str::trim).unwrap_or(feed);
    if feed.is_empty() { "News".into() } else { feed.to_string() }
}

pub fn when(published_at: Option<i64>) -> String {
    let Some(ts) = published_at else { return String::new() };
    let now = glib::DateTime::now_local().ok().map(|d| d.to_unix()).unwrap_or(0);
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        return "Just now".to_string();
    }
    if diff < 3600 {
        let mins = diff / 60;
        return format!("{mins}m ago");
    }
    if diff < 86400 {
        let hours = diff / 3600;
        return format!("{hours}h ago");
    }
    if diff < 172800 {
        return "Yesterday".to_string();
    }
    glib::DateTime::from_unix_local(ts)
        .ok()
        .and_then(|d| d.format("%b %-e").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn read_time(text: &str) -> String {
    let words = text.split_whitespace().count();
    let mins = (words / 200).max(1);
    format!("{mins} min read")
}

/// Today, as the front page names it ("Thursday 12 November").
pub fn today() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%A %-e %B").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Today".to_string())
}

pub fn now() -> i64 {
    glib::DateTime::now_local().ok().map(|d| d.to_unix()).unwrap_or(0)
}

/// When the last refresh came in ("Updated 4 min ago").
pub fn updated(at: i64) -> String {
    let diff = now().saturating_sub(at);
    if diff < 60 {
        "Updated just now".into()
    } else if diff < 3600 {
        format!("Updated {} min ago", diff / 60)
    } else if diff < 86400 {
        format!("Updated {} h ago", diff / 3600)
    } else {
        format!("Updated {}", when(Some(at)).to_lowercase())
    }
}

/// The feeds you follow, gathered by publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub name: String,
    pub feeds: Vec<String>,
    /// Unread stories from the last day.
    pub unread: i64,
}

/// Publications with the most unread stories first.
pub fn publications(feeds: &[nyx_news_core::FeedRow], recent: &std::collections::HashMap<String, i64>) -> Vec<Publication> {
    let mut out: Vec<Publication> = Vec::new();
    for feed in feeds {
        let name = name(&feed.url, &feed.title);
        let unread = recent.get(&feed.id).copied().unwrap_or(0);
        match out.iter_mut().find(|p| p.name == name) {
            Some(publication) => {
                publication.feeds.push(feed.id.clone());
                publication.unread += unread;
            }
            None => out.push(Publication { name, feeds: vec![feed.id.clone()], unread }),
        }
    }
    out.sort_by(|a, b| b.unread.cmp(&a.unread).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_comes_from_the_address() {
        assert_eq!(name("https://www.nytimes.com/2026/10/18/world/story.html", "NYT > World News"), "The New York Times");
        assert_eq!(name("https://www.bbc.co.uk/news/articles/abc", "BBC News"), "BBC");
        assert_eq!(name("https://www.theguardian.com/world/story", "World news | The Guardian"), "The Guardian");
    }

    #[test]
    fn feeds_gather_by_publication() {
        let feed = |id: &str, url: &str| nyx_news_core::FeedRow {
            id: id.into(),
            url: url.into(),
            title: String::new(),
            site_url: String::new(),
            folder_id: None,
            position: 0,
            poll_minutes: 60,
            last_ok_at: None,
            last_error: None,
            consecutive_failures: 0,
            disabled: false,
            unread: 0,
        };
        let feeds = [
            feed("a", "https://feeds.bbci.co.uk/news/world/rss.xml"),
            feed("b", "https://www.wired.com/feed/rss"),
            feed("c", "https://feeds.bbci.co.uk/news/politics/rss.xml"),
        ];
        let recent = [("b".to_string(), 2), ("c".to_string(), 3)].into_iter().collect();
        let list = publications(&feeds, &recent);
        assert_eq!(list[0], Publication { name: "BBC".into(), feeds: vec!["a".into(), "c".into()], unread: 3 });
        assert_eq!(list[1].name, "Wired");
    }
}
