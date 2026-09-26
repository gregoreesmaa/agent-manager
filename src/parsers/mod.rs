//! Transcript parsers: titles, projects, GitHub + file links.

pub mod github;
pub mod refs;
pub mod registry;

use std::collections::HashSet;

use crate::parsers::github::extract_pr_links;

/// Structured facts pulled out of a transcript fragment.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub title: Option<String>,
    pub project: Option<String>,
    pub pr_links: Vec<String>,
    /// Non-PR references (issues, commits, file refs) sharing the same
    /// click-to-copy row treatment as PR links.
    pub related_links: Vec<String>,
}

/// Something that can turn transcript text into [`Parsed`] facts.
pub trait Parser {
    fn parse(&self, text: &str) -> Parsed;
}

/// Shared PR-link extraction available to every parser.
pub fn pr_links(text: &str) -> Vec<String> {
    extract_pr_links(text)
}

/// Shared non-PR references (issues, commits, file refs) in first-seen
/// order, deduplicated across kinds.
pub fn related_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for link in crate::parsers::github::extract_issue_links(text)
        .into_iter()
        .chain(crate::parsers::github::extract_commit_links(text))
        .chain(crate::parsers::refs::extract_file_refs(text))
    {
        if seen.insert(link.clone()) {
            out.push(link);
        }
    }
    out
}
