//! What tells stories apart at a glance: each topic's colour and dot, the kicker over
//! a headline, initials for a publication, and the icons News brings itself.

use crate::source;
use crate::widgets;
use crate::window;
use gtk::prelude::*;
use nyx_news_core::ArticleListItem;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static TOPIC_OF_FEED: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Icons the design needs that Adwaita doesn't have, written to the cache and added to
/// the icon theme. GTK fills every path of a symbolic icon, so lines are filled outlines.
const ICONS: &[(&str, &str)] = &[
    (
        "nyx-news-newspaper-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="#000"><path fill-rule="evenodd" d="M3 4h14v4h4v10a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3zM4.8 5.8V18A1.2 1.2 0 0 0 6 19.2h10.4a3 3 0 0 1-.2-1.2V5.8zM18 9.8V18a.8.8 0 0 0 1.6 0V9.8z"/><path d="M7 8h7v1.8H7zM7 11.6h7v1.6H7zM7 15h5v1.6H7z"/></svg>"##,
    ),
    (
        "nyx-news-compass-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="#000"><path fill-rule="evenodd" d="M12 2a10 10 0 1 1 0 20a10 10 0 1 1 0-20zM12 3.8a8.2 8.2 0 1 0 0 16.4a8.2 8.2 0 1 0 0-16.4z"/><path fill-rule="evenodd" d="M16.2 7.8l-2.4 6-6 2.4 2.4-6zM12 10.6a1.4 1.4 0 1 0 0 2.8a1.4 1.4 0 1 0 0-2.8z"/></svg>"##,
    ),
    (
        "nyx-news-sparkle-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="#000"><path d="M10 2.5c.7 4.6 2.9 6.8 7.5 7.5-4.6.7-6.8 2.9-7.5 7.5-.7-4.6-2.9-6.8-7.5-7.5 4.6-.7 6.8-2.9 7.5-7.5z"/><path d="M18 13.5c.4 2.4 1.6 3.6 4 4-2.4.4-3.6 1.6-4 4-.4-2.4-1.6-3.6-4-4 2.4-.4 3.6-1.6 4-4z"/></svg>"##,
    ),
    (
        "nyx-news-textsize-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="#000"><path fill-rule="evenodd" d="M6.6 5h1.8l5 14h-1.9l-1.3-3.6H4.8L3.5 19H1.6zM5.4 13.7h4.2L7.5 7.8z"/><path fill-rule="evenodd" d="M16.6 10h1.5l3.9 9h-1.8l-.9-2.2h-3.9l-.9 2.2h-1.8zM16 15.3h2.7l-1.35-3.4z"/></svg>"##,
    ),
    (
        "nyx-news-external-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="#000"><path d="M14 3.2h6.8V10h-1.6V5.93l-7.63 7.64-1.14-1.14 7.64-7.63H14z"/><path d="M5.5 5.2H10v1.6H5.5a.7.7 0 0 0-.7.7v11c0 .39.31.7.7.7h11a.7.7 0 0 0 .7-.7V14h1.6v4.5a2.3 2.3 0 0 1-2.3 2.3h-11a2.3 2.3 0 0 1-2.3-2.3v-11a2.3 2.3 0 0 1 2.3-2.3z"/></svg>"##,
    ),
];

pub fn install_icons() {
    let dir = crate::paths::cache_dir().join("icons");
    for (name, svg) in ICONS {
        let file = dir.join(format!("{name}.svg"));
        if std::fs::read_to_string(&file).ok().as_deref() != Some(*svg) {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&file, svg);
        }
    }
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

/// The theme swatch a topic is drawn in (`swatch-blue`).
pub fn topic_swatch(name: &str) -> String {
    let swatch = match name {
        "World" => "blue",
        "National" => "red",
        "Politics" => "violet",
        "Business" => "amber",
        "Technology" => "teal",
        "Science" => "green",
        "Health" => "pink",
        "Culture" => "coral",
        "Sports" => "graphite",
        other => {
            const ORDER: [&str; 8] = ["blue", "violet", "green", "pink", "coral", "teal", "red", "amber"];
            ORDER[other.bytes().map(usize::from).sum::<usize>() % ORDER.len()]
        }
    };
    format!("swatch-{swatch}")
}

/// A round dot in the topic's colour.
pub fn dot(topic: &str) -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("topic-dot");
    dot.add_css_class(&format!("section-{}", topic_swatch(topic)));
    dot.set_valign(gtk::Align::Center);
    dot.set_halign(gtk::Align::Center);
    dot
}

/// Remembers which topic each feed belongs to, for kickers and counts.
pub fn learn_topics() {
    let sections = window::with_db(|db| db.feed_sections()).unwrap_or_default();
    TOPIC_OF_FEED.with(|map| *map.borrow_mut() = sections.into_iter().collect());
}

pub fn topic_of(feed_id: &str) -> Option<String> {
    TOPIC_OF_FEED.with(|map| map.borrow().get(feed_id).cloned())
}

/// The line over a headline: its topic in the topic's colour, then where and when.
pub fn kicker(article: &ArticleListItem) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("kicker");
    if let Some(topic) = topic_of(&article.feed_id) {
        let name = widgets::label(&topic, "kicker-topic");
        name.add_css_class(&format!("section-{}", topic_swatch(&topic)));
        row.append(&name);
    }
    let line = widgets::label(&source::byline(&article.original_url, &article.feed_title, article.published_at), "kicker-source");
    line.set_ellipsize(gtk::pango::EllipsizeMode::End);
    line.set_hexpand(true);
    row.append(&line);
    row
}

/// One or two letters for a publication ("The Meridian" is "M", "Field Notes" "FN").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().filter(|w| !w.eq_ignore_ascii_case("the")).collect();
    let letters: String = match words.as_slice() {
        [] => "N".into(),
        [one] => one.chars().take(1).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// A round badge with a publication's initials, in a colour of its own.
pub fn avatar(name: &str, size: i32) -> gtk::Box {
    let avatar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    avatar.add_css_class("pub-avatar");
    avatar.add_css_class(&format!("section-{}", topic_swatch(name)));
    avatar.set_size_request(size, size);
    avatar.set_valign(gtk::Align::Center);
    avatar.set_halign(gtk::Align::Start);
    avatar.set_hexpand(false);
    let letters = widgets::label(&initials(name), "pub-initials");
    letters.set_halign(gtk::Align::Center);
    letters.set_hexpand(true);
    avatar.append(&letters);
    avatar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_skip_the() {
        assert_eq!(initials("The Meridian"), "M");
        assert_eq!(initials("Field Notes"), "FN");
        assert_eq!(initials("The New York Times"), "NY");
        assert_eq!(initials(""), "N");
    }

    #[test]
    fn known_topics_keep_their_colours() {
        assert_eq!(topic_swatch("World"), "swatch-blue");
        assert_eq!(topic_swatch("Culture"), "swatch-coral");
        assert!(topic_swatch("Gardening").starts_with("swatch-"));
    }
}
