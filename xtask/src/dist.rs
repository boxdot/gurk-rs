use std::{env, fs::File, path::Path};

use anyhow::Result;
use flate2::{write::GzEncoder, Compression};
use xshell::{cmd, Shell};

use crate::{flags, project_root};

impl flags::Dist {
    pub(crate) fn run(self, sh: &Shell) -> Result<()> {
        let dist = project_root().join("dist");
        sh.remove_path(&dist)?;
        sh.create_dir(&dist)?;

        build(sh)?;
        Ok(())
    }
}

fn build(sh: &Shell) -> Result<()> {
    let target = get_target();
    cmd!(sh, "cargo build --target {target} --release --locked").run()?;

    let suffix = exe_suffix(&target);
    let src = Path::new("target")
        .join(&target)
        .join("release")
        .join(format!("gurk{}", suffix));
    let dst = Path::new("dist").join(format!("gurk-{}{}", target, suffix));
    targzip(&src, &dst.with_extension("tar.gz"))?;

    Ok(())
}

fn get_target() -> String {
    match env::var("GURK_TARGET") {
        Ok(target) => target,
        _ => {
            if cfg!(target_os = "linux") {
                "x86_64-unknown-linux-gnu".to_string()
            } else if cfg!(target_os = "macos") {
                "x86_64-apple-darwin".to_string()
            } else {
                panic!("Unsupported OS")
            }
        }
    }
}

fn exe_suffix(target: &str) -> &'static str {
    if target.contains("-windows-") {
        ".exe"
    } else {
        ""
    }
}

fn targzip(src_path: &Path, dest_path: &Path) -> Result<()> {
    let tar_gz = File::create(dest_path)?;
    let enc = GzEncoder::new(tar_gz, Compression::best());
    let mut tar = tar::Builder::new(enc);
    tar.append_path_with_name(src_path, src_path.file_name().unwrap())?;
    Ok(())
}
