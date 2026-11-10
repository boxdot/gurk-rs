//! Inline image previews for image attachments
//!
//! The size of an image is computed from its header only, so the layout of the messages is final
//! from the first frame on. Decoding, downscaling and encoding for the terminal graphics protocol
//! detected by [`Picker`] happen in a background thread, one image at a time, and only for images
//! that are actually on screen. Until an image is ready, a placeholder is drawn in its place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Clear, Paragraph};
use ratatui_image::picker::cap_parser::Parser;
use ratatui_image::picker::{Capability, Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::protocol::kitty::Kitty;
use ratatui_image::{FontSize, Image, Resize};
use tokio::sync::mpsc;
use tracing::warn;

use crate::event::Event;

/// Maximum number of encoded images kept in memory
///
/// Images that were not drawn recently are dropped first. An encoded image is a thumbnail of at
/// most a few hundred KB.
const MAX_ENTRIES: usize = 32;

/// Number of bytes read to find the size of an image
///
/// This covers the headers of most images; only if they are not complete in it, the whole file
/// is read.
const HEADER_BYTES: u64 = 256 * 1024;

/// An encoded image: the path and the size in cells it fills
type Key = (PathBuf, Size);

enum State {
    /// Queued or being encoded
    Loading,
    Failed,
    Ready {
        protocol: Protocol,
        /// Id of the image in the terminal, when using the Kitty protocol
        kitty_id: Option<u32>,
    },
}

struct Entry {
    state: State,
    /// Frame the image was last drawn in
    last_used: u64,
}

/// State shared with the background thread
#[derive(Default)]
struct Shared {
    entries: Mutex<HashMap<Key, Entry>>,
    queue: Mutex<Queue>,
    wake: Condvar,
}

#[derive(Default)]
struct Queue {
    /// Images to encode, in order
    jobs: Vec<Key>,
    stop: bool,
}

pub struct ImageCache {
    picker: Option<Picker>,
    /// Maximum height of an image in lines; 0 disables images
    max_height: u16,
    /// Pixel size of images (with EXIF orientation applied), `None` if unreadable
    dimensions: RefCell<HashMap<PathBuf, Option<(u32, u32)>>>,
    shared: Arc<Shared>,
    /// Number of the current frame
    frame: u64,
    /// Images drawn in the current frame that are not encoded yet
    wanted: Vec<Key>,
    /// Ids of Kitty images to delete from the terminal
    kitty_deletes: Vec<u32>,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl ImageCache {
    pub fn new(max_height: u16, event_tx: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            picker: None,
            max_height,
            dimensions: Default::default(),
            shared: Default::default(),
            frame: 1,
            wanted: Vec::new(),
            kitty_deletes: Vec::new(),
            event_tx,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_height > 0
    }

    /// Enables rendering with the protocol and font size detected by `picker`
    pub fn set_picker(&mut self, picker: Picker) {
        self.clear_encoded();
        self.stop_encoder();
        self.shared = Default::default();
        spawn_encoder(picker.clone(), self.shared.clone(), self.event_tx.clone());
        self.picker = Some(picker);
    }

    /// Size in cells of the image at `path` fitted into `max_width` x `available_height`
    ///
    /// The size is computed from the image header; the first call for a path reads it from disk.
    /// Returns `None` if images are disabled or the image cannot be read.
    pub fn size(&self, path: &Path, max_width: u16, available_height: u16) -> Option<Size> {
        let font_size = self.picker.as_ref()?.font_size();
        let max_height = self.max_height.min(available_height);
        if max_width == 0 || max_height == 0 || font_size.width == 0 || font_size.height == 0 {
            return None;
        }
        let (width, height) = *self
            .dimensions
            .borrow_mut()
            .entry(path.to_owned())
            .or_insert_with(|| {
                // the input is any file a contact sent: a panicking decoder must not take down
                // the UI
                match catch_unwind(|| read_dimensions(path)) {
                    Ok(Ok(dimensions)) => Some(dimensions),
                    Ok(Err(error)) => {
                        warn!(path = %path.display(), %error, "failed to read image size");
                        None
                    }
                    Err(_) => {
                        warn!(path = %path.display(), "panicked while reading image size");
                        None
                    }
                }
            })
            .as_ref()?;
        Some(fit_cells(
            width,
            height,
            font_size,
            Size::new(max_width, max_height),
        ))
    }

    /// Renders the image at `path` with the `size` returned by [`Self::size`] into `area`
    ///
    /// If the image is not encoded yet, a placeholder is drawn, and the image is queued for
    /// encoding when the frame is finished with [`Self::finish_frame`]. When it is ready, an
    /// [`Event::ImageLoaded`] is sent.
    pub fn render(&mut self, f: &mut Frame, path: &Path, size: Size, area: Rect) {
        // reset styles of the cells (e.g. highlight of the selected message)
        f.render_widget(Clear, area);
        let key = (path.to_owned(), size);
        let mut entries = self.shared.entries.lock().unwrap();
        let entry = entries.entry(key.clone()).or_insert(Entry {
            state: State::Loading,
            last_used: 0,
        });
        if entry.last_used != self.frame && matches!(entry.state, State::Loading) {
            self.wanted.push((path.to_owned(), size));
        }
        entry.last_used = self.frame;
        let note = match &entry.state {
            State::Ready { protocol, .. } => {
                f.render_widget(Image::new(protocol), area);
                return;
            }
            State::Loading => "loading image…",
            State::Failed => "image could not be shown",
        };
        if matches!(entries[&key].state, State::Loading) {
            // e.g. after a resize: until the image is encoded for the new size, show the most
            // recent encoding for another size, if it fits
            let stand_in = entries
                .iter_mut()
                .filter(|((stand_in_path, stand_in_size), entry)| {
                    stand_in_path == path
                        && stand_in_size.width <= size.width
                        && stand_in_size.height <= size.height
                        && matches!(entry.state, State::Ready { .. })
                })
                .max_by_key(|(_, entry)| entry.last_used);
            if let Some((_, entry)) = stand_in
                && let State::Ready { protocol, .. } = &entry.state
            {
                // drawn in this frame, so it is not evicted
                entry.last_used = self.frame;
                f.render_widget(Image::new(protocol), area);
                return;
            }
        }
        let line = Rect { height: 1, ..area };
        f.render_widget(
            Paragraph::new(note).style(Style::default().fg(Color::DarkGray)),
            line,
        );
    }

    /// Queues the images drawn in this frame that are not encoded yet, and drops the least
    /// recently drawn images when there are too many
    ///
    /// Images queued in earlier frames and not drawn anymore are not encoded.
    pub fn finish_frame(&mut self) {
        {
            let mut queue = self.shared.queue.lock().unwrap();
            queue.jobs = std::mem::take(&mut self.wanted);
        }
        self.shared.wake.notify_one();

        let mut entries = self.shared.entries.lock().unwrap();
        if MAX_ENTRIES < entries.len() {
            let mut unused: Vec<(u64, Key)> = entries
                .iter()
                .filter(|(_, entry)| entry.last_used != self.frame)
                .map(|(key, entry)| (entry.last_used, key.clone()))
                .collect();
            unused.sort_unstable_by_key(|(last_used, _)| *last_used);
            let excess = entries.len() - MAX_ENTRIES;
            for (_, key) in unused.into_iter().take(excess) {
                if let Some(Entry {
                    state:
                        State::Ready {
                            kitty_id: Some(id), ..
                        },
                    ..
                }) = entries.remove(&key)
                {
                    self.kitty_deletes.push(id);
                }
            }
        }
        self.frame += 1;
    }

    /// Drops the images kept in the terminal, so that they are sent again on the next draw
    ///
    /// Called after the terminal was cleared. Only Kitty images live in the terminal; the others
    /// are part of the drawn cells and are redrawn anyway.
    pub fn clear_encoded(&mut self) {
        let mut entries = self.shared.entries.lock().unwrap();
        entries.retain(|_, entry| match entry.state {
            State::Ready {
                kitty_id: Some(id), ..
            } => {
                self.kitty_deletes.push(id);
                false
            }
            _ => true,
        });
    }

    /// Escape sequences deleting the dropped images from the terminal, to be written to it
    ///
    /// Only the Kitty protocol keeps images in the terminal; for the others this is empty.
    pub fn take_terminal_commands(&mut self) -> String {
        let is_tmux = self.picker.as_ref().is_some_and(|p| p.tmux_detected());
        let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
        self.kitty_deletes
            .drain(..)
            .map(|id| format!("{start}{escape}_Ga=d,d=I,i={id},q=2;{escape}\\{end}"))
            .collect()
    }

    fn stop_encoder(&self) {
        self.shared.queue.lock().unwrap().stop = true;
        self.shared.wake.notify_one();
    }

    #[cfg(test)]
    fn encoded_count(&self) -> usize {
        self.shared.entries.lock().unwrap().len()
    }
}

impl Drop for ImageCache {
    fn drop(&mut self) {
        self.stop_encoder();
    }
}

/// Spawns the thread encoding the queued images
///
/// The thread stops when the queue's `stop` flag is set.
fn spawn_encoder(picker: Picker, shared: Arc<Shared>, event_tx: mpsc::UnboundedSender<Event>) {
    std::thread::spawn(move || {
        let mut next_kitty_id = initial_kitty_id();
        loop {
            let key = {
                let mut queue = shared.queue.lock().unwrap();
                loop {
                    if queue.stop {
                        return;
                    }
                    if !queue.jobs.is_empty() {
                        break queue.jobs.remove(0);
                    }
                    queue = shared.wake.wait(queue).unwrap();
                }
            };
            let is_loading = matches!(
                shared.entries.lock().unwrap().get(&key),
                Some(Entry {
                    state: State::Loading,
                    ..
                })
            );
            if !is_loading {
                continue; // queued twice, or dropped meanwhile
            }

            let (path, size) = &key;
            let kitty_id = (picker.protocol_type() == ProtocolType::Kitty).then(|| {
                next_kitty_id = next_kitty_id.wrapping_add(1).max(1);
                next_kitty_id
            });
            // the input is any file a contact sent: a panicking decoder must only fail this image
            let state =
                match catch_unwind(AssertUnwindSafe(|| encode(&picker, path, *size, kitty_id))) {
                    Ok(Ok(protocol)) => State::Ready { protocol, kitty_id },
                    Ok(Err(error)) => {
                        warn!(path = %path.display(), %error, "failed to load image");
                        State::Failed
                    }
                    Err(_) => {
                        warn!(path = %path.display(), "panicked while loading image");
                        State::Failed
                    }
                };
            if let Some(entry) = shared.entries.lock().unwrap().get_mut(&key) {
                entry.state = state;
            }
            let _ = event_tx.send(Event::ImageLoaded);
        }
    });
}

/// Start of the ids of the Kitty images, so that several instances of gurk in one terminal are
/// unlikely to use the same ids
fn initial_kitty_id() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    nanos ^ std::process::id().rotate_left(16)
}

/// Decodes the image at `path` and encodes it to fill `size` cells
fn encode(
    picker: &Picker,
    path: &Path,
    size: Size,
    kitty_id: Option<u32>,
) -> anyhow::Result<Protocol> {
    let font_size = picker.font_size();
    let width_px = u32::from(size.width) * u32::from(font_size.width);
    let height_px = u32::from(size.height) * u32::from(font_size.height);
    #[cfg(test)]
    if path.ends_with("panic.png") {
        panic!("decoder panicked");
    }
    let image = decode(path, width_px, height_px)?;
    // pad to whole cells, as the protocols expect
    let image = Resize::Fit(None).resize(&image, font_size, size, None);
    let protocol = match kitty_id {
        Some(id) => {
            let compress = picker
                .capabilities()
                .contains(&Capability::KittyCompression);
            Protocol::Kitty(Kitty::new(
                image,
                size,
                id,
                picker.tmux_detected(),
                compress,
            )?)
        }
        None => picker.new_protocol(image, size, Resize::Fit(None))?,
    };
    Ok(protocol)
}

/// Decodes the image at `path`, shrinks it to fit `max_width` x `max_height` pixels and applies
/// its EXIF orientation
fn decode(path: &Path, max_width: u32, max_height: u32) -> anyhow::Result<DynamicImage> {
    let mut decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let image = DynamicImage::from_decoder(decoder)?;
    // rotate the thumbnail rather than the full image, which can be large
    let (max_width, max_height) = if swaps_axes(orientation) {
        (max_height, max_width)
    } else {
        (max_width, max_height)
    };
    let mut image = if max_width < image.width() || max_height < image.height() {
        image.thumbnail(max_width, max_height)
    } else {
        image
    };
    image.apply_orientation(orientation);
    Ok(image)
}

/// Pixel size of the image at `path`, as displayed (with its EXIF orientation applied)
///
/// Only reads the header of the image.
fn read_dimensions(path: &Path) -> anyhow::Result<(u32, u32)> {
    #[cfg(test)]
    if path.ends_with("panic-header.png") {
        panic!("header parser panicked");
    }
    let mut head = Vec::new();
    File::open(path)?
        .take(HEADER_BYTES)
        .read_to_end(&mut head)?;
    match oriented_dimensions(ImageReader::new(Cursor::new(&head))) {
        Ok(dimensions) => Ok(dimensions),
        // the header is not complete in the first bytes
        Err(_) if head.len() as u64 == HEADER_BYTES => {
            oriented_dimensions(ImageReader::new(BufReader::new(File::open(path)?)))
        }
        Err(error) => Err(error),
    }
}

fn oriented_dimensions<R: std::io::BufRead + std::io::Seek>(
    reader: ImageReader<R>,
) -> anyhow::Result<(u32, u32)> {
    let mut decoder = reader.with_guessed_format()?.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if swaps_axes(decoder.orientation()?) {
        Ok((height, width))
    } else {
        Ok((width, height))
    }
}

fn swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// Size in cells of an image of `width` x `height` pixels, shrunk to fit into `max` cells
///
/// Images are never enlarged. This is the size [`Resize::Fit`] gives the image.
fn fit_cells(width: u32, height: u32, font_size: FontSize, max: Size) -> Size {
    let (font_width, font_height) = (u32::from(font_size.width), u32::from(font_size.height));
    let to_cells = |width: u32, height: u32| {
        Size::new(
            width.div_ceil(font_width).try_into().unwrap_or(u16::MAX),
            height.div_ceil(font_height).try_into().unwrap_or(u16::MAX),
        )
    };
    let natural = to_cells(width, height);
    if natural.width <= max.width && natural.height <= max.height {
        return natural;
    }
    let max_width = (u32::from(max.width) * font_width).min(width);
    let max_height = (u32::from(max.height) * font_height).min(height);
    let ratio = f64::min(
        f64::from(max_width) / f64::from(width),
        f64::from(max_height) / f64::from(height),
    );
    let fitted_width = ((f64::from(width) * ratio).round() as u32).max(1);
    let fitted_height = ((f64::from(height) * ratio).round() as u32).max(1);
    let size = to_cells(fitted_width, fitted_height);
    Size::new(size.width.min(max.width), size.height.min(max.height))
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn draw(cache: &mut ImageCache, path: &Path, size: Size) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 1, size.width, size.height);
                cache.render(f, path, size, area);
            })
            .unwrap();
        cache.finish_frame();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn test_size_load_and_render() {
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
        // known from the header right away: 10 lines of 20px are 200px, which are 20 columns
        // of 10px
        let size = cache.size(&path, 40, 20).unwrap();
        assert_eq!(size, Size::new(20, 10));
        // capped by the available height
        assert_eq!(cache.size(&path, 40, 5), Some(Size::new(10, 5)));

        let red = Color::Rgb(255, 0, 0);
        let buffer = draw(&mut cache, &path, size);
        assert_ne!(buffer[(0, 1)].bg, red, "placeholder while loading");
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));

        let buffer = draw(&mut cache, &path, size);
        assert_eq!(buffer[(0, 1)].bg, red, "image drawn");
        assert_eq!(buffer[(19, 10)].bg, red, "image drawn");
        assert_ne!(buffer[(0, 0)].bg, red, "nothing drawn above");
        assert_ne!(buffer[(20, 5)].bg, red, "nothing drawn right of it");
    }

    #[test]
    fn test_only_drawn_images_are_loaded_and_cache_is_bounded() {
        let tempdir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..MAX_ENTRIES + 5)
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
        let size = cache.size(&paths[0], 40, 20).unwrap();
        assert_eq!(size, Size::new(10, 5));
        for path in &paths[1..] {
            assert_eq!(cache.size(path, 40, 20), Some(size));
        }
        assert_eq!(cache.encoded_count(), 0, "sizing loads nothing");

        for path in &paths {
            draw(&mut cache, path, size);
            assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        }
        assert_eq!(cache.encoded_count(), MAX_ENTRIES);

        // the most recently drawn image is still there
        let blue = Color::Rgb(0, 0, 255);
        let buffer = draw(&mut cache, paths.last().unwrap(), size);
        assert_eq!(buffer[(0, 1)].bg, blue);
        assert!(rx.try_recv().is_err(), "not loaded again");
    }

    #[test]
    fn test_images_no_longer_drawn_are_not_loaded() {
        let tempdir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..20)
            .map(|i| {
                let path = tempdir.path().join(format!("{i}.png"));
                RgbImage::from_pixel(1000, 1000, Rgb([0, 0, 255]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        let size = Size::new(10, 5);
        // a frame with all images, then one with only the last one
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| {
                for path in &paths {
                    cache.render(f, path, size, Rect::new(0, 0, 10, 5));
                }
            })
            .unwrap();
        cache.finish_frame();
        let last = (paths.last().unwrap().clone(), size);
        let is_loaded = |cache: &ImageCache, key: &Key| {
            !matches!(
                cache.shared.entries.lock().unwrap()[key].state,
                State::Loading
            )
        };
        while !is_loaded(&cache, &last) {
            draw(&mut cache, &last.0, size);
            assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        }

        let loaded = paths
            .iter()
            .filter(|path| is_loaded(&cache, &((*path).clone(), size)))
            .count();
        // only the few being encoded when the queue was replaced, and the last one
        assert!(loaded <= 4, "{loaded} images loaded");
    }

    #[test]
    fn test_resize_shows_previous_encoding_until_ready() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("photo.png");
        RgbImage::from_pixel(400, 400, Rgb([255, 0, 0]))
            .save(&path)
            .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        let small = Size::new(10, 5);
        draw(&mut cache, &path, small);
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));

        // a bigger size is not encoded yet: the smaller encoding stands in, no placeholder
        let red = Color::Rgb(255, 0, 0);
        let big = Size::new(20, 10);
        let buffer = draw(&mut cache, &path, big);
        assert_eq!(buffer[(0, 1)].bg, red);
        assert_eq!(buffer[(9, 5)].bg, red);
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        assert_eq!(draw(&mut cache, &path, big)[(19, 10)].bg, red);
    }

    #[test]
    fn test_clear_keeps_images_drawn_as_cells() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("photo.png");
        RgbImage::from_pixel(100, 100, Rgb([255, 0, 0]))
            .save(&path)
            .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        let size = Size::new(10, 5);
        draw(&mut cache, &path, size);
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));

        cache.clear_encoded();
        assert_eq!(
            draw(&mut cache, &path, size)[(0, 1)].bg,
            Color::Rgb(255, 0, 0)
        );
        assert_eq!(cache.take_terminal_commands(), "");
    }

    #[test]
    fn test_failed_image() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("not-an-image");
        std::fs::write(&path, b"hello").unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        assert_eq!(cache.size(&path, 40, 20), None, "no rows reserved");
    }

    #[test]
    fn test_panic_fails_only_that_image() {
        let tempdir = tempfile::tempdir().unwrap();
        let paths = ["panic.png", "ok.png"].map(|name| {
            let path = tempdir.path().join(name);
            RgbImage::from_pixel(100, 100, Rgb([0, 0, 255]))
                .save(&path)
                .unwrap();
            path
        });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        let size = Size::new(10, 5);
        for path in &paths {
            draw(&mut cache, path, size);
            assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        }
        let entries = cache.shared.entries.lock().unwrap();
        assert!(matches!(
            entries[&(paths[0].clone(), size)].state,
            State::Failed
        ));
        assert!(matches!(
            entries[&(paths[1].clone(), size)].state,
            State::Ready { .. }
        ));
    }

    #[test]
    fn test_panic_reading_header_reserves_nothing() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("panic-header.png");
        RgbImage::from_pixel(100, 100, Rgb([0, 0, 255]))
            .save(&path)
            .unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks());
        assert_eq!(cache.size(&path, 40, 20), None);
        assert_eq!(cache.size(&path, 40, 20), None, "not read again");
    }

    #[test]
    fn test_exif_orientation() {
        use image::ImageEncoder;
        use image::codecs::png::PngEncoder;

        // landscape: red top half, blue bottom half
        let image = RgbImage::from_fn(400, 200, |_, y| {
            if y < 100 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        });
        // little endian TIFF with one IFD entry: orientation (0x0112) = 6, rotate 90° clockwise
        let exif = [
            0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
            0, 0,
        ];
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("rotated.png");
        let mut encoder = PngEncoder::new(File::create(&path).unwrap());
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder
            .write_image(&image, 400, 200, image::ExtendedColorType::Rgb8)
            .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.set_picker(Picker::halfblocks()); // 10x20px font
        // portrait 200x400px fitted into 10 lines: 100x200px
        let size = cache.size(&path, 40, 20).unwrap();
        assert_eq!(size, Size::new(10, 10));

        draw(&mut cache, &path, size);
        assert!(matches!(rx.blocking_recv(), Some(Event::ImageLoaded)));
        let buffer = draw(&mut cache, &path, size);
        // the top of the image is on the right now
        assert_eq!(buffer[(0, 5)].bg, Color::Rgb(0, 0, 255));
        assert_eq!(buffer[(9, 5)].bg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn test_fit_cells() {
        let font = FontSize::new(10, 20);
        // natural size, rounded up to whole cells
        assert_eq!(fit_cells(15, 15, font, Size::new(40, 10)), Size::new(2, 1));
        // capped by height, then by width
        assert_eq!(
            fit_cells(400, 400, font, Size::new(40, 10)),
            Size::new(20, 10)
        );
        assert_eq!(
            fit_cells(4000, 200, font, Size::new(40, 10)),
            Size::new(40, 1)
        );
    }

    #[test]
    fn test_kitty_delete_commands() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cache = ImageCache::new(10, tx);
        cache.kitty_deletes.push(7);
        assert_eq!(
            cache.take_terminal_commands(),
            "\x1b_Ga=d,d=I,i=7,q=2;\x1b\\"
        );
        assert_eq!(cache.take_terminal_commands(), "");
    }

    #[test]
    fn test_disabled() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let cache = ImageCache::new(0, tx);
        assert!(!cache.is_enabled());
    }
}
