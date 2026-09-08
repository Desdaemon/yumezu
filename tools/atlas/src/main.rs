//! Packs every world's wiki image into the single thumbnail atlas the visualization samples.
//!
//! Run from the repository root, through `just thumbnails`. Downloads are cached under
//! `tools/atlas/cache`, so a re-run after a layout change costs nothing but the packing.
//!
//! A world's cell is its index among the worlds the app draws, which the app derives from the atlas
//! it loads, so there is no manifest to keep in step. What the two sides share is [`CELL`], which
//! the app checks the atlas against on load, and which worlds are left out -- the secrets, which
//! this has to drop where the app does, or every cell after the first of them is off by one.

use std::io::Write;
use std::path::{Path, PathBuf};

/// 4:3, which nearly every world image already is, so the crop usually takes nothing off it.
///
/// Both sides are a multiple of 16, which is what lets the app mipmap the atlas: a mip texel stays
/// inside one cell down to the level where it spans 16 source texels, so the levels the app keeps
/// cannot bleed one world's thumbnail into its neighbour's.
const CELL: [u32; 2] = [64, 48];
const ORIGIN: &str = "https://explorer.yume.wiki";
/// The picture drawn for a world the player has not been to, which is YNOproject's own client's.
///
/// Packed into the last cell of the grid rather than one counted off the worlds, so the app finds it
/// from the grid dimensions alone.
const UNKNOWN: &str = "https://ynoproject.net/2kki/images/unknown_location.png";
/// High enough that the pixel art keeps its edges at the size it is drawn, low enough that the
/// whole atlas is a download rather than a wait.
const JPEG_QUALITY: u8 = 85;
/// Enough to keep the pipe full, few enough to stay a polite caller of someone else's wiki.
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
        .filter(|world| !world["secret"].as_bool().unwrap_or(false))
        .map(|world| world["filename"].as_str().unwrap_or_default().to_owned())
        .collect();

    let images = fetch_all(&urls, &cache);
    let unknown_bytes = fetch_all(&[UNKNOWN.to_owned()], &cache)
        .into_iter()
        .next()
        .flatten()
        .expect("cannot fetch the placeholder for an unvisited world");
    // Beside the atlas as well as inside it: the app draws an unvisited world from the whole
    // picture, there being no sharper version to switch to and a cell of it being this line art run
    // through the atlas's jpeg. The cell is still packed, for the sidebar's catalog.
    //
    // Decoded and written out again rather than copied: the wiki serves this one as an indexed png,
    // which the app's own decoder reads as one channel, drawing the black background red. Every
    // other picture goes through the same conversion into the atlas.
    let unknown_out = repo.join("static/unknown_location.png");
    let unknown_full = image::load_from_memory(&unknown_bytes)
        .expect("the placeholder is not an image")
        .to_rgb8();
    unknown_full
        .save(&unknown_out)
        .expect("cannot write the placeholder");
    let unknown = thumbnail(&unknown_bytes).expect("the placeholder is not an image");

    // Square-ish, so neither dimension runs into a driver's texture size limit. Sized for the
    // worlds *and* the placeholder, so there is always a last cell left over for it.
    let columns = ((images.len() + 1) as f64).sqrt().ceil() as u32;
    let rows = (images.len() + 1).div_ceil(columns as usize) as u32;
    let mut atlas = image::RgbImage::new(columns * CELL[0], rows * CELL[1]);
    let mut packed = 0;
    for (world, bytes) in images.iter().enumerate() {
        let Some(thumbnail) = bytes.as_ref().and_then(|bytes| thumbnail(bytes)) else {
            // Left black: a world whose image the wiki no longer serves is a node without a
            // picture rather than a hole in the atlas.
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
    // Last, in the corner the app reads it out of.
    let last = columns * rows - 1;
    image::imageops::replace(
        &mut atlas,
        &unknown,
        ((last % columns) * CELL[0]) as i64,
        ((last / columns) * CELL[1]) as i64,
    );

    // JPEG, at a fifth of what the same atlas costs as a PNG, and a thumbnail this small has no
    // detail to lose at a few tens of pixels wide. Its 8x8 blocks divide both sides of a cell, so
    // the artefacts it does introduce stay inside the world they belong to.
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
        "{}x{} placeholder -> {}",
        unknown_full.width(),
        unknown_full.height(),
        unknown_out.display()
    );
    println!(
        "{packed}/{} worlds packed into {}x{} -> {} ({:.1} MiB)",
        images.len(),
        atlas.width(),
        atlas.height(),
        out.display(),
        size as f64 / (1 << 20) as f64,
    );
}

/// Cropped to the cell's aspect ratio about its centre, then scaled. Cropping rather than
/// letterboxing: a thumbnail this small has no room to spend on bars, and the subject is in the
/// middle.
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

/// In world order, from the cache where already there. `None` for an image that cannot be had.
fn fetch_all(urls: &[String], cache: &Path) -> Vec<Option<Vec<u8>>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("cannot build an HTTP client");
    // Handed out one at a time rather than split into per-worker ranges: the images vary enough in
    // size that a fixed split leaves workers idle at the end of a run.
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

/// A download is cached before it is returned, so an interrupted run resumes where it stopped.
fn fetch(client: &reqwest::blocking::Client, url: &str, cache: &Path) -> Option<Vec<u8>> {
    // The wiki's own name for the file. Its directory is a hash of that name, so the name alone
    // identifies the image.
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
