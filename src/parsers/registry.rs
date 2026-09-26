//! Parser registry: fan-out over parser strategies.

use crate::parsers::{pr_links, Parsed, Parser};

/// Tries a title heuristic (first non-empty line) plus shared PR extraction.
/// Additional strategies can be pushed into `strategies` later.
pub struct RegistryParser {
    strategies: Vec<Box<dyn Parser>>,
}

impl Default for RegistryParser {
    fn default() -> Self {
        Self {
            strategies: vec![Box::new(FirstLineTitle)],
        }
    }
}

impl RegistryParser {
    pub fn with_strategies(strategies: Vec<Box<dyn Parser>>) -> Self {
        Self { strategies }
    }
}

impl Parser for RegistryParser {
    fn parse(&self, text: &str) -> Parsed {
        let mut merged = Parsed {
            pr_links: pr_links(text),
            ..Default::default()
        };
        for s in &self.strategies {
            let p = s.parse(text);
            if merged.title.is_none() {
                merged.title = p.title;
            }
            if merged.project.is_none() {
                merged.project = p.project;
            }
            for link in p.pr_links {
                if !merged.pr_links.contains(&link) {
                    merged.pr_links.push(link);
                }
            }
        }
        merged
    }
}

/// Title = first non-empty line of the transcript fragment.
struct FirstLineTitle;

impl Parser for FirstLineTitle {
    fn parse(&self, text: &str) -> Parsed {
        let title = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_string);
        Parsed {
            title,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_title_and_pr_links() {
        let p =
            RegistryParser::default().parse("Fix login\nsee https://github.com/acme/app/pull/9");
        assert_eq!(p.title.as_deref(), Some("Fix login"));
        assert_eq!(p.pr_links, vec!["https://github.com/acme/app/pull/9"]);
    }
}
