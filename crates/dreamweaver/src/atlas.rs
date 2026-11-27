//! The thumbnail atlas: every world's wiki picture packed into the one jpeg the app samples.
//!
//! Each world's picture sits at the `cell` the dump assigns; the placeholder takes the last cell.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use axum::body::Bytes;

use crate::model::Dump;

/// Must match the app's `thumbnails::CELL`. Both sides are a multiple of 16 so the app's mip levels
/// cannot bleed one cell into the next.
const CELL: [u32; 2] = [64, 48];
/// YNOproject's own picture for a world the player has not been to.
const PLACEHOLDER_URL: &str = "https://ynoproject.net/2kki/images/unknown_location.png";
const JPEG_QUALITY: u8 = 85;
const PARALLEL_DOWNLOADS: usize = 8;
/// The pack is awaited by the sync loop, so a stalled download must not stall syncing.
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const ATLAS_FILE: &str = "thumbnails.jpg";
const PLACEHOLDER_FILE: &str = "unknown_location.png";
/// Wiki downloads, kept by file name: the wiki hashes the name into the directory.
const DOWNLOADS_DIR: &str = "images";

pub struct Atlas {
    directory: PathBuf,
    current: RwLock<Option<Arc<Packed>>>,
}

pub struct Packed {
    pub atlas_jpeg: Bytes,
    /// Full size and re-encoded as RGB: the app decodes the wiki's indexed png as one channel.
    pub placeholder_png: Bytes,
    /// Empty for a pack read back from disk, so the first [`Atlas::pack`] always repacks.
    sources: Vec<CellSource>,
}

#[derive(PartialEq)]
struct CellSource {
    cell: usize,
    url: String,
}

impl Atlas {
    /// Serves whatever an earlier process left in `directory` until the first [`Atlas::pack`].
    pub fn open(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        let read = |name| std::fs::read(directory.join(name));
        let packed = match (read(ATLAS_FILE), read(PLACEHOLDER_FILE)) {
            (Ok(atlas_jpeg), Ok(placeholder_png)) => Some(Arc::new(Packed {
                atlas_jpeg: atlas_jpeg.into(),
                placeholder_png: placeholder_png.into(),
                sources: Vec::new(),
            })),
            _ => {
                tracing::info!("no atlas in {} yet", directory.display());
                None
            }
        };
        Atlas {
            directory,
            current: RwLock::new(packed),
        }
    }

    pub fn packed(&self) -> Option<Arc<Packed>> {
        self.current.read().unwrap().clone()
    }

    /// Repacks if the dump's pictures differ from the ones last packed. A failure keeps the last
    /// atlas standing.
    pub async fn pack(&self, http: &reqwest::Client, dump: &Dump) {
        let sources = cell_sources(dump);
        if sources.is_empty()
            || self
                .packed()
                .is_some_and(|packed| packed.sources == sources)
        {
            return;
        }
        let downloads = self.directory.join(DOWNLOADS_DIR);
        if let Err(error) = std::fs::create_dir_all(&downloads) {
            tracing::error!("cannot create {}: {error}", downloads.display());
            return;
        }
        let Some(placeholder) = download_cached(http, PLACEHOLDER_URL, &downloads).await else {
            tracing::error!("cannot fetch the placeholder; the atlas stands");
            return;
        };
        let thumbnails = thumbnails(http, &sources, &downloads).await;
        let encoded =
            tokio::task::spawn_blocking(move || encode(sources, &thumbnails, &placeholder)).await;
        let packed = match encoded {
            Ok(Ok(packed)) => packed,
            Ok(Err(error)) => return tracing::error!("cannot pack the atlas: {error}"),
            Err(error) => return tracing::error!("the atlas packer died: {error}"),
        };
        for (name, bytes) in [
            (ATLAS_FILE, &packed.atlas_jpeg),
            (PLACEHOLDER_FILE, &packed.placeholder_png),
        ] {
            let path = self.directory.join(name);
            if let Err(error) = crate::store::write_atomically(&path, bytes) {
                tracing::error!("cannot write {}: {error}", path.display());
            }
        }
        *self.current.write().unwrap() = Some(Arc::new(packed));
    }
}

/// Every world with a picture to show, by cell. Secret worlds are left out, so their cell stays black.
fn cell_sources(dump: &Dump) -> Vec<CellSource> {
    let mut sources: Vec<CellSource> = dump
        .worlds
        .iter()
        .filter(|world| !world.secret && !world.filename.is_empty())
        .filter_map(|world| {
            Some(CellSource {
                cell: world.cell?,
                url: world.filename.clone(),
            })
        })
        .collect();
    sources.sort_by_key(|source| source.cell);
    sources
}

/// One per source, in order; `None` where the picture cannot be had.
async fn thumbnails(
    http: &reqwest::Client,
    sources: &[CellSource],
    downloads: &Path,
) -> Vec<Option<image::RgbImage>> {
    let urls: Arc<[String]> = sources.iter().map(|source| source.url.clone()).collect();
    let next_url = Arc::new(AtomicUsize::new(0));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..PARALLEL_DOWNLOADS {
        let (http, urls, next_url, downloads) = (
            http.clone(),
            urls.clone(),
            next_url.clone(),
            downloads.to_owned(),
        );
        workers.spawn(async move {
            let mut made = Vec::new();
            loop {
                let at = next_url.fetch_add(1, Ordering::Relaxed);
                let Some(url) = urls.get(at) else {
                    return made;
                };
                let thumbnail = match download_cached(&http, url, &downloads).await {
                    Some(bytes) => tokio::task::spawn_blocking(move || decode_thumbnail(&bytes))
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                if thumbnail.is_none() {
                    tracing::warn!("no picture for {url}");
                }
                made.push((at, thumbnail));
            }
        });
    }
    let mut thumbnails = vec![None; urls.len()];
    while let Some(made) = workers.join_next().await {
        for (at, thumbnail) in made.unwrap_or_default() {
            thumbnails[at] = thumbnail;
        }
    }
    thumbnails
}

/// Cached before returning, so a resumed pack skips what was already downloaded.
async fn download_cached(http: &reqwest::Client, url: &str, downloads: &Path) -> Option<Vec<u8>> {
    let name = url.rsplit('/').next().filter(|name| !name.is_empty())?;
    let cached = downloads.join(name);
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some(bytes);
    }
    let response = http
        .get(url)
        .header(reqwest::header::ORIGIN, crate::smw::ORIGIN)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let bytes = response.bytes().await.ok()?.to_vec();
    if let Err(error) = std::fs::write(&cached, &bytes) {
        tracing::warn!("cannot cache {}: {error}", cached.display());
    }
    Some(bytes)
}

fn encode(
    sources: Vec<CellSource>,
    thumbnails: &[Option<image::RgbImage>],
    placeholder: &[u8],
) -> image::ImageResult<Packed> {
    let placeholder_full = image::load_from_memory(placeholder)?.to_rgb8();
    let mut placeholder_png = Vec::new();
    placeholder_full.write_to(
        &mut std::io::Cursor::new(&mut placeholder_png),
        image::ImageFormat::Png,
    )?;

    let (columns, rows) = grid(sources.last().map(|source| source.cell));
    let mut atlas = image::RgbImage::new(columns * CELL[0], rows * CELL[1]);
    let mut place = |cell: u32, picture: &image::RgbImage| {
        let (x, y) = (cell % columns * CELL[0], cell / columns * CELL[1]);
        image::imageops::replace(&mut atlas, picture, x.into(), y.into());
    };
    let mut packed_count = 0;
    for (source, thumbnail) in sources.iter().zip(thumbnails) {
        if let Some(thumbnail) = thumbnail {
            place(source.cell as u32, thumbnail);
            packed_count += 1;
        }
    }
    place(columns * rows - 1, &thumbnail(&placeholder_full));

    // The 8x8 JPEG blocks divide a cell, so artefacts stay in one world.
    let mut atlas_jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut atlas_jpeg, JPEG_QUALITY)
        .encode_image(&atlas)?;
    tracing::info!(
        "packed {packed_count}/{} worlds into a {}x{} atlas ({} KiB)",
        sources.len(),
        atlas.width(),
        atlas.height(),
        atlas_jpeg.len() / 1024,
    );
    Ok(Packed {
        atlas_jpeg: atlas_jpeg.into(),
        placeholder_png: placeholder_png.into(),
        sources,
    })
}

/// A square-ish grid holding every cell up to `highest`, plus the placeholder after them. Sized by
/// the highest cell rather than the world count: a dropped world's cell is never reused.
fn grid(highest: Option<usize>) -> (u32, u32) {
    let slots = highest.map_or(1, |highest| highest + 2);
    let columns = (slots as f64).sqrt().ceil() as u32;
    (columns, slots.div_ceil(columns as usize) as u32)
}

fn decode_thumbnail(bytes: &[u8]) -> Option<image::RgbImage> {
    Some(thumbnail(&image::load_from_memory(bytes).ok()?.to_rgb8()))
}

/// Centre-cropped to the cell's aspect ratio, then scaled to the cell size.
fn thumbnail(picture: &image::RgbImage) -> image::RgbImage {
    let (width, height) = picture.dimensions();
    let scale = (width * CELL[1]).min(height * CELL[0]);
    let (crop_w, crop_h) = ((scale / CELL[1]).max(1), (scale / CELL[0]).max(1));
    let cropped = image::imageops::crop_imm(
        picture,
        (width - crop_w) / 2,
        (height - crop_h) / 2,
        crop_w,
        crop_h,
    )
    .to_image();
    image::imageops::resize(
        &cropped,
        CELL[0],
        CELL[1],
        image::imageops::FilterType::Lanczos3,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_last_cell_is_the_placeholder_and_no_world_reaches_it() {
        for highest in [0, 1, 2, 3, 8, 15, 16, 1578] {
            let (columns, rows) = super::grid(Some(highest));
            let last = (columns * rows) as usize - 1;
            assert!(last > highest, "{highest} of {columns}x{rows}");
        }
        assert_eq!(super::grid(None), (1, 1));
    }

    #[test]
    fn a_secret_world_has_no_picture_to_pack() {
        let mut dump = crate::model::dream_tree();
        dump.worlds[1].secret = true;
        let secret_cell = dump.worlds[1].cell;
        let sources = super::cell_sources(&dump);
        assert_eq!(sources.len(), dump.worlds.len() - 1);
        assert!(
            sources
                .iter()
                .all(|source| Some(source.cell) != secret_cell)
        );
    }

    #[test]
    fn a_wide_picture_loses_its_sides() {
        // Red, then green 128x48 in the middle, then red: only the green survives a 4:3 crop.
        let mut wide = image::RgbImage::from_pixel(256, 48, image::Rgb([255, 0, 0]));
        for x in 64..192 {
            for y in 0..48 {
                wide.put_pixel(x, y, image::Rgb([0, 255, 0]));
            }
        }
        let thumbnail = super::thumbnail(&wide);
        assert_eq!(thumbnail.dimensions(), (64, 48));
        assert!(
            thumbnail
                .pixels()
                .all(|pixel| pixel[0] < 16 && pixel[1] > 240)
        );
    }

    #[test]
    fn the_pack_puts_each_picture_at_its_cell_and_the_placeholder_last() {
        let white = image::RgbImage::from_pixel(64, 48, image::Rgb([255; 3]));
        let mut placeholder = Vec::new();
        white
            .write_to(
                &mut std::io::Cursor::new(&mut placeholder),
                image::ImageFormat::Png,
            )
            .unwrap();
        let sources = vec![super::CellSource {
            cell: 2,
            url: String::new(),
        }];
        let packed = super::encode(sources, &[Some(white)], &placeholder).unwrap();
        let atlas = image::load_from_memory(&packed.atlas_jpeg)
            .unwrap()
            .to_rgb8();
        // Two columns of two rows: cell 2 is bottom left, the placeholder bottom right.
        assert_eq!(atlas.dimensions(), (128, 96));
        let brightness = |x, y| atlas.get_pixel(x, y)[0];
        assert!(brightness(32, 24) < 16, "cell 0 has no picture");
        assert!(brightness(32, 72) > 240, "cell 2");
        assert!(brightness(96, 72) > 240, "placeholder");
    }
}
