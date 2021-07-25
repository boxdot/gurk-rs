mod changelog;
mod dist;
mod flags;

use anyhow::Result;
use std::env;
use std::path::{Path, PathBuf};
use xshell::pushd;

fn main() -> Result<()> {
    let _d = pushd(project_root())?;

    let flags = flags::Xtask::from_env()?;
    match flags.subcommand {
        flags::XtaskCmd::Help(_) => {
            println!("{}", flags::Xtask::HELP);
            Ok(())
        }
        flags::XtaskCmd::Dist(cmd) => cmd.run(),
        flags::XtaskCmd::Changelog(cmd) => cmd.run(),
    }
}

fn project_root() -> PathBuf {
    Path::new(
        &env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned()),
    )
    .ancestors()
    .nth(1)
    .unwrap()
    .to_path_buf()
}
