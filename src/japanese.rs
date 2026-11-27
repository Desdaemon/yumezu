//! Japanese fonts: bundled Shinonome, falling back to an installed system face, or to Noto Sans JP
//! downloaded on the web build. Loaded in every language for the Japanese world names.

/// Shinonome's 12-dot gothic, the face EasyRPG Player draws Japanese in. Public domain.
const SHINONOME: &[u8] = include_bytes!("../assets/JF-Dot-Shinonome12.ttf");

// Only added as fallbacks, never looked up.
const SHINONOME_NAME: &str = "shinonome";
const FALLBACK_NAME: &str = "japanese";

pub(super) enum Japanese {
    /// The state at startup, with the system face lookup already spawned.
    Starting(super::fetch::Pending<Option<(Vec<u8>, u32)>>),
    /// Shinonome installed; a system face is loading to cover missing glyphs.
    Coming(super::fetch::Pending<Option<(Vec<u8>, u32)>>),
    /// The system face installed or not found; nothing more is tried.
    Settled,
}

impl Japanese {
    pub(super) fn new() -> Self {
        Self::Starting(super::fetch::spawn(face()))
    }

    /// Called every frame; acts on only two frames.
    pub(super) fn serve(&mut self, ctx: &egui::Context) {
        // egui has no fonts to measure a face against before the first pass.
        if ctx.cumulative_pass_nr() == 0 {
            return;
        }
        *self = match std::mem::replace(self, Self::Settled) {
            Self::Starting(pending) => {
                install(ctx, SHINONOME_NAME, SHINONOME.to_vec(), 0);
                Self::Coming(pending)
            }
            Self::Coming(pending) => match pending.take() {
                None => Self::Coming(pending),
                Some(None) => {
                    log::warn!("no system Japanese font: what Shinonome lacks is drawn as boxes");
                    Self::Settled
                }
                Some(Some((face, index))) => {
                    install(ctx, FALLBACK_NAME, face, index);
                    Self::Settled
                }
            },
            Self::Settled => Self::Settled,
        };
    }
}

/// Blocks the executor thread while reading the font collection: once, at startup, long before
/// anything else needs that thread.
#[cfg(not(target_family = "wasm"))]
async fn face() -> Option<(Vec<u8>, u32)> {
    installed()
}

#[cfg(target_family = "wasm")]
async fn face() -> Option<(Vec<u8>, u32)> {
    // The served file is a single face, not a collection.
    download().await.map(|face| (face, 0))
}

/// Added at the lowest priority, so Latin text keeps the panel's font, and a face added later draws
/// only the glyphs the earlier ones lack.
fn install(ctx: &egui::Context, name: &str, face: Vec<u8>, index: u32) {
    let data = egui::FontData {
        index,
        tweak: egui::FontTweak {
            y_offset_factor: lowered(ctx, &face, index),
            ..egui::FontTweak::default()
        },
        ..egui::FontData::from_owned(face)
    };
    ctx.add_font(egui::epaint::text::FontInsert::new(
        name,
        data,
        [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
            .into_iter()
            .map(|family| egui::epaint::text::InsertFontFamily {
                family,
                priority: egui::epaint::text::FontPriority::Lowest,
            })
            .collect(),
    ));
}

/// How far to shift the face onto the panel font's baseline, as a fraction of the font size.
/// Positive is downwards, per [`egui::FontTweak::y_offset_factor`]. Zero if unmeasurable.
///
/// egui centres the faces in a family, and a Japanese face's ascent is 1.16 of the font size
/// against a Latin face's 0.93, so centring drops the Japanese a full point. Measured, since the
/// face varies per platform, against the proportional family the panel uses.
fn lowered(ctx: &egui::Context, face: &[u8], index: u32) -> f32 {
    /// The baseline's distance from the middle of the line, which egui's centring aligns.
    fn from_middle(ascent: f32, line: f32) -> f32 {
        ascent - line / 2.0
    }

    let panel = {
        let font = egui::TextStyle::Body.resolve(&ctx.style_of(ctx.theme()));
        let size = font.size;
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap("A".to_owned(), font, egui::Color32::PLACEHOLDER)
        });
        let Some(glyph) = galley.rows.first().and_then(|row| row.row.glyphs.first()) else {
            return 0.0;
        };
        from_middle(glyph.font_ascent, glyph.font_height) / size
    };

    let japanese = {
        use skrifa::{MetadataProvider as _, instance::LocationRef, prelude::Size};

        // Unscaled and divided by the em, as egui reads them.
        let Ok(font) = skrifa::FontRef::from_index(face, index) else {
            return 0.0;
        };
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let line = metrics.ascent - metrics.descent + metrics.leading;
        from_middle(metrics.ascent, line) / metrics.units_per_em as f32
    };

    panel - japanese
}

/// The Japanese UI face each platform ships, best first, ending in the faces of older versions.
/// Named to match the face the system draws Japanese in; a face failing [`SAMPLE`] is skipped.
#[cfg(target_os = "windows")]
const PREFERRED: &[&str] = &[
    "Yu Gothic UI",
    "Yu Gothic",
    "Meiryo UI",
    "Meiryo",
    "MS PGothic",
    "MS Gothic",
];
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PREFERRED: &[&str] = &[
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Hiragino Kaku Gothic Pro",
];
#[cfg(target_os = "android")]
const PREFERRED: &[&str] = &["Noto Sans CJK JP", "Noto Sans JP", "Droid Sans Japanese"];
/// Linux and the BSDs, which use whichever face the distribution packaged.
#[cfg(not(any(
    target_family = "wasm",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
)))]
const PREFERRED: &[&str] = &[
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "IPAexGothic",
    "IPAGothic",
    "VL PGothic",
    "VL Gothic",
    "TakaoPGothic",
];

/// The system font Japanese is drawn with, and the face index. Candidates come from three sources
/// in order, and each must draw [`SAMPLE`]:
///
/// 1. [`PREFERRED`], the face the platform draws Japanese in.
/// 2. The platform's script fallback, for a Japanese font not in [`PREFERRED`]. Keyed by `Hira`:
///    `Jpan` resolves to a Latin font with no kana, and font databases key on the kana code.
/// 3. Every system family, sorted so the same last resort is picked each session.
///
/// A pan-CJK font from 2 or 3 may show Chinese or Korean glyph shapes, still better than none.
#[cfg(not(target_family = "wasm"))]
fn installed() -> Option<(Vec<u8>, u32)> {
    use fontique::{Collection, CollectionOptions, FallbackKey, Script, SourceCache};

    /// One kana, which no Latin font carries, and two of the kanji every CJK font does.
    const SAMPLE: [char; 3] = ['あ', '世', '界'];

    let mut collection = Collection::new(CollectionOptions {
        system_fonts: true,
        shared: false,
    });
    let mut sources = SourceCache::default();

    let named: Vec<_> = PREFERRED
        .iter()
        .filter_map(|name| collection.family_id(name))
        .collect();
    let script: Vec<_> = collection
        .fallback_families(FallbackKey::from((
            Script::from_str_unchecked("Hira"),
            "ja",
        )))
        .collect();
    let mut names: Vec<_> = collection.family_names().map(str::to_owned).collect();
    names.sort_unstable();
    let every: Vec<_> = names
        .into_iter()
        .filter_map(|name| collection.family_id(&name))
        .collect();

    named.into_iter().chain(script).chain(every).find_map(|id| {
        let family = collection.family(id)?;
        let font = family.default_font()?;
        let blob = font.load(Some(&mut sources))?;
        let charmap = font.charmap_index().charmap(blob.as_ref())?;
        SAMPLE
            .iter()
            .all(|&glyph| charmap.map(glyph).is_some())
            .then(|| {
                log::info!("Japanese drawn with {:?}", family.name());
                // Copied: egui keeps the bytes while drawing; borrowing the mapping is `unsafe`.
                (blob.as_ref().to_vec(), font.index())
            })
    })
}

/// Noto Sans JP, pinned, from a host that serves the font cross-origin. The whole face: a subset
/// of the world names is a tenth the size but needs rebuilding with every dump refresh.
#[cfg(target_family = "wasm")]
const URL: &str = "https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf";

/// `None` leaves missing glyphs drawn as boxes.
#[cfg(target_family = "wasm")]
async fn download() -> Option<Vec<u8>> {
    use super::fetch;
    use super::prelude::*;
    fetch::client()
        .get(URL)
        .fetch_bytes()
        .await
        .inspect_err(|error| log::warn!("cannot download the Japanese font: {error}"))
        .ok()
}
