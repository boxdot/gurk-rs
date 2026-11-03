//! Inline image previews for image attachments
//!
//! Images are decoded and downscaled one at a time in a background thread, so that opening a chat
//! with many photos does not hold all of them decoded in memory at once. The renderer only encodes
//! the small thumbnail for the terminal graphics protocol detected by [`Picker`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};

use image::{DynamicImage, ImageDecoder, ImageReader};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Clear;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};
use tokio::sync::mpsc;
use tracing::warn;

use crate::event::Event;

/// Thumbnails are never wider than this, independent of the terminal size
const MAX_THUMBNAIL_WIDTH_PX: u32 = 2048;

enum Decoded {
    Loading,
    Failed,
    Ready(DynamicImage),
}

/// Key of an encoded image: path and the maximum size in cells it was fitted into
type ProtocolKey = (PathBuf, u16, u16);

pub struct ImageCache {
    picker: Option<Picker>,
    /// Maximum height of an image in lines; 0 disables images
    max_height: u16,
    decoded: Arc<Mutex<HashMap<PathBuf, Decoded>>>,
    protocols: RefCell<HashMap<ProtocolKey, Option<Protocol>>>,
    /// Width of the messages view the encoded images were fitted into
    width: Cell<u16>,
    /// Queue of the decoding thread; set together with the picker
    decode_tx: Option<std_mpsc::Sender<PathBuf>>,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl ImageCache {
    pub fn new(max_height: u16, event_tx: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            picker: None,
            max_height,
            decoded: Default::default(),
            protocols: Default::default(),
            width: Cell::new(0),
            decode_tx: None,
            event_tx,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_height > 0
    }

    /// Enables rendering with the protocol and font size detected by `picker`
    pub fn set_picker(&mut self, picker: Picker) {
        let max_height_px = u32::from(self.max_height) * u32::from(picker.font_size().height);
        self.decode_tx = Some(spawn_decoder(
            max_height_px,
            self.decoded.clone(),
            self.event_tx.clone(),
        ));
        self.picker = Some(picker);
        self.protocols.get_mut().clear();
    }

    /// Size in cells of the image at `path` fitted into `max_width` x `available_height`
    ///
    /// Returns `None` if images are disabled, or the image is still loading or failed to load.
    /// The first call for a path starts loading the image in the background; when it is loaded,
    /// an [`Event::ImageLoaded`] is sent.
    pub fn size(&self, path: &Path, max_width: u16, available_height: u16) -> Option<Size> {
        let picker = self.picker.as_ref()?;
        if self.width.replace(max_width) != max_width {
            // the view was resized: images encoded for the old width are not used anymore
            self.protocols.borrow_mut().clear();
        }
        let key = self.key(path, max_width, available_height);
        let (max_width, max_height) = (key.1, key.2);
        if max_width == 0 || max_height == 0 {
            return None;
        }

        if let Some(protocol) = self.protocols.borrow().get(&key) {
            return protocol.as_ref().map(|p| p.size());
        }

        let image = {
            let mut decoded = self.decoded.lock().unwrap();
            match decoded.get(path) {
                Some(Decoded::Ready(image)) => image.clone(),
                Some(Decoded::Loading | Decoded::Failed) => return None,
                None => {
                    decoded.insert(path.to_owned(), Decoded::Loading);
                    if let Some(decode_tx) = &self.decode_tx {
                        let _ = decode_tx.send(path.to_owned());
                    }
                    return None;
                }
            }
        };

        let protocol = picker
            .new_protocol(image, Size::new(max_width, max_height), Resize::Fit(None))
            .inspect_err(|error| warn!(path = %path.display(), %error, "failed to encode image"))
            .ok();
        let size = protocol.as_ref().map(|p| p.size());
        self.protocols.borrow_mut().insert(key, protocol);
        size
    }

    /// Renders the image previously sized with [`Self::size`] with the same arguments into `area`
    pub fn render(
        &self,
        f: &mut Frame,
        path: &Path,
        max_width: u16,
        available_height: u16,
        area: Rect,
    ) {
        let key = self.key(path, max_width, available_height);
        let protocols = self.protocols.borrow();
        if let Some(Some(protocol)) = protocols.get(&key) {
            // reset styles of the cells (e.g. highlight of the selected message)
            f.render_widget(Clear, area);
            f.render_widget(Image::new(protocol), area);
        }
    }

    /// Drops the encoded images, e.g. when the width of the messages view changes
    pub fn clear_encoded(&mut self) {
        self.protocols.get_mut().clear();
    }

    fn key(&self, path: &Path, max_width: u16, available_height: u16) -> ProtocolKey {
        let max_height = self.max_height.min(available_height);
        (path.to_owned(), max_width, max_height)
    }
}

/// Spawns the thread decoding the queued images into thumbnails
///
/// The thread stops when the returned sender is dropped.
fn spawn_decoder(
    max_height_px: u32,
    decoded: Arc<Mutex<HashMap<PathBuf, Decoded>>>,
    event_tx: mpsc::UnboundedSender<Event>,
) -> std_mpsc::Sender<PathBuf> {
    let (tx, rx) = std_mpsc::channel::<PathBuf>();
    std::thread::spawn(move || {
        for path in rx {
            let result = match decode(&path) {
                Ok(image) => Decoded::Ready(downscale(image, max_height_px)),
                Err(error) => {
                    warn!(path = %path.display(), %error, "failed to decode image");
                    Decoded::Failed
                }
            };
            decoded.lock().unwrap().insert(path, result);
            let _ = event_tx.send(Event::ImageLoaded);
        }
    });
    tx
}

/// Shrinks the image to fit the thumbnail bounds; never enlarges it
fn downscale(image: DynamicImage, max_height_px: u32) -> DynamicImage {
    if MAX_THUMBNAIL_WIDTH_PX < image.width() || max_height_px < image.height() {
        image.thumbnail(MAX_THUMBNAIL_WIDTH_PX, max_height_px)
    } else {
        image
    }
}

fn decode(path: &Path) -> anyhow::Result<DynamicImage> {
    let mut decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn test_load_size_and_render() {
        let tempdir = tempfile::tempdir().unwrap();
        // no extension: the format is guessed from the content
        let path = tempdir.path().join("attachment");
        RgbImage::from_pixel(400, 400, Rgb([255, 0, 0]))
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        assert_eq!(cache.size(&path, 40, 20), None, "no picker yet");

        cache.set_picker(Picker::halfblocks()); // 10x20px font
        assert_eq!(cache.size(&path, 40, 20), None, "loading");
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));

        // square image: 10 lines of 20px are 200px, which are 20 columns of 10px
        assert_eq!(cache.size(&path, 40, 20), Some(Size::new(20, 10)));
        // capped by the available height
        assert_eq!(cache.size(&path, 40, 5), Some(Size::new(10, 5)));

        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| cache.render(f, &path, 40, 20, Rect::new(0, 1, 20, 10)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let red = Color::Rgb(255, 0, 0);
        assert_eq!(buffer[(0, 1)].bg, red, "image drawn");
        assert_eq!(buffer[(19, 10)].bg, red, "image drawn");
        assert_ne!(buffer[(0, 0)].bg, red, "nothing drawn above");
        assert_ne!(buffer[(20, 5)].bg, red, "nothing drawn right of it");
    }

    #[test]
    fn test_several_images_and_resize() {
        let tempdir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..5)
            .map(|i| {
                let path = tempdir.path().join(format!("{i}.png"));
                RgbImage::from_pixel(100, 100, Rgb([0, 0, 255]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        for path in &paths {
            assert_eq!(cache.size(path, 40, 20), None);
        }
        for _ in &paths {
            assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        }
        for path in &paths {
            assert_eq!(cache.size(path, 40, 20), Some(Size::new(10, 5)));
        }
        assert_eq!(cache.protocols.borrow().len(), 5);

        // narrower view: images are re-encoded and the old encodings dropped
        assert_eq!(cache.size(&paths[0], 8, 20), Some(Size::new(8, 4)));
        assert_eq!(cache.protocols.borrow().len(), 1);
    }

    #[test]
    fn test_failed_image() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("not-an-image");
        std::fs::write(&path, b"hello").unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks()); // 10x20px font
        assert_eq!(cache.size(&path, 40, 20), None);
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        assert_eq!(cache.size(&path, 40, 20), None);
    }

    #[test]
    fn test_disabled() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let cache = ImageCache::new(0, tx);
        assert!(!cache.is_enabled());
    }
}
