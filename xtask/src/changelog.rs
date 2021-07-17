use anyhow::{anyhow, bail, Result};
use pulldown_cmark::{Event, Parser, Tag};
use semver::Version;
use xshell::{mkdir_p, read_file, rm_rf, write_file};

use crate::{flags, project_root};

impl flags::Changelog {
    pub(crate) fn run(self) -> Result<()> {
        let dist = project_root().join("dist");
        rm_rf(&dist)?;
        mkdir_p(&dist)?;

        let version = get_version()?;

        let changelog = read_file("CHANGELOG.md")?;
        let section = extract_section(&changelog, version)?;

        write_file(dist.join("CHANGELOG.md"), section.trim())?;
        Ok(())
    }
}

fn get_version() -> Result<Version> {
    let github_ref = std::env::var("GITHUB_REF")
        .map_err(|_| anyhow!("missing GITHUB_REF; not running in GitHub actions?"))?;
    github_ref
        .strip_prefix("refs/tags/v")
        .and_then(|s| Version::parse(s).ok())
        .ok_or_else(|| {
            anyhow!(
                "failed to extract version from '{}'; not running for a tag?",
                github_ref
            )
        })
}

fn extract_section(changelog: &str, version: Version) -> Result<&str> {
    let parser = Parser::new(&changelog);
    let h2 = parser.into_offset_iter().filter_map(|(event, range)| {
        if let Event::Start(Tag::Heading(2)) = event {
            Some(range)
        } else {
            None
        }
    });

    let version_str = version.to_string();
    let mut it = h2.skip_while(|range| !changelog[range.clone()].contains(&version_str));

    Ok(match (it.next(), it.next()) {
        (Some(start), Some(end)) => &changelog[start.end..end.start],
        (Some(start), None) => &changelog[start.end..],
        (None, _) => {
            bail!(
                "no h2 entry in CHANGELOG for version '{}'; changelog section missing?",
                version
            );
        }
    })
}
