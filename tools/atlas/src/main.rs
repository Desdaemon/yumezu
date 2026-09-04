//! Packs every world's wiki image into the single thumbnail atlas the visualization samples.
//!
//! Run from the repository root, through `just thumbnails`. Downloads are cached under
//! `tools/atlas/cache`, so a re-run after a layout change costs nothing but the packing.
//!
//! A world's thumbnail is its cell of the grid, and its cell is its index in `data.json`: the
//! app derives the same mapping from the atlas it loads, so there is no manifest to keep in step.
//! Only [`CELL`] is shared knowledge, and the app checks the atlas against it on load.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Size of one thumbnail, in pixels. 4:3, which is what nearly every world image already is, so
/// the crop that fits an image to a cell usually takes nothing off it.
///
/// Both sides are a multiple of 16, which is what lets the app mipmap the atlas: a mip texel
/// stays inside one cell down to the level where it spans 16 source texels, so the levels the app
/// keeps cannot bleed one world's thumbnail into its neighbour's.
const CELL: [u32; 2] = [64, 48];
const ORIGIN: &str = "https://explorer.yume.wiki";
/// How hard the atlas is compressed. High enough that the pixel art keeps its edges at the size
/// it is drawn, low enough that the whole atlas is a download rather than a wait.
const JPEG_QUALITY: u8 = 85;
/// How many images to fetch at once. Enough to keep the pipe full, few enough to stay a polite
/// caller of someone else's wiki.
const WORKERS: usize = 8;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = root.parent().and_then(Path::parent).unwrap().to_path_buf();
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).expect("cannot create the download cache");

    let dump = std::fs::read(repo.join("data.json")).expect("data.json is missing");
    let dump: serde_json::Value = serde_json::from_slice(&dump).expect("data.json is malformed");
    let urls: Vec<String> = dump["worldData"]
        .as_array()
        .expect("data.json has no worldData")
        .iter()
        .map(|world| world["filename"].as_str().unwrap_or_default().to_owned())
        .collect();

    let images = fetch_all(&urls, &cache);

    // Square-ish, so neither dimension of the atlas runs far ahead of the other and into a
    // driver's texture size limit.
    let columns = (images.len() as f64).sqrt().ceil() as u32;
    let rows = images.len().div_ceil(columns as usize) as u32;
    let mut atlas = image::RgbImage::new(columns * CELL[0], rows * CELL[1]);
    let mut packed = 0;
    for (world, bytes) in images.iter().enumerate() {
        let Some(thumbnail) = bytes.as_ref().and_then(|bytes| thumbnail(bytes)) else {
            // Left black. The app draws the world's depth-colored plate either way, so a world
            // whose image the wiki no longer serves is a node without a picture rather than a
            // hole in the atlas.
            continue;
        };
        let cell = (world as u32 % columns, world as u32 / columns);
        image::imageops::replace(
            &mut atlas,
            &thumbnail,
            (cell.0 * CELL[0]) as i64,
            (cell.1 * CELL[1]) as i64,
        );
        packed += 1;
    }

    // JPEG, at a fifth of what the same atlas costs as a PNG: the app fetches it over the
    // network, and a thumbnail this small has no detail for the compression to lose that survives
    // being drawn a few tens of pixels wide. Its 8x8 blocks divide both sides of a cell, so the
    // artefacts it does introduce stay inside the world they belong to.
    let out = repo.join("static/thumbnails.jpg");
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(&out).expect("cannot write the atlas"));
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, JPEG_QUALITY)
        .encode_image(&atlas)
        .expect("cannot encode the atlas");
    file.flush().expect("cannot write the atlas");
    drop(file);
    let size = std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0);
    println!(
        "{packed}/{} worlds packed into {}x{} -> {} ({:.1} MiB)",
        images.len(),
        atlas.width(),
        atlas.height(),
        out.display(),
        size as f64 / (1 << 20) as f64,
    );
}

/// Fits one downloaded image to a cell: cropped to the cell's shape about its centre, then scaled.
///
/// Cropping rather than letterboxing, because a thumbnail this small has no room to spend on
/// bars, and the middle of a screenshot is where its subject is.
fn thumbnail(bytes: &[u8]) -> Option<image::RgbImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgb8();
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }
    // The largest cell-shaped rectangle the image contains.
    let scale = (width * CELL[1]).min(height * CELL[0]);
    let (crop_w, crop_h) = (scale / CELL[1], scale / CELL[0]);
    let cropped = image::imageops::crop_imm(
        &image,
        (width - crop_w) / 2,
        (height - crop_h) / 2,
        crop_w.max(1),
        crop_h.max(1),
    )
    .to_image();
    Some(image::imageops::resize(
        &cropped,
        CELL[0],
        CELL[1],
        image::imageops::FilterType::Lanczos3,
    ))
}

/// Every image, in world order, taken from the cache where it is already there and downloaded
/// where it is not. `None` for a world whose image cannot be had at all.
fn fetch_all(urls: &[String], cache: &Path) -> Vec<Option<Vec<u8>>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("cannot build an HTTP client");
    // Handed out one at a time rather than split into per-worker ranges: the images vary enough
    // in size that a fixed split leaves workers idle at the end of a run.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (fetched, collect) = std::sync::mpsc::channel();

    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            let (client, next, fetched) = (client.clone(), &next, fetched.clone());
            scope.spawn(move || {
                loop {
                    let world = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if world >= urls.len() {
                        return;
                    }
                    let bytes = fetch(&client, &urls[world], cache);
                    if bytes.is_none() {
                        eprintln!("\rno image for world {world}: {}", urls[world]);
                    }
                    // The receiver outlives every sender, so this cannot fail.
                    fetched.send((world, bytes)).unwrap();
                }
            });
        }
        // The workers hold the only senders left, so the collection below ends when they do.
        drop(fetched);

        let mut images = vec![None; urls.len()];
        for (done, (world, bytes)) in collect.iter().enumerate() {
            images[world] = bytes;
            let done = done + 1;
            if done % 50 == 0 || done == urls.len() {
                print!("\r{done}/{} fetched", urls.len());
                let _ = std::io::stdout().flush();
            }
        }
        println!();
        images
    })
}

/// One image, from the cache or from the wiki. A download is cached before it is returned, so an
/// interrupted run resumes where it stopped.
fn fetch(client: &reqwest::blocking::Client, url: &str, cache: &Path) -> Option<Vec<u8>> {
    // The wiki's own name for the file. Its directory is a hash of that name, so the name alone
    // already identifies the image.
    let name = url.rsplit('/').next().filter(|name| !name.is_empty())?;
    let cached = cache.join(name);
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some(bytes);
    }
    let response = client
        .get(url)
        .header(reqwest::header::ORIGIN, ORIGIN)
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().ok()?.to_vec();
    // Not fatal: a cache that cannot be written only costs the next run its downloads.
    let _ = std::fs::write(&cached, &bytes);
    Some(bytes)
}
