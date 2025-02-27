use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use chrono::Local;
use mime_guess::mime::{APPLICATION_OCTET_STREAM, IMAGE_JPEG};
use mime_guess::{Mime, get_mime_extensions};
use presage::proto::AttachmentPointer;
use regex::Regex;
use tracing::info;

use crate::signal::Attachment;
use crate::util::utc_timestamp_msec_to_local;

const DIGEST_BYTES_LEN: usize = 4;

pub(super) fn save(
    data_dir: impl AsRef<Path>,
    pointer: AttachmentPointer,
    data: &[u8],
) -> anyhow::Result<Attachment> {
    let base_dir = data_dir.as_ref().join("files");

    let digest = pointer
        .digest
        .as_deref()
        .context("dropping attachment without digest")?;
    let digest_hex = hex::encode(digest);

    let mime: Mime = pointer
        .content_type()
        .parse()
        .unwrap_or(APPLICATION_OCTET_STREAM);

    let name = derive_name(pointer.file_name, digest, &mime);

    let date = pointer
        .upload_timestamp
        .map(utc_timestamp_msec_to_local)
        .unwrap_or_else(Local::now)
        .date_naive();
    let filedir = base_dir.join(date.to_string());
    let filepath = conflict_free_filename(&filedir, name);

    std::fs::create_dir_all(&filedir)
        .with_context(|| format!("failed to create dir: {}", filedir.display()))?;
    std::fs::write(&filepath, data)
        .with_context(|| format!("failed to save attachment at: {}", filepath.display()))?;

    info!(dest =% filepath.display(), "saved attachment");

    Ok(Attachment {
        id: digest_hex,
        content_type: mime.to_string(),
        filename: filepath,
        size: pointer.size.unwrap_or_default(),
    })
}

fn conflict_free_filename(filedir: &Path, name: String) -> PathBuf {
    let mut filepath = filedir.join(&name);

    // resolve conflicts
    let mut idx = 0;
    while filepath.exists() {
        let name_path = Path::new(&name);
        match name_path.file_stem().zip(name_path.extension()) {
            Some((stem, extension)) => {
                idx += 1;
                let stem = stem.to_string_lossy();
                let extension = extension.to_string_lossy();
                filepath = filedir.join(format!("{stem}.{idx}.{extension}"));
            }
            None => {
                idx += 1;
                filepath = filedir.join(format!("{name}.{idx}"));
            }
        }
    }
    filepath
}

static INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[\x00-\x1F\x7F-\x9F<>"\s{}\^⟨⟩`]"#).unwrap());

fn derive_name(file_name: Option<String>, digest: &[u8], mime: &Mime) -> String {
    if let Some(name) = file_name {
        match INVALID_CHARS.replace_all(&name, "-") {
            Cow::Owned(name) => name,
            Cow::Borrowed(_) => name,
        }
    } else {
        let mut name = hex::encode(&digest[..DIGEST_BYTES_LEN]);
        let extension = if mime == &IMAGE_JPEG {
            // special case due to: <https://github.com/abonander/mime_guess/issues/59>
            Some("jpeg")
        } else if mime == &APPLICATION_OCTET_STREAM {
            None
        } else {
            get_mime_extensions(mime).and_then(|extensions| extensions.first().copied())
        };
        if let Some(extension) = extension {
            name.push('.');
            name.push_str(extension);
        };
        name
    }
}

#[cfg(test)]
mod tests {
    use hex_literal::hex;
    use uuid::Uuid;

    use super::*;

    fn attachment_pointer(
        content_type: &str,
        digest: &[u8],
        file_name: Option<&str>,
        upload_timestamp: u64,
    ) -> AttachmentPointer {
        AttachmentPointer {
            uuid: Some(Uuid::nil().into_bytes().to_vec()),
            content_type: Some(content_type.into()),
            digest: Some(digest.into()),
            file_name: file_name.map(|s| s.to_owned()),
            upload_timestamp: Some(upload_timestamp),
            key: None,
            size: Some(42),
            thumbnail: None,
            incremental_mac: None,
            incremental_mac_chunk_size: None,
            flags: None,
            width: None,
            height: None,
            caption: None,
            blur_hash: None,
            cdn_number: None,
            attachment_identifier: None,
        }
    }

    #[test]
    fn test_save() {
        let tempdir = tempfile::tempdir().unwrap();

        let digest = hex!("d51e9a355d4351ae5fbf2846d18bb384471555aa0ea6ee9075eb63f99ecddf77");
        let upload_timestamp = 1703160458 * 1000;

        let attachment = save(
            tempdir.path(),
            attachment_pointer("image/jpeg", &digest, Some("image.jpeg"), upload_timestamp),
            &[42],
        )
        .unwrap();

        assert_eq!(attachment.id, hex::encode(digest));
        assert_eq!(attachment.content_type, "image/jpeg");
        assert_eq!(attachment.size, 42);
        assert_eq!(
            attachment.filename,
            tempdir.path().join("files/2023-12-21/image.jpeg")
        );

        assert_eq!(std::fs::read(attachment.filename).unwrap(), &[42]);

        // duplicate
        let attachment = save(
            tempdir.path(),
            attachment_pointer("image/jpeg", &digest, Some("image.jpeg"), upload_timestamp),
            &[42],
        )
        .unwrap();
        assert_eq!(
            attachment.filename,
            tempdir.path().join("files/2023-12-21/image.1.jpeg")
        );

        // without name
        let attachment = save(
            tempdir.path(),
            attachment_pointer("image/jpeg", &digest, None, upload_timestamp),
            &[42],
        )
        .unwrap();
        assert_eq!(
            attachment.filename,
            tempdir.path().join("files/2023-12-21/d51e9a35.jpeg")
        );

        // without name and mime octet-stream
        let attachment = save(
            tempdir.path(),
            attachment_pointer("application/octet-stream", &digest, None, upload_timestamp),
            &[42],
        )
        .unwrap();
        assert_eq!(
            attachment.filename,
            tempdir.path().join("files/2023-12-21/d51e9a35")
        );

        // without name and mime pdf
        let attachment = save(
            tempdir.path(),
            attachment_pointer("application/pdf", &digest, None, upload_timestamp),
            &[42],
        )
        .unwrap();
        assert_eq!(
            attachment.filename,
            tempdir.path().join("files/2023-12-21/d51e9a35.pdf")
        );
    }

    #[test]
    fn test_derive_name() {
        assert_eq!(
            derive_name(
                Some("Screenshot 2000-00-00 at 12.00.00.png".to_owned()),
                &[],
                &IMAGE_JPEG,
            ),
            "Screenshot-2000-00-00-at-12.00.00.png"
        );
    }
}
