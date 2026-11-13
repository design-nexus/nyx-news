//! Channels and topics: follow the topics on your front page and the publications in
//! the bundled directory, add a feed or website by its address, and import or export
//! your subscriptions.

use crate::dialogs;
use crate::look;
use crate::prefs;
use crate::source;
use crate::timeline;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::prelude::*;
use nyx_ui::clamp::Clamp;
use std::cell::RefCell;

thread_local! {
    static FOUND: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

/// A publication from the directory: its feeds and the topics they file under.
struct Listing {
    name: String,
    feeds: Vec<(String, String)>,
}

fn directory_listings() -> Vec<Listing> {
    let mut out: Vec<Listing> = Vec::new();
    for source in nyx_news_core::directory_sources() {
        let name = source::name(&source.url, &source.title);
        match out.iter_mut().find(|l| l.name == name) {
            Some(listing) => listing.feeds.push((source.url, source.section)),
            None => out.push(Listing { name, feeds: vec![(source.url, source.section)] }),
        }
    }
    out
}

fn topics_line(listing: &Listing) -> String {
    let mut topics: Vec<&str> = Vec::new();
    for (_, topic) in &listing.feeds {
        if !topics.contains(&topic.as_str()) {
            topics.push(topic);
        }
    }
    match topics.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn section_icon(name: &str) -> &'static str {
    match name {
        "World" => "web-browser-symbolic",
        "National" => "emoji-flags-symbolic",
        "Politics" => "system-users-symbolic",
        "Business" => "x-office-spreadsheet-symbolic",
        "Technology" => "computer-symbolic",
        "Science" => "applications-science-symbolic",
        "Health" => "face-smile-symbolic",
        "Sports" => "applications-games-symbolic",
        "Culture" => "applications-graphics-symbolic",
        _ => "nyx-news-newspaper-symbolic",
    }
}

fn head(text: &str) -> gtk::Label {
    widgets::label(text, "shelf-title")
}

pub fn populate(host: &gtk::Box) {
    while let Some(child) = host.first_child() {
        host.remove(&child);
    }

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    bar.add_css_class("page-bar");
    bar.append(&widgets::label("Channels and topics", "page-title"));
    let sub = widgets::label("Follow what you want on your front page", "page-sub");
    sub.set_hexpand(true);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bar.append(&sub);
    let import = widgets::labeled_button("document-open-symbolic", "Import");
    import.add_css_class("flat");
    import.set_valign(gtk::Align::Center);
    import.set_tooltip_text(Some("Import subscriptions from an OPML file (Ctrl+I)"));
    import.connect_clicked(|_| {
        if let Some(win) = window::window() {
            dialogs::import_opml(&win);
        }
    });
    bar.append(&import);
    let export = widgets::labeled_button("document-save-symbolic", "Export");
    export.add_css_class("flat");
    export.set_valign(gtk::Align::Center);
    export.set_tooltip_text(Some("Export your subscriptions as OPML (Ctrl+E)"));
    export.connect_clicked(|_| {
        if let Some(win) = window::window() {
            dialogs::export_opml(&win);
        }
    });
    bar.append(&export);
    host.append(&bar);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.add_css_class("channels");

    body.append(&head("Topics"));
    let grid = gtk::FlowBox::new();
    grid.add_css_class("topic-grid");
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_homogeneous(true);
    grid.set_min_children_per_line(2);
    grid.set_max_children_per_line(4);
    grid.set_column_spacing(10);
    grid.set_row_spacing(10);
    let disabled = prefs::get().disabled_categories;
    for topic in nyx_news_core::directory().unwrap_or_default() {
        let name = topic.title.clone();
        let following = !disabled.contains(&name);
        let card = gtk::Button::new();
        card.add_css_class("topic-card");
        if following {
            card.add_css_class("on");
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("topic-badge");
        badge.add_css_class(&format!("section-{}", look::topic_swatch(&name)));
        badge.set_valign(gtk::Align::Center);
        badge.set_hexpand(false);
        let glyph = gtk::Image::from_icon_name(section_icon(&name));
        glyph.set_halign(gtk::Align::Center);
        glyph.set_hexpand(true);
        badge.append(&glyph);
        row.append(&badge);
        let title = widgets::label(&name, "topic-name");
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&title);
        let state = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        state.add_css_class(if following { "topic-following" } else { "topic-follow" });
        state.append(&gtk::Image::from_icon_name(if following { "object-select-symbolic" } else { "list-add-symbolic" }));
        state.append(&gtk::Label::new(Some(if following { "Following" } else { "Follow" })));
        row.append(&state);
        card.set_child(Some(&row));
        card.set_tooltip_text(Some(&if following {
            format!("Take {name} off your front page")
        } else {
            format!("Put {name} on your front page")
        }));
        card.connect_clicked(move |_| {
            timeline::set_topic_followed(&name, !following);
            window::refresh_lists();
            timeline::refresh_discover();
        });
        grid.insert(&card, -1);
    }
    body.append(&grid);

    body.append(&head("Publications"));
    let feeds = window::with_db(|db| db.feeds()).unwrap_or_default();
    let followed: Vec<source::Publication> = source::publications(&feeds, &Default::default());
    let listings = directory_listings();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("group-list");
    list.add_css_class("publications");
    for listing in &listings {
        let following = followed.iter().any(|p| p.name == listing.name);
        let name = listing.name.clone();
        let wanted = listing.feeds.clone();
        list.append(&publication_row(&listing.name, &topics_line(listing), following, move |follow| {
            if follow {
                for (url, topic) in &wanted {
                    worker::add_in_category(url.clone(), topic.clone());
                }
                window::toast(&format!("Following {name}"));
            } else {
                unfollow(&name);
            }
        }));
    }
    for publication in followed.iter().filter(|p| !listings.iter().any(|l| l.name == p.name)) {
        let site = feeds
            .iter()
            .find(|f| publication.feeds.first() == Some(&f.id))
            .map(|f| if f.site_url.is_empty() { f.url.clone() } else { f.site_url.clone() })
            .unwrap_or_default();
        let host = site.split("//").nth(1).unwrap_or(&site).split('/').next().unwrap_or("").trim_start_matches("www.").to_string();
        let name = publication.name.clone();
        list.append(&publication_row(&publication.name, &host, true, move |follow| {
            if !follow {
                unfollow(&name);
            }
        }));
    }
    body.append(&list);

    body.append(&head("Add a feed or a website"));
    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("A website or feed address, like arstechnica.com"));
    entry.set_icon_from_icon_name(gtk::EntryIconPosition::Primary, Some("insert-link-symbolic"));
    entry.set_hexpand(true);
    let find_btn = gtk::Button::with_label("Find");
    find_btn.add_css_class("suggested-action");
    let entry_for_find = entry.clone();
    find_btn.connect_clicked(move |_| find(&entry_for_find.text()));
    entry.connect_activate(|entry| find(&entry.text()));
    find_row.append(&entry);
    find_row.append(&find_btn);
    body.append(&find_row);

    let found = gtk::Box::new(gtk::Orientation::Vertical, 0);
    found.add_css_class("group-list");
    found.set_visible(false);
    body.append(&found);
    FOUND.with(|slot| *slot.borrow_mut() = Some(found));

    let note = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    note.add_css_class("channels-note");
    note.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    note.append(&widgets::label("RSS, Atom and JSON feeds. Private and local addresses aren't fetched.", ""));
    body.append(&note);

    let clamped = Clamp::new(&body, 860);
    clamped.set_start(true);
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&clamped)
        .build();
    host.append(&scroll);
}

fn find(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        window::toast("Enter a website or feed address.");
        return;
    }
    let url = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
    worker::find_feeds(url);
}

fn unfollow(name: &str) {
    let feeds = window::with_db(|db| db.feeds()).unwrap_or_default();
    let _ = window::with_db(|db| {
        for feed in feeds.iter().filter(|f| source::name(&f.url, &f.title) == name) {
            db.delete_feed(&feed.id)?;
        }
        Ok(())
    });
    window::toast(&format!("Unfollowed {name}"));
    window::refresh_lists();
    timeline::refresh_discover();
}

fn publication_row(name: &str, about: &str, following: bool, act: impl Fn(bool) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("settings-option");
    row.add_css_class("publication");
    row.append(&look::avatar(name, 32));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = widgets::label(name, "settings-option-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    if !about.is_empty() {
        let desc = widgets::label(about, "settings-option-description");
        desc.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&desc);
    }
    row.append(&text);
    let button = widgets::labeled_button(if following { "object-select-symbolic" } else { "list-add-symbolic" }, if following { "Following" } else { "Follow" });
    if !following {
        button.add_css_class("suggested-action");
    }
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(&if following { format!("Stop following {name}") } else { format!("Follow {name}") }));
    button.connect_clicked(move |button| {
        act(!following);
        if !following {
            button.set_sensitive(false);
        }
    });
    row.append(&button);
    row
}

pub fn show_found(feeds: Vec<nyx_news_core::DiscoveredFeed>) {
    let Some(box_) = FOUND.with(|slot| slot.borrow().clone()) else { return };
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
    box_.set_visible(true);
    if feeds.is_empty() {
        let note = widgets::label("No feeds found at that address.", "dim");
        note.set_margin_top(12);
        note.set_margin_bottom(12);
        note.set_margin_start(16);
        box_.append(&note);
        return;
    }
    for feed in feeds {
        let name = source::name(&feed.url, &feed.title);
        let url = feed.url.clone();
        let host = url.split("//").nth(1).unwrap_or(&url).split('/').next().unwrap_or("").trim_start_matches("www.").to_string();
        let label = name.clone();
        box_.append(&publication_row(&name, &host, false, move |_| {
            worker::add_url(url.clone());
            window::toast(&format!("Following {label}"));
        }));
    }
}
