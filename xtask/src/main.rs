mod changelog;
mod dist;
mod flags;

use anyhow::Result;
use std::env;
use std::path::{Path, PathBuf};
use xshell::Shell;

fn main() -> Result<()> {
    let sh = Shell::new()?;
    let _d = sh.push_dir(project_root());

    let flags = flags::Xtask::from_env_or_exit();
    match flags.subcommand {
        flags::XtaskCmd::Dist(cmd) => cmd.run(&sh),
        flags::XtaskCmd::Changelog(cmd) => cmd.run(&sh),
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
