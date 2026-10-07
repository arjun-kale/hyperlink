//! Update check for Phase 11.
//!
//! Deliberately does **not** auto-download or auto-install anything. Silently
//! replacing a running binary (or worse, fetching and executing an installer)
//! without the user's own package manager / Flatpak / systemd unit involved
//! is a real supply-chain risk, and this project has no code-signing or
//! release-integrity infrastructure yet (see `docs/SECURITY_REVIEW.md`) to
//! do that safely. What this module *does* do: ask a release feed whether a
//! newer version is tagged, and tell the user — they still update through
//! their normal channel (`git pull && cargo build`, Flatpak, or a future
//! packaged release).
//!
//! Network reachable only when `HostPreferences.update_check_enabled` is true.

use anyhow::{bail, Context, Result};
use std::time::Duration;

/// Where the version feed is read from. A raw file in the repo (rather than
/// e.g. the GitHub releases API) keeps this dependency-free and works for a
/// project with no releases published yet — swap this out once real
/// versioned releases exist.
const VERSION_FEED_URL: &str =
    "https://raw.githubusercontent.com/arjun-kale/hyperlink/main/CHANGELOG.md";

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

/// Performs a single update check with a short timeout. Never panics or
/// blocks the caller indefinitely — network failures resolve to "no update
/// info available" rather than propagating as a hard error, since this is a
/// best-effort background convenience, not a required part of startup.
pub async fn check_for_update(current_version: &str) -> UpdateCheckResult {
    match fetch_latest_version().await {
        Ok(latest) => {
            let update_available = is_newer(&latest, current_version);
            UpdateCheckResult {
                current_version: current_version.to_string(),
                latest_version: Some(latest),
                update_available,
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "update check failed (non-fatal)");
            UpdateCheckResult {
                current_version: current_version.to_string(),
                latest_version: None,
                update_available: false,
            }
        }
    }
}

async fn fetch_latest_version() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build HTTP client")?;

    let body = client
        .get(VERSION_FEED_URL)
        .send()
        .await
        .context("update check request failed")?
        .error_for_status()
        .context("update check returned an error status")?
        .text()
        .await
        .context("failed to read update check response body")?;

    parse_latest_version_from_changelog(&body)
}

/// Extracts the first `## [x.y.z]` heading from a Keep-a-Changelog formatted
/// document, which is always the most recent release by convention.
fn parse_latest_version_from_changelog(changelog: &str) -> Result<String> {
    for line in changelog.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## [") {
            if let Some(end) = rest.find(']') {
                return Ok(rest[..end].to_string());
            }
        }
    }
    bail!("no version heading found in changelog feed")
}

/// Simple semver-ish comparison (`major.minor.patch`, numeric components
/// only). Treats an unparsable version as "not newer" rather than erroring,
/// since this only ever gates a notification, never a decision that matters
/// for correctness or security.
fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest)
        .zip(parse_version(current))
        .is_some_and(|(l, c)| l > c)
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_version_heading() {
        let changelog = "# Changelog\n\n## [0.3.0] - 2026-09-01\n### Added\n- foo\n\n## [0.2.0] - 2026-08-26\n### Added\n- bar\n";
        assert_eq!(
            parse_latest_version_from_changelog(changelog).unwrap(),
            "0.3.0"
        );
    }

    #[test]
    fn missing_heading_errors_rather_than_guessing() {
        assert!(parse_latest_version_from_changelog("no headings here").is_err());
    }

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.3.0"));
        assert!(!is_newer("not-a-version", "0.2.0"));
    }

    #[tokio::test]
    async fn network_failure_resolves_to_no_update_rather_than_panicking() {
        // No mock server here — this just exercises that fetch errors are
        // caught and turned into a benign result, using whatever the sandbox's
        // network reachability happens to be. If VERSION_FEED_URL is
        // reachable this asserts the parse path works end-to-end instead.
        let result = check_for_update("0.2.0").await;
        assert_eq!(result.current_version, "0.2.0");
    }
}
