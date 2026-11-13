//! The story. Opening one replaces the page and loads the article text. Its bar puts
//! Open at the publisher first and Save beside it; Brief, Compare, Read aloud and
//! Export sit in one menu. With summaries on, a short summary from Assistant leads.

use crate::assistant_client;
use crate::cmd;
use crate::dialogs;
use crate::html_text;
use crate::look;
use crate::note;
use crate::paths;
use crate::prefs;
use crate::source;
use crate::timeline;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::prelude::*;
use nyx_news_core::{AiFailure, ArticleListItem, Excerpt};
use nyx_ui::clamp::Clamp;
use std::cell::{Cell, RefCell};

thread_local! {
    static TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static KICKER: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static COVER: RefCell<Option<gtk::Picture>> = const { RefCell::new(None) };
    static COVER_WRAP: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static META: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static BODY: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static BRIEF_CARD: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static BRIEF_TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static BRIEF_BODY: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static BRIEF_CANCEL: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    static BRIEF_TEXT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    static TASK: RefCell<String> = const { RefCell::new(String::new()) };
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    static OPEN_ID: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static BACK: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static OPEN: RefCell<Option<(gtk::Button, gtk::Label)>> = const { RefCell::new(None) };
    static SAVE: RefCell<Option<(gtk::Button, gtk::Label)>> = const { RefCell::new(None) };
    static PUBLISHER_BOX: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static NARROW: Cell<bool> = const { Cell::new(false) };
}

const UNAVAILABLE: &str = "This story did not open. It is still at the publisher.";

fn labeled(icon: &str, text: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    face.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(Some(text));
    face.append(&label);
    button.set_child(Some(&face));
    button.set_valign(gtk::Align::Center);
    (button, label)
}

pub fn build() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("reader-view");
    root.set_hexpand(true);
    root.set_vexpand(true);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    toolbar.add_css_class("page-bar");
    toolbar.add_css_class("reader-bar");

    let (back, back_label) = labeled("go-previous-symbolic", "Top stories");
    back.add_css_class("flat");
    back.add_css_class("reader-back");
    back.set_tooltip_text(Some("Back to the stories (Esc)"));
    back_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    back.connect_clicked(|_| hide());
    toolbar.append(&back);

    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    toolbar.append(&gap);

    let (open, open_label) = labeled("nyx-news-external-symbolic", "Open at the publisher");
    open.add_css_class("suggested-action");
    open.set_tooltip_text(Some("Open at the publisher (O)"));
    open.connect_clicked(|_| open_original());
    toolbar.append(&open);

    let (save, save_label) = labeled("user-bookmarks-symbolic", "Save");
    save.add_css_class("save-toggle");
    save.set_tooltip_text(Some("Save story (B)"));
    save.connect_clicked(|_| toggle_save());
    toolbar.append(&save);

    let copy = widgets::bar_button("edit-copy-symbolic", "Copy link");
    copy.connect_clicked(|_| {
        if let Some(url) = OPEN_URL.with(|p| p.borrow().clone())
            && let Some(display) = gtk::gdk::Display::default()
        {
            display.clipboard().set_text(&url);
            window::toast("Story link copied");
        }
    });
    toolbar.append(&copy);

    let font_toggle = widgets::bar_button("nyx-news-textsize-symbolic", "Text size");
    font_toggle.connect_clicked(|_| {
        let next = match prefs::get().reader_font_scale.as_str() {
            "small" => "medium",
            "medium" => "large",
            _ => "small",
        };
        prefs::update(|p| p.reader_font_scale = next.into());
        update_font_size();
        window::toast(&format!("Text size: {next}"));
    });
    toolbar.append(&font_toggle);

    let more = widgets::bar_button("view-more-horizontal-symbolic", "More: brief, compare sources, read aloud, export as a note");
    more.connect_clicked(|button| nyx_ui::menu::popup_below(button, more_entries()));
    toolbar.append(&more);

    root.append(&toolbar);

    let pane = gtk::Box::new(gtk::Orientation::Vertical, 12);
    pane.add_css_class("reader");
    pane.set_hexpand(true);

    let kicker = gtk::Box::new(gtk::Orientation::Horizontal, 0);

    let title = gtk::Label::new(None);
    title.add_css_class("reader-title");
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::Word);

    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    meta.add_css_class("reader-meta");

    let brief_card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    brief_card.add_css_class("in-short");
    let brief_icon = gtk::Image::from_icon_name("nyx-news-sparkle-symbolic");
    brief_icon.add_css_class("in-short-icon");
    brief_icon.set_valign(gtk::Align::Start);
    brief_card.append(&brief_icon);
    let brief_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    brief_column.set_hexpand(true);
    let brief_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let brief_title = widgets::label("In short", "in-short-title");
    brief_title.set_hexpand(true);
    brief_head.append(&brief_title);
    let cancel = widgets::bar_button("window-close-symbolic", "Stop");
    cancel.add_css_class("in-short-cancel");
    cancel.connect_clicked(|_| cancel_brief());
    brief_head.append(&cancel);
    brief_column.append(&brief_head);
    let brief_body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    brief_column.append(&brief_body);
    brief_card.append(&brief_column);
    brief_card.set_visible(false);

    let cover = gtk::Picture::new();
    cover.set_content_fit(gtk::ContentFit::Cover);
    cover.set_can_shrink(true);
    cover.set_height_request(280);
    cover.add_css_class("reader-cover");
    let cover_wrap = gtk::Overlay::new();
    cover_wrap.set_child(Some(&cover));
    cover_wrap.set_measure_overlay(&cover, false);
    cover_wrap.set_overflow(gtk::Overflow::Hidden);
    cover_wrap.add_css_class("reader-cover-frame");
    cover_wrap.set_visible(false);

    let body = gtk::Label::new(None);
    body.add_css_class("reader-body");
    body.set_hexpand(true);
    body.set_xalign(0.0);
    body.set_yalign(0.0);
    body.set_wrap(true);
    body.set_wrap_mode(gtk::pango::WrapMode::Word);

    pane.append(&kicker);
    pane.append(&title);
    pane.append(&meta);
    pane.append(&brief_card);
    pane.append(&cover_wrap);
    pane.append(&body);

    let pub_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    pub_box.add_css_class("publisher-box");
    pub_box.set_visible(false);
    let pub_notice = gtk::Label::new(Some("This story couldn't be laid out for reading here. The whole story is at the publisher."));
    pub_notice.add_css_class("publisher-notice");
    pub_notice.set_xalign(0.0);
    pub_notice.set_wrap(true);
    pub_box.append(&pub_notice);
    let (pub_btn, _) = labeled("nyx-news-external-symbolic", "Read it at the publisher");
    pub_btn.add_css_class("suggested-action");
    pub_btn.set_halign(gtk::Align::Start);
    pub_btn.connect_clicked(|_| open_original());
    pub_box.append(&pub_btn);
    pane.append(&pub_box);

    let column = Clamp::new(&pane, 660);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&column)
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .build();

    let swipe = gtk::GestureSwipe::new();
    swipe.connect_swipe(|_, vx, _vy| {
        if vx > 150.0 {
            hide();
        }
    });
    scroll.add_controller(swipe);

    let scroll_ctrl = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::HORIZONTAL
            | gtk::EventControllerScrollFlags::VERTICAL
            | gtk::EventControllerScrollFlags::KINETIC,
    );
    let accum_dx = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    let last_trigger = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now()));
    let accum = accum_dx.clone();
    let last = last_trigger.clone();
    scroll_ctrl.connect_scroll(move |_, dx, dy| {
        if dx.abs() > dy.abs() * 1.5 {
            let current = accum.get() + dx;
            accum.set(current);
            if current.abs() > 30.0 && last.get().elapsed().as_millis() > 500 {
                last.set(std::time::Instant::now());
                accum.set(0.0);
                hide();
                return gtk::glib::Propagation::Stop;
            }
        } else {
            accum.set(0.0);
        }
        gtk::glib::Propagation::Proceed
    });
    let accum_dec = accum_dx.clone();
    scroll_ctrl.connect_decelerate(move |_, _, _| {
        accum_dec.set(0.0);
    });
    scroll.add_controller(scroll_ctrl);

    root.append(&scroll);

    TITLE.with(|p| *p.borrow_mut() = Some(title));
    KICKER.with(|p| *p.borrow_mut() = Some(kicker));
    COVER.with(|p| *p.borrow_mut() = Some(cover));
    COVER_WRAP.with(|p| *p.borrow_mut() = Some(cover_wrap));
    META.with(|p| *p.borrow_mut() = Some(meta));
    BODY.with(|p| *p.borrow_mut() = Some(body));
    BRIEF_CARD.with(|p| *p.borrow_mut() = Some(brief_card));
    BRIEF_TITLE.with(|p| *p.borrow_mut() = Some(brief_title));
    BRIEF_BODY.with(|p| *p.borrow_mut() = Some(brief_body));
    BRIEF_CANCEL.with(|p| *p.borrow_mut() = Some(cancel));
    BACK.with(|p| *p.borrow_mut() = Some(back_label));
    OPEN.with(|p| *p.borrow_mut() = Some((open, open_label)));
    SAVE.with(|p| *p.borrow_mut() = Some((save, save_label)));
    PUBLISHER_BOX.with(|p| *p.borrow_mut() = Some(pub_box));
    root
}

fn more_entries() -> Vec<nyx_ui::menu::Entry> {
    use nyx_ui::menu::{item, separator};
    let mut entries = Vec::new();
    if prefs::get().summaries {
        entries.push(item("Brief today's top stories", || start_brief("brief")).icon("nyx-news-sparkle-symbolic"));
        entries.push(item("Compare sources", || start_brief("compare")).icon("view-dual-symbolic"));
        entries.push(item("Read aloud", || start_brief("spoken")).icon("audio-volume-high-symbolic"));
        entries.push(separator());
    }
    entries.push(item("Export as a note…", export_note).icon("document-save-symbolic"));
    entries
}

/// A narrow window shows Open at the publisher as its icon only.
pub fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    if let Some((_, label)) = OPEN.with(|p| p.borrow().clone()) {
        label.set_visible(!narrow);
    }
}

pub fn show(article: &ArticleListItem) {
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(false);
    }
    if let Some(cover_wrap) = COVER_WRAP.with(|p| p.borrow().clone()) {
        if let Some(url) = article.image_url.as_ref().filter(|url| paths::cover_file(url).exists()) {
            if let Some(cover) = COVER.with(|p| p.borrow().clone()) {
                cover.set_filename(Some(paths::cover_file(url)));
            }
            cover_wrap.set_visible(true);
        } else {
            cover_wrap.set_visible(false);
        }
    }
    let pub_name = source::name(&article.original_url, &article.feed_title);
    if let Some(kicker) = KICKER.with(|p| p.borrow().clone()) {
        while let Some(child) = kicker.first_child() {
            kicker.remove(&child);
        }
        kicker.append(&look::kicker(article));
    }
    if let Some(title) = TITLE.with(|p| p.borrow().clone()) {
        title.set_text(if article.title.is_empty() { "Untitled" } else { &article.title });
    }
    if let Some(back) = BACK.with(|p| p.borrow().clone()) {
        back.set_text(&if window::search_active() { "Search".to_string() } else { window::current_page_name() });
    }
    if let Some((open, label)) = OPEN.with(|p| p.borrow().clone()) {
        label.set_text(&format!("Open at {pub_name}"));
        label.set_visible(!NARROW.with(|n| n.get()));
        open.set_tooltip_text(Some(&format!("Open at {pub_name} (O)")));
    }
    let cached = article
        .content_hash
        .as_ref()
        .and_then(|hash| window::with_db(|db| Ok(db.body(&paths::content_cache(), hash))).ok().flatten());
    let saved = std::fs::read_to_string(paths::story_file(&article.id)).ok().filter(|text| text.len() >= 400);
    let ready = saved.or_else(|| cached.map(|html| html_text::plain(&html)).filter(|t| t.len() >= 400));
    let feed_fallback = if !article.summary.trim().is_empty() { Some(html_text::plain(&article.summary)) } else { None };
    let initial_text = ready.clone().or_else(|| feed_fallback.clone()).unwrap_or_else(|| "Opening the story…".into());
    if let Some(meta) = META.with(|p| p.borrow().clone()) {
        while let Some(child) = meta.first_child() {
            meta.remove(&child);
        }
        let author = article.author.trim();
        let who = if author.is_empty() { pub_name.clone() } else { author.to_string() };
        meta.append(&look::avatar(&who, 26));
        let mut line = if author.is_empty() {
            format!("<b>{}</b> · {}", gtk::glib::markup_escape_text(&pub_name), source::read_time(&initial_text))
        } else {
            format!("By <b>{}</b> · {}", gtk::glib::markup_escape_text(author), source::read_time(&initial_text))
        };
        if ready.is_some() {
            line.push_str(" · Saved for offline reading");
        }
        let label = gtk::Label::new(None);
        label.set_markup(&line);
        label.set_xalign(0.0);
        label.set_wrap(true);
        meta.append(&label);
    }
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.set_text(&initial_text);
    }
    update_font_size();
    OPEN_ID.with(|p| *p.borrow_mut() = Some(article.id.clone()));
    OPEN_URL.with(|p| *p.borrow_mut() = Some(article.original_url.clone()));
    if ready.is_none() && (article.original_url.starts_with("https://") || article.original_url.starts_with("http://")) {
        worker::story(article.id.clone(), article.original_url.clone());
    }
    clear_brief();
    set_ai_visible(prefs::get().summaries);
    window::select_id(Some(article.id.clone()));
    if prefs::get().mark_read_on_open && !article.read {
        let _ = window::with_db(|db| db.set_read(&article.id, true));
        window::reload();
    }
    paint_actions();
    timeline::show_story();
    if prefs::get().summaries {
        let id = article.id.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if OPEN_ID.with(|p| p.borrow().as_deref() == Some(id.as_str())) && !RUNNING.get() {
                start_brief("short");
            }
        });
    }
}

pub fn replace_body(id: &str, text: &str) {
    let open = OPEN_ID.with(|p| p.borrow().clone());
    if open.as_deref() != Some(id) {
        return;
    }
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.set_text(if text.trim().is_empty() { UNAVAILABLE } else { text });
    }
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(false);
    }
}

pub fn unavailable(id: &str) {
    let open = OPEN_ID.with(|p| p.borrow().clone());
    if open.as_deref() != Some(id) {
        return;
    }
    let summary = window::find_article(id).map(|a| a.summary).unwrap_or_default();
    let text = html_text::plain(&summary);
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        if !text.trim().is_empty() {
            body.set_text(&text);
        } else {
            body.set_text(UNAVAILABLE);
        }
    }
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(true);
    }
}

pub fn update_font_size() {
    let scale = prefs::get().reader_font_scale;
    let font = prefs::get().reader_font;
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.remove_css_class("font-small");
        body.remove_css_class("font-medium");
        body.remove_css_class("font-large");
        match scale.as_str() {
            "small" => body.add_css_class("font-small"),
            "large" => body.add_css_class("font-large"),
            _ => body.add_css_class("font-medium"),
        }
        body.remove_css_class("font-serif");
        body.remove_css_class("font-mono");
        body.remove_css_class("font-sans");
        match font.as_str() {
            "mono" => body.add_css_class("font-mono"),
            "sans" => body.add_css_class("font-sans"),
            _ => body.add_css_class("font-serif"),
        }
    }
}

pub fn hide() {
    timeline::close_story();
}

#[allow(dead_code)]
pub fn toggle_read() {
    let Some(id) = window::selected_id() else { return };
    let read = window::find_article(&id).is_some_and(|a| a.read);
    let _ = window::with_db(|db| db.set_read(&id, !read));
    window::reload();
    paint_actions();
}

#[allow(dead_code)]
pub fn toggle_star() {
    let Some(id) = window::selected_id() else { return };
    let starred = window::find_article(&id).is_some_and(|a| a.starred);
    let _ = window::with_db(|db| db.set_starred(&id, !starred));
    window::reload();
    paint_actions();
}

pub fn toggle_save() {
    let Some(id) = window::selected_id() else { return };
    let saved = window::find_article(&id).or_else(|| window::with_db(|db| db.article(&id)).ok().flatten()).is_some_and(|a| a.saved);
    let _ = window::with_db(|db| db.set_saved(&id, !saved));
    window::toast(if saved { "Story removed from Saved" } else { "Story saved" });
    window::reload();
    paint_actions();
}

fn paint_actions() {
    let Some(id) = window::selected_id() else { return };
    let Some(article) = window::find_article(&id).or_else(|| window::with_db(|db| db.article(&id)).ok().flatten()) else {
        return;
    };
    if let Some((button, label)) = SAVE.with(|p| p.borrow().clone()) {
        label.set_text(if article.saved { "Saved" } else { "Save" });
        button.set_tooltip_text(Some(if article.saved { "Remove from Saved (B)" } else { "Save story (B)" }));
        if article.saved {
            button.add_css_class("on");
        } else {
            button.remove_css_class("on");
        }
    }
}

pub fn set_ai_visible(on: bool) {
    if !on {
        cancel_brief();
        if let Some(card) = BRIEF_CARD.with(|p| p.borrow().clone()) {
            card.set_visible(false);
        }
    }
}

fn card_title(task: &str) -> &'static str {
    match task {
        "brief" => "Today's top stories",
        "compare" => "How the sources compare",
        "spoken" => "Read aloud",
        _ => "In short",
    }
}

fn set_card(title: &str, text: &str, running: bool) {
    if let Some(label) = BRIEF_TITLE.with(|p| p.borrow().clone()) {
        label.set_text(title);
    }
    if let Some(cancel) = BRIEF_CANCEL.with(|p| p.borrow().clone()) {
        cancel.set_visible(running);
    }
    let Some(body) = BRIEF_BODY.with(|p| p.borrow().clone()) else { return };
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    if running {
        body.append(&widgets::label(text, "in-short-status"));
    } else {
        let points = nyx_news_core::points(text);
        let short = points.len() <= 5 && points.iter().all(|p| p.text.chars().count() < 220);
        if short && !points.is_empty() {
            for point in points {
                let line = match point.label {
                    Some(label) => format!("{label}: {}", point.text),
                    None => point.text,
                };
                let row = widgets::label(&format!("•  {line}"), "in-short-line");
                row.set_wrap(true);
                row.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                body.append(&row);
            }
        } else {
            let all = widgets::label(text.trim(), "in-short-line");
            all.set_wrap(true);
            all.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.append(&all);
        }
    }
    if let Some(card) = BRIEF_CARD.with(|p| p.borrow().clone()) {
        card.set_visible(true);
    }
}

fn hide_card() {
    if let Some(card) = BRIEF_CARD.with(|p| p.borrow().clone()) {
        card.set_visible(false);
    }
}

pub fn apply_brief(generation: u64, result: Result<String, AiFailure>) {
    if generation != GENERATION.get() {
        return;
    }
    RUNNING.set(false);
    let task = TASK.with(|t| t.borrow().clone());
    match result {
        Ok(text) if !text.trim().is_empty() => {
            let title = card_title(&task).to_string();
            set_card(&title, &text, false);
            BRIEF_TEXT.with(|b| *b.borrow_mut() = Some((title, text)));
        }
        Ok(_) => restore_card(),
        Err(err) => {
            restore_card();
            // A summary nobody asked for fails quietly; the story is still there.
            if task != "short" {
                window::toast(&err.message());
            }
        }
    }
}

fn restore_card() {
    match BRIEF_TEXT.with(|b| b.borrow().clone()) {
        Some((title, text)) => set_card(&title, &text, false),
        None => hide_card(),
    }
}

fn start_brief(task: &str) {
    if !prefs::get().summaries {
        return;
    }
    let Some(id) = window::selected_id() else {
        window::toast("Open a story first.");
        return;
    };
    let Some(current) = window::find_article(&id).or_else(|| window::with_db(|db| db.article(&id)).ok().flatten()) else {
        window::toast("Open a story first.");
        return;
    };
    let list = window::articles();
    let articles = match task {
        "brief" => {
            let mut picked: Vec<_> = list.iter().take(5).cloned().collect();
            if picked.is_empty() {
                picked.push(current);
            }
            picked
        }
        "compare" => {
            let Some(other) = list.iter().find(|article| article.id != current.id).cloned() else {
                window::toast("Comparing needs a second story.");
                return;
            };
            vec![current, other]
        }
        _ => vec![current],
    };
    let mut excerpts: Vec<Excerpt> = articles.iter().map(excerpt_of).collect();
    if task == "short"
        && let (Some(first), Some(body)) = (excerpts.first_mut(), BODY.with(|p| p.borrow().clone()))
    {
        let text = body.text().to_string();
        if text.len() > first.text.len() {
            first.text = text;
        }
    }
    let generation = bump_generation();
    TASK.with(|t| *t.borrow_mut() = task.to_string());
    RUNNING.set(true);
    let status = match task {
        "brief" => "Writing a brief of today's top stories…",
        "compare" => "Comparing sources…",
        "spoken" => "Writing something to read aloud…",
        _ => "Writing a short summary…",
    };
    set_card(card_title(task), status, true);
    worker::brief(task.to_string(), excerpts, generation);
}

fn cancel_brief() {
    let was_running = RUNNING.get();
    bump_generation();
    RUNNING.set(false);
    if was_running {
        assistant_client::cancel();
        restore_card();
    }
}

fn bump_generation() -> u64 {
    GENERATION.with(|generation| {
        let next = generation.get().wrapping_add(1);
        generation.set(next);
        next
    })
}

fn clear_brief() {
    let running = RUNNING.get();
    bump_generation();
    RUNNING.set(false);
    BRIEF_TEXT.with(|b| *b.borrow_mut() = None);
    hide_card();
    if running {
        assistant_client::cancel();
    }
}

fn excerpt_of(article: &ArticleListItem) -> Excerpt {
    let cached = article
        .content_hash
        .as_ref()
        .and_then(|hash| window::with_db(|db| Ok(db.body(&paths::content_cache(), hash))).ok().flatten());
    let text =
        cached.map(|html| html_text::plain(&html)).filter(|text| !text.is_empty()).unwrap_or_else(|| article.summary.clone());
    Excerpt { title: article.title.clone(), feed: article.feed_title.clone(), url: article.original_url.clone(), text }
}

fn export_note() {
    let Some(id) = window::selected_id() else {
        window::toast("Open a story first.");
        return;
    };
    let Some(article) = window::find_article(&id).or_else(|| window::with_db(|db| db.article(&id)).ok().flatten()) else {
        window::toast("Open a story first.");
        return;
    };
    let Some(parent) = window::window() else { return };
    dialogs::export_markdown(&parent, note::markdown(&article), note::filename(&article.title));
}

pub fn open_original() {
    let Some(url) = OPEN_URL.with(|p| p.borrow().clone()) else { return };
    if url.starts_with("https://") || url.starts_with("http://") {
        cmd::spawn(&["xdg-open", &url]);
    }
}
