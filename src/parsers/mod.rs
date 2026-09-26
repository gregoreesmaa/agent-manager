//! Transcript parsers: titles, projects, GitHub PR links.

pub mod github;
pub mod registry;

use crate::parsers::github::extract_pr_links;

/// Structured facts pulled out of a transcript fragment.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub title: Option<String>,
    pub project: Option<String>,
    pub pr_links: Vec<String>,
}

/// Something that can turn transcript text into [`Parsed`] facts.
pub trait Parser {
    fn parse(&self, text: &str) -> Parsed;
}

/// Shared PR-link extraction available to every parser.
pub fn pr_links(text: &str) -> Vec<String> {
    extract_pr_links(text)
}
