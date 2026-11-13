//! Optional briefs from Nyx Assistant. This crate does not load a model.
//!
//! A failure returns the article text unchanged. Clustering from the reader
//! does not call this module.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub title: String,
    pub feed: String,
    pub url: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiFailure {
    /// The Assistant socket could not be opened.
    Down,
    Failed(String),
    TimedOut,
}

impl AiFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Down => "Assistant isn’t running".into(),
            Self::Failed(text) => text.clone(),
            Self::TimedOut => "Assistant took too long.".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub article: String,
    pub brief: Option<String>,
    pub toast: Option<String>,
}

pub trait Assistant {
    fn complete(&mut self, prompt: &str) -> Result<String, AiFailure>;
    fn speak(&mut self, script: &str) -> Result<(), AiFailure>;
}

/// Instructions plus a short excerpt. The model is told to write something original and to keep the source URL.
pub fn prompt(task: &str, articles: &[Excerpt]) -> String {
    let mut out = String::from("Write an original brief. Do not reproduce the article. Include the source URL.\n");
    out.push_str(match task {
        "brief" => "Write one brief that covers every article.\n",
        "compare" => "Compare these sources. Say where they agree and where they differ.\n",
        "spoken" => "Write a short script that can be read aloud.\n",
        "daily" => "Write three lines, one for each of the first three stories. Start each line with a label of two or three words and a colon, then one sentence.\n",
        "short" => "Summarize this article as three short points, one per line.\n",
        _ => "Summarize this article.\n",
    });
    for article in articles.iter().take(5) {
        let excerpt: String = article.text.chars().take(800).collect();
        out.push_str(&format!(
            "Title: {}\nFeed: {}\nSource: {}\nExcerpt: {excerpt}\n\n",
            article.title, article.feed, article.url
        ));
    }
    out
}

pub fn ask(client: &mut impl Assistant, task: &str, articles: &[Excerpt]) -> Result<String, AiFailure> {
    if articles.is_empty() {
        return Err(AiFailure::Failed("There is no article to brief.".into()));
    }
    let written = client.complete(&prompt(task, articles))?;
    let written = written.trim().to_string();
    if written.is_empty() {
        return Err(AiFailure::Failed("Assistant returned nothing.".into()));
    }
    if task == "spoken" {
        client.speak(&written)?;
    }
    Ok(written)
}

/// Keep the article text when Assistant fails. A successful brief is separate from that text.
pub fn finish(article: &str, current_brief: Option<&str>, result: Result<String, AiFailure>) -> Pane {
    match result {
        Ok(brief) => Pane { article: article.to_string(), brief: Some(brief), toast: None },
        Err(err) => Pane { article: article.to_string(), brief: current_brief.map(str::to_string), toast: Some(err.message()) },
    }
}

/// One line of a brief, with its leading "Label:" kept apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    pub label: Option<String>,
    pub text: String,
}

/// A brief as short lines for the page: bullets, numbering, bold markers, links and
/// source lines taken off.
pub fn points(text: &str) -> Vec<Point> {
    text.lines().filter_map(point).collect()
}

fn point(line: &str) -> Option<Point> {
    let mut line = line.trim().trim_start_matches(['-', '*', '•', '·', '#']).trim_start();
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && matches!(line[digits..].chars().next(), Some('.' | ')')) {
        line = line[digits + 1..].trim_start();
    }
    let unmarked = line.replace("**", "").replace("__", "");
    let words: Vec<&str> = unmarked
        .split_whitespace()
        .filter(|word| !word.trim_start_matches(['(', '[', '<']).starts_with("http"))
        .collect();
    let joined = words.join(" ").replace("()", "").replace("[]", "");
    let text = joined.trim().trim_end_matches([' ', '-', '—', '(']);
    let lower = text.to_lowercase();
    if text.is_empty() || text.ends_with(':') || lower.starts_with("source") {
        return None;
    }
    if let Some((head, rest)) = text.split_once(':') {
        let head = head.trim();
        if !rest.trim().is_empty() && head.split_whitespace().count() <= 4 && head.chars().count() <= 40 {
            return Some(Point { label: Some(head.to_string()), text: rest.trim().to_string() });
        }
    }
    Some(Point { label: None, text: text.to_string() })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    Done(String),
    Failed(AiFailure),
}

/// One IPC line from Assistant. Callers stop on [`Step::Done`] or [`Step::Failed`].
pub fn interpret(line: &str) -> Step {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Step::Continue;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("error") => {
            let message = value.get("message").and_then(Value::as_str).unwrap_or("Assistant failed.");
            Step::Failed(AiFailure::Failed(message.to_string()))
        }
        Some("assistant_text") if value.get("done").and_then(Value::as_bool) == Some(true) => {
            Step::Done(value.get("text").and_then(Value::as_str).unwrap_or("").to_string())
        }
        _ => Step::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        mode: &'static str,
        prompt: RefCell<String>,
        spoke: RefCell<Option<String>>,
    }

    impl Assistant for Fake {
        fn complete(&mut self, prompt: &str) -> Result<String, AiFailure> {
            *self.prompt.borrow_mut() = prompt.to_string();
            match self.mode {
                "text" => Ok("A short original brief. Source: https://example.com/story".into()),
                "empty" => Ok("  ".into()),
                "down" => Err(AiFailure::Down),
                "timeout" => Err(AiFailure::TimedOut),
                _ => Err(AiFailure::Failed("the model stopped".into())),
            }
        }

        fn speak(&mut self, script: &str) -> Result<(), AiFailure> {
            *self.spoke.borrow_mut() = Some(script.to_string());
            Ok(())
        }
    }

    fn story() -> Excerpt {
        Excerpt {
            title: "Ship lands".into(),
            feed: "Example".into(),
            url: "https://example.com/story".into(),
            text: "The feed's own wording stays in the excerpt field.".into(),
        }
    }

    #[test]
    fn a_reply_keeps_the_article_and_asks_for_an_original_brief() {
        let article = "The feed's own wording stays on screen.";
        let mut client = Fake { mode: "text", prompt: RefCell::new(String::new()), spoke: RefCell::new(None) };
        let result = ask(&mut client, "summary", &[story()]);
        let pane = finish(article, None, result);
        assert_eq!(pane.article, article);
        assert!(pane.brief.unwrap().contains("https://example.com/story"));
        assert!(pane.toast.is_none());
        let prompt = client.prompt.borrow().clone();
        assert!(prompt.contains("original brief"));
        assert!(prompt.contains("https://example.com/story"));
        assert!(prompt.contains("The feed's own wording"));
        assert!(client.spoke.borrow().is_none());
    }

    #[test]
    fn an_error_leaves_the_article_and_the_previous_brief() {
        let article = "Still readable.";
        let mut client = Fake { mode: "error", prompt: RefCell::new(String::new()), spoke: RefCell::new(None) };
        let pane = finish(article, Some("Earlier brief"), ask(&mut client, "summary", &[story()]));
        assert_eq!(pane.article, article);
        assert_eq!(pane.brief.as_deref(), Some("Earlier brief"));
        assert_eq!(pane.toast.as_deref(), Some("the model stopped"));
    }

    #[test]
    fn a_timeout_leaves_the_article() {
        let article = "Still readable.";
        let mut client = Fake { mode: "timeout", prompt: RefCell::new(String::new()), spoke: RefCell::new(None) };
        let pane = finish(article, None, ask(&mut client, "compare", &[story(), story()]));
        assert_eq!(pane.article, article);
        assert!(pane.brief.is_none());
        assert_eq!(pane.toast.as_deref(), Some("Assistant took too long."));
    }

    #[test]
    fn a_missing_daemon_is_named() {
        let mut client = Fake { mode: "down", prompt: RefCell::new(String::new()), spoke: RefCell::new(None) };
        let err = ask(&mut client, "summary", &[story()]).unwrap_err();
        assert_eq!(err.message(), "Assistant isn’t running");
    }

    #[test]
    fn a_spoken_brief_is_sent_to_speech_after_the_text_exists() {
        let mut client = Fake { mode: "text", prompt: RefCell::new(String::new()), spoke: RefCell::new(None) };
        let text = ask(&mut client, "spoken", &[story()]).unwrap();
        assert_eq!(client.spoke.borrow().as_deref(), Some(text.as_str()));
    }

    #[test]
    fn a_brief_becomes_labelled_lines_without_links() {
        let text = "Here are today's stories:\n\n- **Ocean plastics:** 140 countries agree to halve packaging (https://example.com/a).\n2. Rates: the bank holds steady.\n• A reef nursery bred hardier coral.\nSource: https://example.com/c\nhttps://example.com/d";
        assert_eq!(
            points(text),
            vec![
                Point { label: Some("Ocean plastics".into()), text: "140 countries agree to halve packaging".into() },
                Point { label: Some("Rates".into()), text: "the bank holds steady.".into() },
                Point { label: None, text: "A reef nursery bred hardier coral.".into() },
            ]
        );
    }

    #[test]
    fn completion_lines_stop_on_text_or_an_error() {
        assert_eq!(interpret("not json"), Step::Continue);
        assert_eq!(interpret(r#"{"type":"assistant_text","text":"partial","done":false}"#), Step::Continue);
        assert_eq!(interpret(r#"{"type":"assistant_text","text":"done text","done":true}"#), Step::Done("done text".into()));
        assert_eq!(interpret(r#"{"type":"error","message":"busy"}"#), Step::Failed(AiFailure::Failed("busy".into())));
    }
}
