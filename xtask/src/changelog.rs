use anyhow::{anyhow, bail, Result};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag};
use semver::Version;
use xshell::Shell;

use crate::{flags, project_root};

impl flags::Changelog {
    pub(crate) fn run(self, sh: &Shell) -> Result<()> {
        let dist = project_root().join("dist");
        sh.remove_path(&dist)?;
        sh.create_dir(&dist)?;

        let version = get_version()?;

        let changelog = sh.read_file("CHANGELOG.md")?;
        let section = extract_section(&changelog, version)?;

        sh.write_file(dist.join("CHANGELOG.md"), section.trim())?;
        Ok(())
    }
}

fn get_version() -> Result<Version> {
    let github_ref = std::env::var("GITHUB_REF")
        .map_err(|_| anyhow!("missing GITHUB_REF; not running in GitHub actions?"))?;
    github_ref
        .strip_prefix("refs/tags/v")
        .map(|s| Version::parse(s))
        .transpose()?
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
        if let Event::Start(Tag::Heading(HeadingLevel::H2, _, _)) = event {
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
