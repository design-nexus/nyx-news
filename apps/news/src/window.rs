//! The News window: the header (the app and the sidebar toggle; the search in the
//! middle; Follow and Settings at the end) over the sidebar and the pages. The sidebar
//! groups Today, your topics and the publications you follow; it collapses to an icon
//! rail under 980 px or when the user collapses it.

use crate::dialogs;
use crate::look;
use crate::paths;
use crate::prefs;
use crate::reader;
use crate::settings_dialog;
use crate::source;
use crate::timeline;
use crate::widgets;
use crate::worker;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nyx_news_core::{ArticleListItem, Db, View, open};
use nyx_ui::chrome;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "F"], "Search stories"),
    (&["/"], "Quick search filter"),
    (&["Ctrl", "N"], "Follow a feed"),
    (&["J"], "Next story"),
    (&["K"], "Previous story"),
    (&["Enter"], "Read story"),
    (&["B"], "Save story (Bookmark)"),
    (&["O"], "Open at the publisher"),
    (&["R"], "Refresh news"),
    (&["Esc"], "Back to the stories or clear search"),
    (&["Ctrl", "B"], "Collapse or expand sidebar"),
    (&["Ctrl", ","], "Settings"),
    (&["F1"], "Show keyboard shortcuts"),
    (&["Ctrl", "Q"], "Close"),
];

/// Publications the Following group shows before "All publications".
const FOLLOWING_SHOWN: usize = 6;

struct Ui {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    nav: gtk::Box,
    nav_list: gtk::Box,
    search: gtk::SearchEntry,
    view: View,
    articles: Vec<ArticleListItem>,
    selected: Option<String>,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: Cell<bool> = const { Cell::new(false) };
    static TOAST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static ALL_PUBLICATIONS: Cell<bool> = const { Cell::new(false) };
}

pub fn is_narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

pub fn overlay() -> Option<gtk::Overlay> {
    ui().map(|u| u.borrow().overlay.clone())
}

pub fn with_db<T>(f: impl FnOnce(&mut Db) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut db = open(&paths::library_db())?;
    f(&mut db)
}

pub fn articles() -> Vec<ArticleListItem> {
    ui().map(|u| u.borrow().articles.clone()).unwrap_or_default()
}

pub fn find_article(id: &str) -> Option<ArticleListItem> {
    ui()?.borrow().articles.iter().find(|a| a.id == id).cloned()
}

pub fn view() -> View {
    ui().map(|u| u.borrow().view.clone()).unwrap_or(View::All)
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if ui().is_none() {
        build(app);
        worker::start();
        if prefs::get().refresh_on_startup {
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), worker::refresh_due);
        }
        if prefs::get().sync_enabled {
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), worker::sync_background);
        }
    }
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.present();
    match section {
        Some("settings") => settings_dialog::open(),
        Some("discover") => show_discover(),
        Some("all") | Some("following") | None => {
            if !prefs::get().saw_headlines {
                prefs::update(|prefs| prefs.saw_headlines = true);
                navigate(&View::All);
            } else {
                let start = nyx_theme::start_page("all", &prefs::get().last_section);
                navigate(&view_from_section(&start));
            }
        }
        Some(other) => navigate(&view_from_section(other)),
    }
    // Developer aid: a test copy on a virtual monitor opens the page's first story.
    if std::env::var_os("NYX_NEWS_TEST_OPEN").is_some() {
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), || {
            if let Some(article) = articles().into_iter().next() {
                reader::show(&article);
            }
        });
    }}

fn view_from_section(section: &str) -> View {
    match section {
        "unread" => View::Unread,
        "starred" => View::Starred,
        "saved" => View::Saved,
        "foryou" if prefs::get().recommendations => View::ForYou,
        "discover" => View::All,
        id if id.starts_with("feed:") => View::Feed(id.trim_start_matches("feed:").into()),
        id if id.starts_with("feeds:") => {
            let wanted: Vec<String> = id.trim_start_matches("feeds:").split(',').map(str::to_string).collect();
            let have = with_db(|db| db.feeds()).unwrap_or_default();
            let ids: Vec<String> = wanted.into_iter().filter(|id| have.iter().any(|f| f.id == *id)).collect();
            if ids.is_empty() { View::All } else { View::Feeds(ids) }
        }
        id if id.starts_with("folder:") => View::Folder(id.trim_start_matches("folder:").into()),
        id if id.starts_with("topic:") => View::Topic(id.trim_start_matches("topic:").into()),
        _ => View::All,
    }
}

fn section_id(view: &View) -> String {
    match view {
        View::All => "all".into(),
        View::Unread => "unread".into(),
        View::Starred => "starred".into(),
        View::Saved => "saved".into(),
        View::Feed(id) => format!("feed:{id}"),
        View::Feeds(ids) => format!("feeds:{}", ids.join(",")),
        View::Folder(id) => format!("folder:{id}"),
        View::ForYou => "foryou".into(),
        View::Topic(id) => format!("topic:{id}"),
    }
}

fn page_name(view: &View) -> String {
    match view {
        View::All => "Top stories".into(),
        View::Unread => "Recent stories".into(),
        View::Starred => "Starred".into(),
        View::Saved => "Saved".into(),
        View::Feed(id) => feed_title(id),
        View::Feeds(ids) => ids.first().map(|id| publication_name(id)).unwrap_or_else(|| "Publication".into()),
        View::Folder(id) => folder_name(id),
        View::ForYou => "For you".into(),
        View::Topic(id) => topic_name(id),
    }
}

fn feed_title(id: &str) -> String {
    with_db(|db| Ok(db.feed(id)?.map(|f| f.title).unwrap_or_else(|| "Feed".into()))).unwrap_or_else(|_| "Feed".into())
}

fn publication_name(feed_id: &str) -> String {
    with_db(|db| Ok(db.feed(feed_id)?.map(|f| source::name(&f.url, &f.title)).unwrap_or_else(|| "Publication".into())))
        .unwrap_or_else(|_| "Publication".into())
}

fn topic_name(id: &str) -> String {
    with_db(|db| {
        Ok(db
            .smart_filters()?
            .into_iter()
            .find(|filter| filter.id == id)
            .map(|filter| filter.name)
            .unwrap_or_else(|| "Topic".into()))
    })
    .unwrap_or_else(|_| "Topic".into())
}

fn folder_name(id: &str) -> String {
    with_db(|db| Ok(db.folders()?.into_iter().find(|f| f.id == id).map(|f| f.name).unwrap_or_else(|| "Folder".into())))
        .unwrap_or_else(|_| "Folder".into())
}

pub fn navigate(view: &View) {
    let Some(ui) = ui() else { return };
    ui.borrow_mut().view = view.clone();
    prefs::update(|p| p.last_section = section_id(view));
    chrome::set_title(&ui.borrow().window, "News", &current_page_name());
    timeline::show_list();
    reader::hide();
    reload();
}

/// Opens one of the front page's topics (World, Science…).
pub fn open_topic(name: &str) {
    timeline::show_section(name);
    navigate(&View::All);
    worker::open_section(name.to_string());
}

pub fn show_discover() {
    timeline::show_discover();
    reader::hide();
    let ui = ui().unwrap();
    chrome::set_title(&ui.borrow().window, "News", "Channels and topics");
    rebuild_nav();
}

pub fn refresh_lists() {
    reload();
}

pub fn apply_synced_prefs(fields: std::collections::BTreeMap<String, String>) {
    prefs::update(|prefs| {
        if let Some(days) = fields.get("retention_days").and_then(|value| value.parse().ok())
            && prefs.retention_days != days
        {
            prefs.retention_days = days;
        }
        if let Some(layout) = fields.get("layout")
            && (layout == "list" || layout == "compact")
            && prefs.layout != *layout
        {
            prefs.layout = layout.clone();
        }
        if let Some(minutes) = fields.get("poll_minutes").and_then(|value| value.parse::<i64>().ok()) {
            let minutes = minutes.clamp(15, 24 * 60);
            if prefs.poll_minutes != minutes {
                prefs.poll_minutes = minutes;
            }
        }
        if let Some(mark) = fields.get("mark_read_on_open") {
            let mark = mark == "true" || mark == "1";
            if prefs.mark_read_on_open != mark {
                prefs.mark_read_on_open = mark;
            }
        }
        if let Some(on) = fields.get("recommendations") {
            let on = on == "true" || on == "1";
            if prefs.recommendations != on {
                prefs.recommendations = on;
            }
        }
        if let Some(on) = fields.get("summaries") {
            let on = on == "true" || on == "1";
            if prefs.summaries != on {
                prefs.summaries = on;
            }
        }
    });
    reader::set_ai_visible(prefs::get().summaries);
    let _ = with_db(|db| db.set_poll_minutes(prefs::get().poll_minutes));
    if !prefs::get().recommendations && view() == View::ForYou {
        navigate(&View::All);
    } else {
        refresh_lists();
    }
}

pub fn search_active() -> bool {
    ui().map(|u| !u.borrow().search.text().trim().is_empty()).unwrap_or(false)
}

pub fn search_query() -> String {
    ui().map(|u| u.borrow().search.text().trim().to_string()).unwrap_or_default()
}

pub fn current_page_name() -> String {
    let Some(ui) = ui() else { return "Top stories".into() };
    let v = ui.borrow().view.clone();
    if v == View::All {
        let s = timeline::section();
        if !s.is_empty() {
            return s;
        }
    }
    page_name(&v)
}

pub fn reload() {
    let Some(ui) = ui() else { return };
    let view = ui.borrow().view.clone();
    let query = ui.borrow().search.text().to_string();
    look::learn_topics();
    let loaded = with_db(|db| {
        db.set_poll_minutes(prefs::get().poll_minutes)?;
        match &view {
            View::ForYou => db.for_you(200),
            View::Topic(id) => db.topic_articles(id, 200),
            View::All if query.trim().is_empty() => {
                let section = timeline::section();
                if section.is_empty() { db.edition_articles(8) } else { db.section_articles(&section, "", 40) }
            }
            View::All => {
                let section = timeline::section();
                if section.is_empty() {
                    db.articles(&View::All, query.trim(), 200, 0)
                } else {
                    db.section_articles(&section, query.trim(), 200)
                }
            }
            other => db.articles(other, query.trim(), 200, 0),
        }
    });
    match loaded {
        Ok(items) => {
            ui.borrow_mut().articles = items;
            timeline::bind_articles();
        }
        Err(err) => toast(&err.to_string()),
    }
    rebuild_nav();
}

pub fn selected_id() -> Option<String> {
    ui()?.borrow().selected.clone()
}

pub fn select_id(id: Option<String>) {
    if let Some(ui) = ui() {
        ui.borrow_mut().selected = id;
    }
}

pub fn move_selection(delta: i32) {
    let Some(ui) = ui() else { return };
    let articles = ui.borrow().articles.clone();
    if articles.is_empty() {
        return;
    }
    let current = ui.borrow().selected.clone();
    let index = current.and_then(|id| articles.iter().position(|a| a.id == id)).unwrap_or(0);
    let next = (index as i32 + delta).clamp(0, articles.len() as i32 - 1) as usize;
    let id = articles[next].id.clone();
    ui.borrow_mut().selected = Some(id.clone());
    timeline::select_position(next as u32);
    reader::show(&articles[next]);
}

/// What a sidebar row shows after its name.
enum End {
    None,
    Count(i64),
    Badge(i64),
}

fn nav_row(lead: &impl IsA<gtk::Widget>, title: &str, tooltip: &str, end: End, active: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    if active {
        button.add_css_class("active");
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(lead);
    let label = widgets::label(title, "nav-label");
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("compact-hide");
    content.append(&label);
    let readout = match end {
        End::Count(n) if n > 0 => Some(widgets::label(&n.to_string(), "nav-readout")),
        End::Badge(n) if n > 0 => Some(widgets::label(&n.to_string(), "nav-badge")),
        _ => None,
    };
    if let Some(readout) = readout {
        readout.add_css_class("compact-hide");
        readout.set_valign(gtk::Align::Center);
        content.append(&readout);
    }
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(title)]);
    button
}

fn icon(name: &str) -> gtk::Image {
    let image = gtk::Image::from_icon_name(name);
    image.add_css_class("nav-icon");
    image
}

/// A group's heading in its section's colour, and the icon rail's bar in the same colour.
fn nav_group(list: &gtk::Box, title: &str, section: &str, first: bool, add: Option<(&str, fn())>) {
    let bar = gtk::Separator::new(gtk::Orientation::Horizontal);
    bar.add_css_class("nav-divider");
    bar.add_css_class("compact-show");
    bar.add_css_class(&format!("section-{section}"));
    bar.set_visible(false);
    list.append(&bar);
    let row = match add {
        Some((tip, run)) => nyx_ui::sidebar::group_label_with_add(title, first, tip, run),
        None => nyx_ui::sidebar::group_label(title, first),
    };
    nyx_ui::sidebar::set_section(&row, section);
    list.append(&row);
}

fn rebuild_nav() {
    let Some(ui) = ui() else { return };
    let list = ui.borrow().nav_list.clone();
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let current = section_id(&ui.borrow().view);
    let discover_open = timeline::discover_visible();
    let open_section = timeline::section();
    let on = |id: &str| !discover_open && current == id;

    let feeds = with_db(|db| db.feeds()).unwrap_or_default();
    let recent = with_db(|db| db.unread_since(source::now() - 24 * 60 * 60)).unwrap_or_default();
    let (_, _, saved_count) = with_db(|db| db.counts()).unwrap_or((0, 0, 0));

    nav_group(&list, "Today", nyx_ui::sidebar::ACCENT_SECTION, true, None);
    let top = nav_row(
        &icon("nyx-news-newspaper-symbolic"),
        "Top stories",
        "Today's front page",
        End::Count(timeline::front_count() as i64),
        on("all") && open_section.is_empty(),
    );
    top.connect_clicked(|_| {
        timeline::show_section("");
        navigate(&View::All);
    });
    list.append(&top);
    let for_you = nav_row(&icon("non-starred-symbolic"), "For you", "Stories picked from what you read", End::None, on("foryou"));
    for_you.connect_clicked(|_| navigate(&View::ForYou));
    list.append(&for_you);
    let saved = nav_row(&icon("user-bookmarks-symbolic"), "Saved", "Stories saved to read later", End::Badge(saved_count), on("saved"));
    saved.connect_clicked(|_| navigate(&View::Saved));
    list.append(&saved);

    nav_group(&list, "Topics", "swatch-violet", false, None);
    let disabled = prefs::get().disabled_categories;
    let sections = with_db(|db| db.feed_sections()).unwrap_or_default();
    let topics: Vec<String> = nyx_news_core::directory()
        .map(|topics| topics.into_iter().map(|topic| topic.title).filter(|name| !disabled.contains(name)).collect())
        .unwrap_or_default();
    for name in topics {
        let unread: i64 = sections.iter().filter(|(_, topic)| *topic == name).map(|(feed, _)| recent.get(feed).copied().unwrap_or(0)).sum();
        let button = nav_row(&look::dot(&name), &name, &format!("{name}: {unread} new today"), End::Count(unread), on("all") && open_section == name);
        button.connect_clicked(move |_| open_topic(&name));
        list.append(&button);
    }
    for topic in with_db(|db| db.smart_filters()).unwrap_or_default() {
        let id = topic.id.clone();
        let button = nav_row(&look::dot(&topic.name), &topic.name, &format!("Stories that mention {}", topic.name), End::None, on(&format!("topic:{id}")));
        button.connect_clicked(move |_| navigate(&View::Topic(id.clone())));
        list.append(&button);
    }

    nav_group(&list, "Following", "swatch-amber", false, Some(("Follow a feed (Ctrl+N)", dialogs::add_feed)));
    let publications = source::publications(&feeds, &recent);
    let all = ALL_PUBLICATIONS.with(|a| a.get());
    for (index, publication) in publications.iter().enumerate() {
        let view = View::Feeds(publication.feeds.clone());
        let active = on(&section_id(&view));
        if !all && index >= FOLLOWING_SHOWN && !active {
            continue;
        }
        let button = nav_row(
            &icon("application-rss+xml-symbolic"),
            &publication.name,
            &format!("{}: {} new today", publication.name, publication.unread),
            End::Count(publication.unread),
            active,
        );
        button.connect_clicked(move |_| navigate(&view));
        list.append(&button);
    }
    if publications.len() > FOLLOWING_SHOWN {
        let (title, tip) = if all {
            ("Fewer publications".to_string(), "Show the publications with the most new stories")
        } else {
            (format!("All {} publications", publications.len()), "Show every publication you follow")
        };
        let more = nav_row(&icon("view-more-horizontal-symbolic"), &title, tip, End::None, false);
        more.add_css_class("nav-more");
        more.add_css_class("compact-hide");
        more.connect_clicked(|_| {
            ALL_PUBLICATIONS.with(|a| a.set(!a.get()));
            rebuild_nav();
        });
        list.append(&more);
    }
    let channels = nav_row(
        &icon("nyx-news-compass-symbolic"),
        "Channels and topics",
        "Follow topics and publications",
        End::None,
        discover_open,
    );
    channels.connect_clicked(|_| show_discover());
    list.append(&channels);

    if let Some(at) = feeds.iter().filter_map(|f| f.last_ok_at).max() {
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        foot.add_css_class("nav-foot");
        foot.add_css_class("compact-hide");
        let failing = feeds.iter().filter(|f| f.consecutive_failures > 0).count();
        let mark = gtk::Image::from_icon_name(if failing > 0 { "dialog-warning-symbolic" } else { "object-select-symbolic" });
        mark.add_css_class("status-icon");
        mark.add_css_class(if failing > 0 { "warn" } else { "ok" });
        foot.append(&mark);
        let text = widgets::label(&source::updated(at), "status-readout");
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        foot.append(&text);
        if failing > 0 {
            foot.set_tooltip_text(Some(&if failing == 1 {
                "1 feed didn't refresh".to_string()
            } else {
                format!("{failing} feeds didn't refresh")
            }));
        }
        list.append(&foot);
    }

    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn build(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app).title("News").default_width(1280).default_height(800).build();
    window.add_css_class("news-window");
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some("io.github.design_nexus.nyx.News"));
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common {
            settings: Some(settings_dialog::open),
            shortcuts: Some(show_shortcuts),
            close: true,
            ..Default::default()
        },
    );

    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_hexpand(false);
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hexpand(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    // The search, always in the header (Ctrl+F or /).
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search stories"));
    search.add_css_class("bar-search");
    search.set_width_chars(SEARCH_WIDE);
    search.update_property(&[gtk::accessible::Property::Label("Search stories")]);
    search.connect_search_changed(|_| reload());
    search.connect_stop_search(|entry| entry.set_text(""));
    let search_box = gtk::Overlay::new();
    search_box.set_child(Some(&search));
    let hint = widgets::label("Ctrl F", "key-hint");
    hint.set_halign(gtk::Align::End);
    hint.set_valign(gtk::Align::Center);
    hint.set_margin_end(10);
    hint.set_can_target(false);
    search_box.add_overlay(&hint);
    search.connect_changed(move |s| hint.set_visible(s.text().is_empty()));

    let pages = timeline::build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("settings-content");
    content.set_hexpand(true);
    content.append(&pages);

    // ----- Header -----
    // The app and the sidebar toggle; the search in the middle; Follow and Settings at
    // the end. The page's own bar carries its name, count, layout and Refresh.
    let header = chrome::app_header(&window, "application-rss+xml-symbolic", "News");
    let collapse = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse);
    header.center.append(&search_box);
    let follow = gtk::Button::new();
    follow.add_css_class("new-button");
    follow.set_tooltip_text(Some("Follow a feed (Ctrl+N)"));
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    face.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    face.append(&gtk::Label::new(Some("Follow")));
    face.append(&widgets::label("Ctrl N", "key-hint"));
    follow.set_child(Some(&face));
    follow.set_valign(gtk::Align::Center);
    follow.connect_clicked(|_| dialogs::add_feed());
    header.end.append(&follow);
    header.end.append(&chrome::settings_button(settings_dialog::open));

    let frame = nyx_ui::frame::AppFrame::new(&header.root);
    frame.set_sidebar(&nav);
    frame.set_page(&content);
    frame.set_flush(true);
    let overlay = frame.install(&window);
    install_keys(&window, &search);

    let ui = Ui {
        window: window.clone(),
        overlay: overlay.clone(),
        nav: nav.clone(),
        nav_list: list,
        search,
        view: View::All,
        articles: Vec::new(),
        selected: None,
    };
    UI.with(|slot| *slot.borrow_mut() = Some(Rc::new(RefCell::new(ui))));

    let apply_width = {
        let nav = nav.clone();
        let window = window.clone();
        move || {
            let width = if window.width() > 0 { window.width() } else { window.default_width() };
            let narrow = width > 0 && width < 980;
            settings_dialog::fit(&window);
            let prev = NARROW.with(|n| n.get());
            let sized = nav.has_css_class("sized");
            if narrow == prev && sized {
                return;
            }
            nav.add_css_class("sized");
            NARROW.with(|n| n.set(narrow));
            apply_compact(&nav, narrow || prefs::get().sidebar_collapsed);
            reader::set_narrow(narrow);
            // A tiled half-screen header gets a shorter search.
            if let Some(ui) = self::ui() {
                ui.borrow().search.set_width_chars(if narrow { SEARCH_NARROW } else { SEARCH_WIDE });
            }
            if sized && prev != narrow {
                reload();
            }
        }
    };
    let aw = apply_width.clone();
    window.connect_realize(move |_| aw());
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    probe.set_can_focus(false);
    overlay.add_overlay(&probe);
    overlay.set_measure_overlay(&probe, false);
    probe.connect_resize(move |_, _, _| {
        let aw = apply_width.clone();
        glib::idle_add_local_once(aw);
    });
    rebuild_nav();
}

/// The search's width in characters, in a full window and a narrow one.
const SEARCH_WIDE: i32 = 30;
const SEARCH_NARROW: i32 = 14;

fn apply_compact(nav: &gtk::Box, compact: bool) {
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    fn walk(widget: &gtk::Widget, compact: bool) {
        if widget.has_css_class("compact-hide") {
            widget.set_visible(!compact);
        }
        if widget.has_css_class("compact-show") {
            widget.set_visible(compact);
        }
        // Icon-only: centre the icon in its row. The header buttons are already centred.
        if widget.has_css_class("nav-item")
            && let Some(content) = widget.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            walk(&next, compact);
            child = next.next_sibling();
        }
    }
    walk(nav.upcast_ref(), compact);
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn typing(window: &gtk::Window) -> bool {
    let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) else { return false };
    focus.is::<gtk::Text>() || focus.is::<gtk::TextView>() || focus.ancestor(gtk::Entry::static_type()).is_some()
}

fn install_keys(window: &gtk::ApplicationWindow, search: &gtk::SearchEntry) {
    let keys = gtk::EventControllerKey::new();
    let search = search.clone();
    let win = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        if ctrl && key == gdk::Key::f {
            search.grab_focus();
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::n {
            dialogs::add_feed();
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::i {
            dialogs::import_opml(&win);
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::e {
            dialogs::export_opml(&win);
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::b {
            toggle_sidebar();
            return glib::Propagation::Stop;
        }
        if typing(win.upcast_ref()) {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::j | gdk::Key::Down => move_selection(1),
            gdk::Key::k | gdk::Key::Up => move_selection(-1),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(id) = selected_id()
                    && let Some(article) = find_article(&id)
                {
                    reader::show(&article);
                }
            }
            gdk::Key::b | gdk::Key::s => reader::toggle_save(),
            gdk::Key::o => reader::open_original(),
            gdk::Key::r => worker::refresh_current(),
            gdk::Key::slash => {
                search.grab_focus();
            }
            gdk::Key::Escape => {
                if !search.text().is_empty() {
                    search.set_text("");
                } else {
                    reader::hide();
                    timeline::show_list();
                }
            }
            gdk::Key::F1 => show_shortcuts(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

pub fn show_shortcuts() {
    let (dialog, card) = widgets::dialog("Keyboard shortcuts", 520);
    let list = widgets::vbox(0);
    list.add_css_class("group-list");
    for (keys, what) in SHORTCUTS {
        list.append(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&list)
        .build();
    card.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let dialog2 = dialog.clone();
    close.connect_clicked(move |_| dialog2.close());
    card.append(&close);
    dialog.present();
}

pub fn notify(title: &str, body: &str) {
    let Some(window) = window() else {
        return;
    };
    if window.is_visible() && window.is_active() {
        toast(&format!("{title}: {body}"));
        return;
    }
    let Some(app) = window.application() else { return };
    let notification = gio::Notification::new(title);
    notification.set_body(Some(body));
    notification.set_icon(&gio::ThemedIcon::new("io.github.design_nexus.nyx.News"));
    app.send_notification(Some("news-refresh"), &notification);
}

pub fn toast(message: &str) {
    let Some(ui) = ui() else {
        eprintln!("nyx-news: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    if let Some(old) = TOAST.with(|t| t.borrow_mut().take())
        && old.parent().is_some()
    {
        overlay.remove_overlay(&old);
    }
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.add_css_class("toast");
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    bx.set_margin_bottom(48);
    bx.append(&label);
    overlay.add_overlay(&bx);
    TOAST.with(|t| *t.borrow_mut() = Some(bx.clone()));
    let overlay = overlay.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
        if bx.parent().is_some() {
            overlay.remove_overlay(&bx);
        }
    });
}
