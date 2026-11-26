use regex::Regex;
use std::path::Path;
use url::Url;

use crate::contracts::RoutingRule;

pub fn resolve_save_dir(
    raw_url: &str,
    filename: &str,
    rules: &[RoutingRule],
    default_dir: &str,
) -> String {
    let host = Url::parse(raw_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default();

    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    for rule in rules {
        if rule.target_dir.trim().is_empty() {
            continue;
        }
        let has_domain = rule
            .domain_pattern
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty());
        let has_regex = rule
            .filename_regex
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty());
        let has_ext = !rule.extensions.is_empty();

        if !has_domain && !has_regex && !has_ext {
            continue;
        }

        if let Some(pattern) = rule.domain_pattern.as_deref().filter(|s| !s.trim().is_empty()) {
            if !domain_matches(&host, pattern) {
                continue;
            }
        }

        if let Some(re_str) = rule.filename_regex.as_deref().filter(|s| !s.trim().is_empty()) {
            let Ok(re) = Regex::new(re_str) else {
                continue;
            };
            if !re.is_match(filename) {
                continue;
            }
        }

        if has_ext {
            let ext_matched = rule
                .extensions
                .iter()
                .any(|e| e.trim().trim_start_matches('.').eq_ignore_ascii_case(&ext));
            if !ext_matched {
                continue;
            }
        }

        return rule.target_dir.clone();
    }

    default_dir.to_string()
}

fn domain_matches(host: &str, pattern: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let pat = pattern.trim().to_ascii_lowercase();
    if let Some(suffix) = pat.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pat || host.ends_with(&format!(".{pat}")) || host.contains(&pat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rules() -> Vec<RoutingRule> {
        vec![
            RoutingRule {
                name: "GitHub Releases".to_string(),
                domain_pattern: Some("*.github.com".to_string()),
                filename_regex: Some(r"(?i).*windows.*\.zip$".to_string()),
                extensions: vec!["zip".to_string()],
                target_dir: r"D:\Downloads\GitHubBuilds".to_string(),
            },
            RoutingRule {
                name: "ISOs".to_string(),
                domain_pattern: None,
                filename_regex: Some(r"(?i)^ubuntu-.*".to_string()),
                extensions: vec!["iso".to_string()],
                target_dir: r"D:\Downloads\Linux".to_string(),
            },
            RoutingRule {
                name: "Archives".to_string(),
                domain_pattern: None,
                filename_regex: None,
                extensions: vec!["zip".to_string(), "7z".to_string()],
                target_dir: r"D:\Downloads\Archives".to_string(),
            },
        ]
    }

    #[test]
    fn routes_by_domain_regex_and_extension() {
        let rules = sample_rules();
        let dir = resolve_save_dir(
            "https://objects.githubusercontent.com/app-windows-x64.zip",
            "app-windows-x64.zip",
            &rules,
            r"D:\Downloads",
        );
        // Host does not match *.github.com, falls through to Archives rule
        assert_eq!(dir, r"D:\Downloads\Archives");

        let gh_dir = resolve_save_dir(
            "https://release-assets.github.com/tool-windows.zip",
            "tool-windows.zip",
            &rules,
            r"D:\Downloads",
        );
        assert_eq!(gh_dir, r"D:\Downloads\GitHubBuilds");
    }

    #[test]
    fn routes_by_regex_and_falls_back_to_default() {
        let rules = sample_rules();
        let iso_dir = resolve_save_dir(
            "https://releases.ubuntu.com/24.04/ubuntu-24.04-desktop-amd64.iso",
            "ubuntu-24.04-desktop-amd64.iso",
            &rules,
            r"D:\Downloads",
        );
        assert_eq!(iso_dir, r"D:\Downloads\Linux");

        let fallback = resolve_save_dir(
            "https://example.com/readme.txt",
            "readme.txt",
            &rules,
            r"D:\Downloads",
        );
        assert_eq!(fallback, r"D:\Downloads");
    }
}
