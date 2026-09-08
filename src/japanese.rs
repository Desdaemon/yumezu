//! The Japanese face, which nothing this app starts with has and everything Japanese needs.
//!
//! egui's own fonts carry Latin and little else, so Japanese is drawn as empty boxes without this.
//! A face carrying the glyphs is a few megabytes wherever it comes from, so it is sent for as the
//! app starts and installed the frame it turns up.
//!
//! Sent for whatever language the run is in: an English run shows the wiki's Japanese names beside
//! the English ones.
//!
//! Where it comes from is the whole of what the platforms differ in. A device usually has one
//! already, which [`installed`] asks for by name; a page downloads one.

// Only ever added, never looked up: a fallback rather than a family anything asks for.
const NAME: &str = "japanese";

pub(super) enum Japanese {
    /// Started as the app is built, so this is what every run opens in.
    Coming(super::fetch::Pending<Option<(Vec<u8>, u32)>>),
    /// Installed, or looked for and not found: there is no second place to look.
    Settled,
}

impl Japanese {
    pub(super) fn new() -> Self {
        Self::Coming(super::fetch::spawn(face()))
    }

    /// Called every frame, and does nothing on all but one of them.
    pub(super) fn serve(&mut self, ctx: &egui::Context) {
        let Self::Coming(pending) = self else {
            return;
        };
        let Some(face) = pending.take() else {
            return;
        };
        install(ctx, face);
        *self = Self::Settled;
    }
}

/// Reading a whole font collection is the one blocking thing this app does off its own thread: one
/// call at startup, over long before anything else wants that thread.
#[cfg(not(target_family = "wasm"))]
async fn face() -> Option<(Vec<u8>, u32)> {
    installed()
}

#[cfg(target_family = "wasm")]
async fn face() -> Option<(Vec<u8>, u32)> {
    // Index zero: what is served is a single face rather than a collection.
    download().await.map(|face| (face, 0))
}

/// Added at the lowest priority, so it is reached only for the glyphs nothing already installed
/// carries: the Latin in a Japanese sentence keeps the shape the rest of the panel is drawn in.
/// `None` leaves the panel exactly as it was.
fn install(ctx: &egui::Context, face: Option<(Vec<u8>, u32)>) {
    let Some((face, index)) = face else {
        log::warn!("no Japanese font: Japanese will be drawn as empty boxes");
        return;
    };
    let data = egui::FontData {
        index,
        tweak: egui::FontTweak {
            y_offset_factor: lowered(ctx, &face, index),
            ..egui::FontTweak::default()
        },
        ..egui::FontData::from_owned(face)
    };
    ctx.add_font(egui::epaint::text::FontInsert::new(
        NAME,
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

/// How far the face has to be moved for its baseline to land on the panel's own, as a fraction of
/// the font size. Positive is downwards, per [`egui::FontTweak::y_offset_factor`].
///
/// egui centres the faces in a family rather than aligning their baselines, which suits the emoji
/// faces it ships and not a second text face: a Japanese face reserves far more of its line above
/// the baseline than a Latin one -- Noto Sans CJK JP asks 1.16 of the font size where Ubuntu Light
/// asks 0.93 -- so centring drops the Japanese a full point below the Latin beside it, visible in a
/// line like `ここへ: Chainsaw が必要。`.
///
/// Measured rather than guessed at, the face differing per platform. Both sides are measured
/// against the proportional family, which is what the panel is drawn in; the face is inserted
/// into the monospace family too, whose own first face sits a sixth of a point differently, which
/// is not worth a second copy of the face to correct.
///
/// Zero if either side cannot be measured, which leaves the placement exactly as egui had it.
fn lowered(ctx: &egui::Context, face: &[u8], index: u32) -> f32 {
    /// The one quantity the two sides are comparable in: egui's centring aligns the middles of
    /// the lines, so what is left over is the difference between the baselines' distances from
    /// them. Scaled to the font size.
    fn from_middle(ascent: f32, line: f32) -> f32 {
        ascent - line / 2.0
    }

    let panel = {
        // Asked of egui rather than read off the file egui happens to be built with, so this
        // stays right if the panel is ever given a different Latin face. A glyph carries the
        // metrics of the family it was placed against.
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

        // Unscaled and divided by the em, which is how egui reads these too, so that the two
        // sides are comparable.
        let Ok(font) = skrifa::FontRef::from_index(face, index) else {
            return 0.0;
        };
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let line = metrics.ascent - metrics.descent + metrics.leading;
        from_middle(metrics.ascent, line) / metrics.units_per_em as f32
    };

    panel - japanese
}

/// The Japanese interface face each platform is expected to have, best first.
///
/// Named rather than searched for, because a search chooses between fonts that are all readable
/// and only one of which is what the rest of the system draws Japanese in. Every name is a
/// Japanese face -- the panel already has a Latin one this is inserted underneath -- so Windows
/// is asked for Yu Gothic UI, the Japanese of the Segoe UI it draws its own interface in.
///
/// Each list ends in what the platform had before the face it has now, so a device a version or
/// two behind is still answered. Nothing here is guaranteed to exist, which is why the list is
/// walked rather than indexed: a missing name is not found, and an installed one that cannot draw
/// Japanese is rejected by [`SAMPLE`].
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
/// Linux and the BSDs, which have no face of their own and carry whichever the distribution
/// packaged.
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

/// The system font family Japanese is drawn with, and which face of it. Three ways of asking, each
/// falling through to the next, and every candidate having to draw [`SAMPLE`] before it is taken:
///
/// 1. [`PREFERRED`], the face the platform draws its own Japanese in.
/// 2. The platform's script fallback, for a device carrying a Japanese font this app has never
///    heard of. Asked by `Hira` rather than `Jpan`: the composite code resolves to a Latin font
///    with no kana in it, and the kana code is what the underlying font databases key on.
/// 3. Every family the system has, sorted only so that it is the same last resort every run, the
///    names coming out of a hash map.
///
/// A pan-CJK font reached by 2 or 3 may show the Chinese or Korean glyph shapes a Japanese reader
/// will notice, one file covering all four languages and only the platform knowing which of its
/// faces was meant. Those two are choosing between a font this app can read and none at all.
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
    let asked: Vec<_> = collection
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

    named.into_iter().chain(asked).chain(every).find_map(|id| {
        let family = collection.family(id)?;
        let font = family.default_font()?;
        let blob = font.load(Some(&mut sources))?;
        let charmap = font.charmap_index().charmap(blob.as_ref())?;
        SAMPLE
            .iter()
            .all(|&glyph| charmap.map(glyph).is_some())
            .then(|| {
                log::info!("Japanese drawn with {:?}", family.name());
                // Copied out of the mapping the system holds it in: egui keeps a font's bytes for
                // as long as it draws with them, and borrowing out of that mapping needs unsafe.
                (blob.as_ref().to_vec(), font.index())
            })
    })
}

/// Noto Sans JP, pinned, off a host that serves it cross-origin.
///
/// The whole face rather than the glyphs this app can name in advance: a subset of exactly the
/// world names in `data.json` is a tenth of the size, but also an asset to build and keep in step
/// with every refresh of the dump, where this is one download the browser then keeps.
#[cfg(target_family = "wasm")]
const URL: &str = "https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf";

/// `None` leaves the panel drawing the boxes it was already drawing.
#[cfg(target_family = "wasm")]
async fn download() -> Option<Vec<u8>> {
    let response = super::fetch::client()
        .get(URL)
        .send()
        .await
        .inspect_err(|error| log::warn!("cannot reach the Japanese font: {error}"))
        .ok()?
        .error_for_status()
        .inspect_err(|error| log::warn!("{URL} does not serve a font: {error}"))
        .ok()?;
    let face = response
        .bytes()
        .await
        .inspect_err(|error| log::warn!("the Japanese font did not arrive whole: {error}"))
        .ok()?;
    Some(face.into())
}
