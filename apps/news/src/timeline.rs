//! The pages under the page bar. Top stories and each topic are a front page: a
//! briefing, a lead story with its photo beside it, then cards and headlines. Every
//! other page is a list whose rows a factory rebinds as the user scrolls.

use crate::discover_page;
use crate::look;
use crate::paths;
use crate::prefs;
use crate::reader;
use crate::source;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nyx_news_core::{AiFailure, ArticleListItem, Edition, Excerpt, Point, View, lay_paper};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

thread_local! {
    static STACK: RefCell<Option<gtk::Stack>> = const { RefCell::new(None) };
    static STORE: RefCell<Option<gio::ListStore>> = const { RefCell::new(None) };
    static LIST_PAGE: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static PAPER: RefCell<Option<gtk::ScrolledWindow>> = const { RefCell::new(None) };
    static PAPER_BODY: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static SECTION: RefCell<String> = const { RefCell::new(String::new()) };
    static SELECTION: RefCell<Option<gtk::SingleSelection>> = const { RefCell::new(None) };
    static LIST: RefCell<Option<gtk::ListView>> = const { RefCell::new(None) };
    static EMPTY: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static EMPTY_TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static EMPTY_DESC: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static PAGE_TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static PAGE_META: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static LAYOUT: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static SYNCING_LAYOUT: Cell<bool> = const { Cell::new(false) };
    static FOLLOW_BTN: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    static DAILY_BRIEF: RefCell<Option<Vec<Point>>> = const { RefCell::new(None) };
    static DAILY_BRIEF_RUNNING: Cell<bool> = const { Cell::new(false) };
    static EDITION: RefCell<Vec<Edition>> = const { RefCell::new(Vec::new()) };
    static FRONT_COUNT: Cell<usize> = const { Cell::new(0) };
    static DISCOVER: Cell<bool> = const { Cell::new(false) };
    static DISCOVER_HOST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

pub fn build() -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&page_bar());
    let store = gio::ListStore::new::<gtk::StringObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    selection.set_autoselect(false);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        outer.add_css_class("article-row");
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("unread-dot");
        dot.set_valign(gtk::Align::Start);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let kicker = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        kicker.add_css_class("kicker");
        let topic = widgets::label("", "kicker-topic");
        let source = widgets::label("", "kicker-source");
        source.set_ellipsize(gtk::pango::EllipsizeMode::End);
        kicker.append(&topic);
        kicker.append(&source);
        let title = gtk::Label::new(None);
        title.add_css_class("article-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let excerpt = gtk::Label::new(None);
        excerpt.add_css_class("article-excerpt");
        excerpt.set_xalign(0.0);
        excerpt.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&kicker);
        text.append(&title);
        text.append(&excerpt);
        outer.append(&dot);
        outer.append(&text);
        item.set_child(Some(&outer));
    });
    factory.connect_bind(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(id) = item.item().and_downcast::<gtk::StringObject>() else { return };
        let Some(article) = window::find_article(&id.string()) else { return };
        let Some(outer) = item.child().and_downcast::<gtk::Box>() else { return };
        let compact = prefs::get().layout == "compact";
        if compact {
            outer.add_css_class("compact");
        } else {
            outer.remove_css_class("compact");
        }
        let Some(dot) = outer.first_child().and_downcast::<gtk::Box>() else { return };
        let Some(text) = dot.next_sibling().and_downcast::<gtk::Box>() else { return };
        let Some(kicker) = text.first_child().and_downcast::<gtk::Box>() else { return };
        let Some(title) = kicker.next_sibling().and_downcast::<gtk::Label>() else { return };
        let Some(excerpt) = title.next_sibling().and_downcast::<gtk::Label>() else { return };
        let Some(topic) = kicker.first_child().and_downcast::<gtk::Label>() else { return };
        let Some(source_line) = topic.next_sibling().and_downcast::<gtk::Label>() else { return };

        dot.set_valign(if compact { gtk::Align::Center } else { gtk::Align::Start });
        if article.read {
            dot.remove_css_class("active");
            title.remove_css_class("unread");
        } else {
            dot.add_css_class("active");
            title.add_css_class("unread");
        }
        for class in topic.css_classes() {
            if class.starts_with("section-") {
                topic.remove_css_class(&class);
            }
        }
        match look::topic_of(&article.feed_id) {
            Some(name) => {
                topic.add_css_class(&format!("section-{}", look::topic_swatch(&name)));
                topic.set_text(&name);
                topic.set_visible(true);
            }
            None => topic.set_visible(false),
        }
        let mut line = source::byline(&article.original_url, &article.feed_title, article.published_at);
        if article.starred {
            line.push_str(" · Starred");
        }
        if article.saved {
            line.push_str(" · Saved");
        }
        if !article.why.is_empty() {
            line = format!("{line} · {}", article.why);
        }
        source_line.set_text(&line);
        kicker.set_visible(!compact);
        title.set_text(headline(&article.title));
        excerpt.set_text(&article.summary);
        excerpt.set_visible(!compact && !article.summary.is_empty());
    });
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.set_single_click_activate(true);
    list.connect_activate(move |list, pos| {
        let Some(model) = list.model() else { return };
        let Some(id) = model.item(pos).and_downcast::<gtk::StringObject>() else { return };
        window::select_id(Some(id.string().to_string()));
        if let Some(article) = window::find_article(&id.string()) {
            reader::show(&article);
        }
    });
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&list)
        .hexpand(true)
        .vexpand(true)
        .build();

    let paper_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    paper_body.add_css_class("front-page");
    paper_body.set_hexpand(true);
    let paper = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&paper_body)
        .hexpand(true)
        .vexpand(true)
        .build();
    paper.set_visible(false);

    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
    empty.add_css_class("empty-state");
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_can_target(false);
    let empty_icon = gtk::Image::from_icon_name("nyx-news-newspaper-symbolic");
    empty_icon.set_pixel_size(48);
    empty_icon.add_css_class("dim");
    let empty_title = widgets::label("No stories", "empty-title");
    empty_title.set_halign(gtk::Align::Center);
    let empty_desc = widgets::label("Follow topics or publications to start reading.", "dim");
    empty_desc.set_halign(gtk::Align::Center);
    empty_desc.set_justify(gtk::Justification::Center);
    empty_desc.set_wrap(true);
    empty_desc.set_max_width_chars(52);
    empty.append(&empty_icon);
    empty.append(&empty_title);
    empty.append(&empty_desc);

    let page = gtk::Overlay::new();
    page.set_child(Some(&scroll));
    page.add_overlay(&empty);
    root.append(&page);
    root.append(&paper);
    stack.add_named(&root, Some("list"));
    stack.add_named(&reader::build(), Some("story"));

    let discover = gtk::Box::new(gtk::Orientation::Vertical, 0);
    discover.set_hexpand(true);
    discover.set_vexpand(true);
    DISCOVER_HOST.with(|host| *host.borrow_mut() = Some(discover.clone()));
    stack.add_named(&discover, Some("discover"));

    STACK.with(|s| *s.borrow_mut() = Some(stack.clone()));
    STORE.with(|s| *s.borrow_mut() = Some(store));
    SELECTION.with(|s| *s.borrow_mut() = Some(selection));
    LIST.with(|s| *s.borrow_mut() = Some(list));
    LIST_PAGE.with(|s| *s.borrow_mut() = Some(page));
    PAPER.with(|s| *s.borrow_mut() = Some(paper));
    PAPER_BODY.with(|s| *s.borrow_mut() = Some(paper_body));
    EMPTY.with(|s| *s.borrow_mut() = Some(empty));
    EMPTY_TITLE.with(|s| *s.borrow_mut() = Some(empty_title));
    EMPTY_DESC.with(|s| *s.borrow_mut() = Some(empty_desc));
    stack
}

/// The page's name and count, Follow for a topic or publication, Cards or Headlines,
/// and Refresh.
fn page_bar() -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    bar.add_css_class("page-bar");
    let title = widgets::label("Top stories", "page-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = widgets::label("", "page-sub");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_hexpand(true);
    bar.append(&title);
    bar.append(&meta);

    let follow = gtk::Button::new();
    follow.add_css_class("follow-button");
    follow.set_valign(gtk::Align::Center);
    follow.set_visible(false);
    follow.connect_clicked(|_| follow_clicked());
    bar.append(&follow);

    let layout = widgets::segmented(&widgets::opts(&[("list", "Cards"), ("compact", "Headlines")]), &prefs::get().layout, |layout| {
        if SYNCING_LAYOUT.with(|s| s.get()) {
            return;
        }
        prefs::update(|p| p.layout = layout);
        window::reload();
    });
    layout.set_tooltip_text(Some("Show stories as cards with photos, or as a list of headlines"));
    bar.append(&layout);

    let refresh = widgets::bar_button("view-refresh-symbolic", "Refresh (R)");
    refresh.set_valign(gtk::Align::Center);
    refresh.connect_clicked(|_| {
        window::toast("Refreshing news…");
        worker::refresh_current();
    });
    bar.append(&refresh);

    PAGE_TITLE.with(|s| *s.borrow_mut() = Some(title));
    PAGE_META.with(|s| *s.borrow_mut() = Some(meta));
    LAYOUT.with(|s| *s.borrow_mut() = Some(layout));
    FOLLOW_BTN.with(|s| *s.borrow_mut() = Some(follow));
    bar
}

fn follow_clicked() {
    let sec = section();
    if !sec.is_empty() {
        let shown = !prefs::get().disabled_categories.contains(&sec);
        set_topic_followed(&sec, !shown);
        if shown {
            show_section("");
            window::navigate(&View::All);
        } else {
            window::refresh_lists();
        }
        return;
    }
    let name = window::current_page_name();
    match window::view() {
        View::Topic(_) => {
            let _ = window::with_db(|db| db.unfollow_topic(&name));
        }
        View::Feed(id) => {
            let _ = window::with_db(|db| db.delete_feed(&id));
        }
        View::Feeds(ids) => {
            let _ = window::with_db(|db| {
                for id in &ids {
                    db.delete_feed(id)?;
                }
                Ok(())
            });
        }
        _ => return,
    }
    window::toast(&format!("Unfollowed {name}"));
    show_section("");
    window::navigate(&View::All);
}

/// A topic is followed when it's on the front page and in the sidebar.
pub fn set_topic_followed(name: &str, follow: bool) {
    prefs::update(|p| {
        if follow {
            p.disabled_categories.retain(|c| c != name);
        } else if !p.disabled_categories.iter().any(|c| c == name) {
            p.disabled_categories.push(name.to_string());
        }
    });
    window::toast(&if follow { format!("Following {name}") } else { format!("{name} is off your front page") });
}

#[allow(dead_code)]
pub fn mark_current_read() {
    let ids: Vec<String> = window::articles().into_iter().filter(|a| !a.read).map(|a| a.id).collect();
    if ids.is_empty() {
        return;
    }
    let count = ids.len();
    let _ = window::with_db(|db| {
        for id in &ids {
            db.set_read(id, true)?;
        }
        Ok(())
    });
    window::refresh_lists();
    window::toast(&if count == 1 { "1 story marked read.".into() } else { format!("{count} stories marked read.") });
}

fn stories(count: usize) -> String {
    if count == 1 { "1 story".into() } else { format!("{count} stories") }
}

fn update_page_bar(count: usize) {
    let searching = window::search_active();
    let front = window::view() == View::All && section().is_empty() && !searching;
    let (title, meta) = if searching {
        ("Search".to_string(), format!("“{}” · {}", window::search_query(), stories(count)))
    } else if front {
        ("Top stories".to_string(), format!("{} · {}", source::today(), stories(count)))
    } else {
        (window::current_page_name(), stories(count))
    };
    if let Some(label) = PAGE_TITLE.with(|s| s.borrow().clone()) {
        label.set_text(&title);
    }
    if let Some(label) = PAGE_META.with(|s| s.borrow().clone()) {
        label.set_text(&meta);
    }
    if let Some(layout) = LAYOUT.with(|s| s.borrow().clone()) {
        let current = if prefs::get().layout == "compact" { 1 } else { 0 };
        SYNCING_LAYOUT.with(|s| s.set(true));
        let mut child = layout.first_child();
        let mut index = 0;
        while let Some(button) = child {
            if let Some(toggle) = button.downcast_ref::<gtk::ToggleButton>()
                && index == current
                && !toggle.is_active()
            {
                toggle.set_active(true);
            }
            index += 1;
            child = button.next_sibling();
        }
        SYNCING_LAYOUT.with(|s| s.set(false));
    }
    if let Some(btn) = FOLLOW_BTN.with(|s| s.borrow().clone()) {
        let sec = section();
        let face = |icon: &str, text: &str| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&gtk::Label::new(Some(text)));
            row
        };
        if !sec.is_empty() && !searching {
            btn.set_visible(true);
            btn.set_child(Some(&face("object-select-symbolic", "Following")));
            btn.add_css_class("following");
            btn.set_tooltip_text(Some(&format!("Take {sec} off your front page")));
        } else if matches!(window::view(), View::Topic(_) | View::Feed(_) | View::Feeds(_)) && !searching {
            btn.set_visible(true);
            btn.set_child(Some(&face("object-select-symbolic", "Following")));
            btn.add_css_class("following");
            btn.set_tooltip_text(Some("Stop following"));
        } else {
            btn.set_visible(false);
        }
    }
}

pub fn show_story() {
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("story");
    }
}

pub fn close_story() {
    let Some(stack) = STACK.with(|s| s.borrow().clone()) else { return };
    if stack.visible_child_name().as_deref() == Some("story") {
        stack.set_visible_child_name("list");
    }
}

fn unique(articles: Vec<ArticleListItem>) -> Vec<ArticleListItem> {
    let mut seen_urls = HashSet::new();
    let mut seen_titles: Vec<(String, Option<i64>)> = Vec::new();
    let mut out = Vec::new();
    for article in articles {
        let link = if !article.url.is_empty() { &article.url } else { &article.original_url };
        let clean = nyx_news_core::canonical_url(link).unwrap_or_else(|_| link.clone());
        if !clean.is_empty() && seen_urls.contains(&clean) {
            continue;
        }
        if !article.title.trim().is_empty()
            && seen_titles.iter().any(|(t, when)| nyx_news_core::same_story(&article.title, article.published_at, t, *when))
        {
            continue;
        }
        if !clean.is_empty() {
            seen_urls.insert(clean);
        }
        seen_titles.push((article.title.clone(), article.published_at));
        out.push(article);
    }
    out
}

pub fn bind_articles() {
    let headlines = window::view() == View::All && !discover_visible() && !window::search_active();
    if let Some(paper) = PAPER.with(|s| s.borrow().clone()) {
        paper.set_visible(headlines);
    }
    if let Some(page) = LIST_PAGE.with(|s| s.borrow().clone()) {
        page.set_visible(!headlines);
    }
    if headlines {
        let count = fill_paper();
        update_page_bar(count);
        return;
    }
    let Some(store) = STORE.with(|s| s.borrow().clone()) else { return };
    store.remove_all();
    for article in unique(window::articles()) {
        store.append(&gtk::StringObject::new(&article.id));
    }
    update_page_bar(store.n_items() as usize);
    if let Some(empty) = EMPTY.with(|s| s.borrow().clone()) {
        let feeds = window::with_db(|db| db.feeds()).map(|f| f.len()).unwrap_or(0);
        empty.set_visible(store.n_items() == 0);
        let (title, desc) = if window::search_active() {
            ("No matching stories", "Try other words, or press Esc to clear the search.")
        } else if window::view() == View::ForYou {
            ("Nothing in For you yet", "Save stories or follow topics, and For you picks more like them.")
        } else if feeds == 0 {
            ("No publications followed", "Open Channels and topics in the sidebar to follow some.")
        } else if window::view() == View::Unread {
            ("You're caught up", "No unread stories right now.")
        } else if window::view() == View::Starred {
            ("No starred stories", "Star stories to find them here.")
        } else if window::view() == View::Saved {
            ("No saved stories", "Save a story with B or its Save button to read it later.")
        } else {
            ("No stories right now", "Press R to check for new ones.")
        };
        if let Some(t) = EMPTY_TITLE.with(|s| s.borrow().clone()) {
            t.set_text(title);
        }
        if let Some(d) = EMPTY_DESC.with(|s| s.borrow().clone()) {
            d.set_text(desc);
        }
    }
}

pub fn show_section(name: &str) {
    SECTION.with(|section| *section.borrow_mut() = name.to_string());
}

pub fn section() -> String {
    SECTION.with(|section| section.borrow().clone())
}

/// Stories on the front page the last time it was laid out.
pub fn front_count() -> usize {
    FRONT_COUNT.with(|c| c.get())
}

pub fn apply_daily_brief(result: Result<String, AiFailure>) {
    DAILY_BRIEF_RUNNING.with(|r| r.set(false));
    let lines = result.map(|text| nyx_news_core::points(&text)).unwrap_or_default();
    DAILY_BRIEF.with(|b| *b.borrow_mut() = Some(lines));
    if window::view() == View::All && section().is_empty() {
        bind_articles();
    }
}

fn ensure_daily_brief(leads: &[ArticleListItem]) {
    if !prefs::get().summaries || leads.is_empty() {
        return;
    }
    if DAILY_BRIEF.with(|b| b.borrow().is_some()) || DAILY_BRIEF_RUNNING.with(|r| r.get()) {
        return;
    }
    DAILY_BRIEF_RUNNING.with(|r| r.set(true));
    let excerpts: Vec<Excerpt> = leads
        .iter()
        .take(6)
        .map(|a| Excerpt {
            title: a.title.clone(),
            feed: a.feed_title.clone(),
            url: a.original_url.clone(),
            text: if a.summary.is_empty() { a.title.clone() } else { format!("{}: {}", a.title, a.summary) },
        })
        .collect();
    worker::brief("daily".to_string(), excerpts, 9999);
}

/// The briefing's lines: Assistant's when it wrote some, otherwise each topic's lead.
fn briefing_points(edition: &[Edition]) -> Vec<Point> {
    let written = DAILY_BRIEF.with(|b| b.borrow().clone()).unwrap_or_default();
    if !written.is_empty() {
        return written;
    }
    edition
        .iter()
        .filter_map(|section| section.stories.first().map(|lead| (section, lead)))
        .map(|(section, lead)| Point { label: Some(section.name.clone()), text: headline(&lead.title).to_string() })
        .collect()
}

fn point_row(point: &Point) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let bullet = widgets::label("•", "briefing-bullet");
    bullet.set_valign(gtk::Align::Start);
    row.append(&bullet);
    let text = glib::markup_escape_text(&point.text);
    let markup = match &point.label {
        Some(label) => format!("<b>{}:</b> <span alpha=\"75%\">{text}</span>", glib::markup_escape_text(label)),
        None => format!("<span alpha=\"75%\">{text}</span>"),
    };
    let line = gtk::Label::new(None);
    line.set_markup(&markup);
    line.add_css_class("briefing-line");
    line.set_xalign(0.0);
    line.set_wrap(true);
    line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    line.set_hexpand(true);
    row.append(&line);
    row
}

fn spoken(points: &[Point]) -> String {
    points
        .iter()
        .map(|p| match &p.label {
            Some(label) => format!("{label}: {}", p.text),
            None => p.text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn listen_button(points: Vec<Point>) -> gtk::Button {
    let listen = widgets::labeled_button("audio-volume-high-symbolic", "Listen");
    listen.add_css_class("flat");
    listen.add_css_class("briefing-action");
    listen.set_tooltip_text(Some("Have Assistant read the briefing aloud"));
    listen.connect_clicked(move |_| {
        window::toast("Reading your briefing aloud…");
        worker::speak(spoken(&points));
    });
    listen
}

fn briefing_card(edition: &[Edition]) -> gtk::Box {
    let leads: Vec<ArticleListItem> = edition.iter().filter_map(|s| s.stories.first().cloned()).collect();
    ensure_daily_brief(&leads);
    let points = briefing_points(edition);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("briefing");

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = gtk::Image::from_icon_name("nyx-news-sparkle-symbolic");
    icon.add_css_class("briefing-icon");
    head.append(&icon);
    head.append(&widgets::label("Your briefing", "briefing-title"));
    let running = DAILY_BRIEF_RUNNING.with(|r| r.get());
    let sub = widgets::label(
        if running { "Writing your briefing…" } else { "Five minutes, from today's top stories" },
        "briefing-sub",
    );
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    sub.set_visible(!window::is_narrow());
    head.append(&sub);
    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    head.append(&gap);
    if prefs::get().summaries {
        head.append(&listen_button(points.iter().take(3).cloned().collect()));
    }
    let all = gtk::Button::with_label("Read it all");
    all.add_css_class("briefing-action");
    all.add_css_class("small");
    all.connect_clicked(|_| read_briefing());
    head.append(&all);
    card.append(&head);

    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    lines.add_css_class("briefing-lines");
    for point in points.iter().take(3) {
        lines.append(&point_row(point));
    }
    card.append(&lines);
    card
}

/// The whole briefing, then the lead story of each topic with its summary.
fn read_briefing() {
    let edition = EDITION.with(|e| e.borrow().clone());
    let points = briefing_points(&edition);
    let (dialog, card) = widgets::dialog("Your briefing", 600);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.add_css_class("briefing-full");
    for point in &points {
        body.append(&point_row(point));
    }
    let heading = widgets::label("From each topic", "shelf-title");
    heading.set_margin_top(12);
    body.append(&heading);
    for section in &edition {
        let Some(lead) = section.stories.first().cloned() else { continue };
        let button = gtk::Button::new();
        button.add_css_class("briefing-story");
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.append(&look::kicker(&lead));
        let title = widgets::label(headline(&lead.title), "card-headline");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        column.append(&title);
        if !lead.summary.is_empty() {
            let summary = widgets::label(&lead.summary, "card-summary");
            summary.set_wrap(true);
            summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            summary.set_lines(3);
            summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
            column.append(&summary);
        }
        button.set_child(Some(&column));
        let d = dialog.clone();
        button.connect_clicked(move |_| {
            d.close();
            reader::show(&lead);
        });
        body.append(&button);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(520)
        .child(&body)
        .build();
    card.append(&scroll);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    if prefs::get().summaries {
        buttons.append(&listen_button(points));
    }
    let close = gtk::Button::with_label("Close");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    buttons.append(&close);
    card.append(&buttons);
    dialog.present();
}

/// Lays out the front page or a topic's page; returns how many stories it holds.
fn fill_paper() -> usize {
    let Some(body) = PAPER_BODY.with(|s| s.borrow().clone()) else { return 0 };
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    let wanted = section();
    let articles = window::articles();
    let edition = if !wanted.is_empty() {
        let stories = unique(articles);
        if stories.is_empty() { Vec::new() } else { vec![Edition { name: wanted.clone(), stories }] }
    } else {
        let sections = window::with_db(|db| db.feed_sections()).unwrap_or_default();
        lay_paper(&articles, &sections)
    };
    let disabled = prefs::get().disabled_categories;
    let edition: Vec<_> = if wanted.is_empty() {
        edition.into_iter().filter(|s| !disabled.iter().any(|d| d.eq_ignore_ascii_case(&s.name))).collect()
    } else {
        edition
    };
    let count = edition.iter().map(|s| s.stories.len()).sum();
    if wanted.is_empty() {
        FRONT_COUNT.with(|c| c.set(count));
        EDITION.with(|e| *e.borrow_mut() = edition.clone());
    }

    if edition.is_empty() {
        let feeds = window::with_db(|db| db.feeds()).map(|f| f.len()).unwrap_or(0);
        let name = if wanted.is_empty() { "Top stories".into() } else { wanted };
        let empty = widgets::empty_state(
            "nyx-news-newspaper-symbolic",
            &if feeds == 0 { "Welcome to News".into() } else { format!("No stories in {name} yet") },
            if feeds == 0 {
                "Follow topics and publications in Channels and topics to build your front page."
            } else {
                "Fetching the latest stories. They show here as they come in."
            },
            if feeds == 0 { Some(("Open Channels and topics", Box::new(window::show_discover))) } else { None },
        );
        body.append(&empty);
        return 0;
    }
    let mut missing_images: Vec<(String, String)> = Vec::new();
    for section in &edition {
        for story in &section.stories {
            if story.image_url.is_none()
                && (story.original_url.starts_with("https://") || story.original_url.starts_with("http://"))
                && !paths::no_image_file(&story.id).exists()
            {
                missing_images.push((story.id.clone(), story.original_url.clone()));
            }
        }
    }
    if wanted.is_empty() {
        body.append(&briefing_card(&edition));
    }
    let mut covers = Vec::new();
    if prefs::get().layout == "compact" {
        for section in &edition {
            if wanted.is_empty() {
                let name = section.name.clone();
                body.append(&shelf_head(&section.name, move || window::open_topic(&name)));
            }
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.add_css_class("headline-list");
            let take = if wanted.is_empty() { 6 } else { usize::MAX };
            for story in section.stories.iter().take(take) {
                list.append(&headline_row(story));
            }
            body.append(&list);
        }
    } else {
        let per_row = if window::is_narrow() { 2 } else { 3 };
        let lead = edition[0].stories[0].clone();
        body.append(&lead_story(&lead, &mut covers));
        let mut shown: HashSet<String> = HashSet::from([lead.id.clone()]);
        if wanted.is_empty() {
            let mut also: Vec<ArticleListItem> =
                edition.iter().skip(1).filter_map(|s| s.stories.first()).take(per_row).cloned().collect();
            for story in edition.iter().flat_map(|s| s.stories.iter()) {
                if also.len() >= per_row {
                    break;
                }
                if !shown.contains(&story.id) && !also.iter().any(|a| a.id == story.id) {
                    also.push(story.clone());
                }
            }
            shown.extend(also.iter().map(|a| a.id.clone()));
            body.append(&shelf_head("Also today", || window::navigate(&View::Unread)));
            body.append(&card_row(&also, per_row, &mut covers));
            for section in &edition {
                let rest: Vec<ArticleListItem> = section.stories.iter().filter(|s| !shown.contains(&s.id)).cloned().collect();
                if rest.is_empty() {
                    continue;
                }
                let name = section.name.clone();
                body.append(&shelf_head(&section.name, move || window::open_topic(&name)));
                let cards: Vec<ArticleListItem> = rest.iter().take(per_row).cloned().collect();
                body.append(&card_row(&cards, per_row, &mut covers));
                let more: Vec<&ArticleListItem> = rest.iter().skip(per_row).take(4).collect();
                if !more.is_empty() {
                    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    list.add_css_class("headline-list");
                    for story in more {
                        list.append(&headline_row(story));
                    }
                    body.append(&list);
                }
                shown.extend(rest.iter().map(|s| s.id.clone()));
            }
        } else {
            let rest: Vec<ArticleListItem> = edition[0].stories.iter().filter(|s| !shown.contains(&s.id)).cloned().collect();
            let cards: Vec<ArticleListItem> = rest.iter().take(per_row * 2).cloned().collect();
            if !cards.is_empty() {
                body.append(&shelf_head("More stories", || {}));
            }
            for chunk in cards.chunks(per_row) {
                body.append(&card_row(chunk, per_row, &mut covers));
            }
            let more: Vec<&ArticleListItem> = rest.iter().skip(per_row * 2).collect();
            if !more.is_empty() {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                list.add_css_class("headline-list");
                for story in more {
                    list.append(&headline_row(story));
                }
                body.append(&list);
            }
        }
    }
    worker::covers(covers);
    if !missing_images.is_empty() {
        worker::discover_images(missing_images);
    }
    count
}

/// A shelf's heading, with "See all" when it leads somewhere.
fn shelf_head(name: &str, see_all: impl Fn() + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("shelf-head");
    let head = widgets::label(name, "shelf-title");
    head.set_hexpand(true);
    row.append(&head);
    if name != "Other" && name != "More stories" {
        let more = gtk::Button::new();
        more.add_css_class("see-all");
        let face = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        face.append(&gtk::Label::new(Some("See all")));
        face.append(&gtk::Image::from_icon_name("pan-end-symbolic"));
        more.set_child(Some(&face));
        more.set_tooltip_text(Some(&if name == "Also today" { "Every recent story".to_string() } else { format!("Every {name} story") }));
        more.connect_clicked(move |_| see_all());
        row.append(&more);
    }
    row
}

fn card_row(stories: &[ArticleListItem], per_row: usize, covers: &mut Vec<String>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("card-row");
    row.set_homogeneous(true);
    for story in stories.iter().take(per_row) {
        row.append(&story_card(story, covers));
    }
    for _ in stories.len()..per_row {
        row.append(&gtk::Box::new(gtk::Orientation::Vertical, 0));
    }
    row
}

fn save_label(saved: bool) -> gtk::Box {
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    face.append(&gtk::Image::from_icon_name("user-bookmarks-symbolic"));
    face.append(&gtk::Label::new(Some(if saved { "Saved" } else { "Save" })));
    face
}

/// The lead story: its photo beside the headline (above it in a narrow window).
fn lead_story(article: &ArticleListItem, covers: &mut Vec<String>) -> gtk::Box {
    let narrow = window::is_narrow();
    let lead = gtk::Box::new(if narrow { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal }, if narrow { 12 } else { 20 });
    lead.add_css_class("lead");
    if narrow {
        lead.add_css_class("narrow");
    }
    lead.set_homogeneous(!narrow);
    lead.set_cursor_from_name(Some("pointer"));
    let art = cover(article, -1, if narrow { 200 } else { 250 }, covers);
    lead.append(&art);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.add_css_class("lead-text");
    column.append(&look::kicker(article));
    let title = widgets::label(headline(&article.title), "lead-headline");
    if article.read {
        title.add_css_class("read");
    }
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(3);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&title);
    if !article.summary.is_empty() {
        let summary = widgets::label(&article.summary, "lead-summary");
        summary.set_wrap(true);
        summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        summary.set_lines(if narrow { 2 } else { 4 });
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        column.append(&summary);
    }
    if !narrow {
        let gap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        gap.set_vexpand(true);
        column.append(&gap);
    }
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    foot.add_css_class("lead-meta");
    foot.append(&widgets::label(&format!("{} ·", source::read_time(&article.summary)), ""));
    let save = gtk::Button::new();
    save.add_css_class("lead-save");
    save.set_child(Some(&save_label(article.saved)));
    save.set_tooltip_text(Some(if article.saved { "Remove from Saved" } else { "Save to read later" }));
    let id = article.id.clone();
    let saved = article.saved;
    save.connect_clicked(move |_| {
        let _ = window::with_db(|db| db.set_saved(&id, !saved));
        window::toast(if saved { "Story removed from Saved" } else { "Story saved" });
        window::reload();
    });
    foot.append(&save);
    column.append(&foot);
    lead.append(&column);

    let click = gtk::GestureClick::new();
    let story = article.clone();
    let save_button = save.clone();
    click.connect_released(move |gesture, _, x, y| {
        let Some(widget) = gesture.widget() else { return };
        if let Some(hit) = widget.pick(x, y, gtk::PickFlags::DEFAULT)
            && (hit == *save_button.upcast_ref::<gtk::Widget>() || hit.is_ancestor(&save_button))
        {
            return;
        }
        reader::show(&story);
    });
    lead.add_controller(click);
    lead
}

fn story_card(article: &ArticleListItem, covers: &mut Vec<String>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("story-card");
    button.set_hexpand(true);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_hexpand(true);
    column.append(&cover(article, -1, 140, covers));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.append(&look::kicker(article));
    let title = widgets::label(headline(&article.title), "card-headline");
    if article.read {
        title.add_css_class("read");
    }
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(3);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    column.append(&text);
    button.set_child(Some(&column));
    let article = article.clone();
    button.connect_clicked(move |_| reader::show(&article));
    button
}

fn is_external_article(article: &ArticleListItem) -> bool {
    nyx_news_core::requires_publisher_site(&article.original_url) || paths::unavailable_file(&article.id).exists()
}

fn cover(article: &ArticleListItem, width: i32, height: i32, covers: &mut Vec<String>) -> gtk::Widget {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("cover-frame");
    overlay.set_size_request(width, height);
    overlay.set_hexpand(width < 0);
    overlay.set_overflow(gtk::Overflow::Hidden);

    let fallback = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fallback.add_css_class("cover-fallback");
    if let Some(topic) = look::topic_of(&article.feed_id) {
        fallback.add_css_class(&format!("section-{}", look::topic_swatch(&topic)));
    }
    fallback.set_size_request(width, height);
    fallback.set_hexpand(width < 0);
    fallback.set_halign(gtk::Align::Fill);
    fallback.set_valign(gtk::Align::Fill);
    let pub_name = source::name(&article.original_url, &article.feed_title);
    let letter = widgets::label(&look::initials(&pub_name), "cover-mark");
    letter.set_halign(gtk::Align::Center);
    letter.set_valign(gtk::Align::Center);
    letter.set_vexpand(true);
    fallback.append(&letter);
    overlay.set_child(Some(&fallback));

    if let Some(url) = article.image_url.as_ref().filter(|url| url.starts_with("https://")) {
        let path = paths::cover_file(url);
        if path.exists() {
            let picture = gtk::Picture::for_filename(&path);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            picture.set_halign(gtk::Align::Fill);
            picture.set_valign(gtk::Align::Fill);
            overlay.add_overlay(&picture);
            overlay.set_measure_overlay(&picture, false);
        } else {
            covers.push(url.clone());
        }
    }

    if is_external_article(article) {
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        badge.add_css_class("publisher-badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(6);
        badge.set_margin_end(6);
        let icon = gtk::Image::from_icon_name("nyx-news-external-symbolic");
        icon.set_pixel_size(11);
        badge.append(&icon);
        badge.append(&widgets::label("Web", "publisher-badge-label"));
        badge.set_tooltip_text(Some("Opens at the publisher"));
        overlay.add_overlay(&badge);
        overlay.set_measure_overlay(&badge, false);
    }
    overlay.upcast()
}

fn headline_row(article: &ArticleListItem) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("headline-row");
    button.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("unread-dot");
    if !article.read {
        dot.add_css_class("active");
    }
    dot.set_valign(gtk::Align::Center);
    row.append(&dot);

    let title = widgets::label(headline(&article.title), "headline-title");
    if article.read {
        title.add_css_class("read");
    }
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);

    if is_external_article(article) {
        let ext = gtk::Image::from_icon_name("nyx-news-external-symbolic");
        ext.add_css_class("external-icon");
        ext.set_pixel_size(12);
        ext.set_tooltip_text(Some("Opens at the publisher"));
        row.append(&ext);
    }

    let line = source::byline(&article.original_url, &article.feed_title, article.published_at);
    let meta = widgets::label(&line, "kicker-source");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_xalign(1.0);
    meta.set_valign(gtk::Align::Center);
    row.append(&meta);

    button.set_child(Some(&row));
    let article = article.clone();
    button.connect_clicked(move |_| reader::show(&article));
    button
}

fn headline(title: &str) -> &str {
    if title.is_empty() { "Untitled" } else { title }
}

pub fn select_position(pos: u32) {
    if let Some(selection) = SELECTION.with(|s| s.borrow().clone()) {
        selection.set_selected(pos);
    }
}

pub fn show_list() {
    DISCOVER.with(|d| d.set(false));
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("list");
    }
}

pub fn show_discover() {
    DISCOVER.with(|d| d.set(true));
    refresh_discover();
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("discover");
    }
}

pub fn refresh_discover() {
    if let Some(host) = DISCOVER_HOST.with(|host| host.borrow().clone()) {
        discover_page::populate(&host);
    }
}

pub fn discover_visible() -> bool {
    DISCOVER.with(|d| d.get())
}
