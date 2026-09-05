//! The Japanese face, which nothing this app starts with has and everything Japanese needs.
//!
//! egui's own fonts carry Latin and little else, so Japanese -- the messages, and the Japanese
//! name the wiki publishes for nearly every world -- is drawn as the empty boxes of a glyph no
//! installed font carries. A face that has the glyphs is a few megabytes wherever it comes from,
//! so it is not compiled in; it is sent for as the app starts and installed the frame it turns
//! up.
//!
//! Sent for whatever language the run is in, because the language is not what decides whether
//! Japanese is drawn. Nearly every world carries the Japanese name the wiki publishes for it, and
//! an English run shows those names beside the English ones. Waiting for the settings tab would
//! mean drawing them as boxes until somebody happened to open it.
//!
//! Nothing waits for it either. It is fetched on [`super::fetch`]'s executor like anything else
//! that would otherwise stall a frame, so the panel is drawn in whatever it already has and gains
//! the glyphs partway through the first second.
//!
//! Where it comes from is the whole of what the platforms differ in. A device has one already --
//! Android and Windows and macOS always, a desktop Linux with any CJK font installed -- and
//! [`installed`] asks for it by name, from a list of what each platform is expected to have. A
//! page has no fonts to ask about and downloads one instead.

/// The name egui knows the face by. Only ever added, never looked up: it is a fallback rather
/// than a family anything asks for by name.
const NAME: &str = "japanese";

/// The face, and how far along getting hold of it this run has got.
pub(super) enum Japanese {
    /// On its way, on whatever executor this platform has. Started as the app is built, so this
    /// is what every run opens in.
    Coming(super::fetch::Pending<Option<(Vec<u8>, u32)>>),
    /// Installed, or looked for and not found. Either way there is nothing further to do: a
    /// device with no Japanese font draws the boxes, and there is no second place to look.
    Settled,
}

impl Japanese {
    /// Sends for the face. Nothing is loaded here -- this only starts the work and hands back the
    /// slot its answer will land in.
    pub(super) fn new() -> Self {
        Self::Coming(super::fetch::spawn(face()))
    }

    /// Installs the face the frame it arrives.
    ///
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

/// Reads the face out of the system's own fonts.
///
/// Reading a whole font collection is the one blocking thing this app does off its own thread. It
/// is a single call at startup and it is over long before anything else wants that thread, which
/// is why it is left on [`super::fetch`]'s executor rather than given one of its own.
#[cfg(not(target_family = "wasm"))]
async fn face() -> Option<(Vec<u8>, u32)> {
    installed()
}

/// Fetches the face, there being none on the page to read.
#[cfg(target_family = "wasm")]
async fn face() -> Option<(Vec<u8>, u32)> {
    // Index zero: what is served is a single face rather than a collection.
    download().await.map(|face| (face, 0))
}

/// Adds the face to every family egui draws text in, at the lowest priority.
///
/// Lowest, so it is reached only for the glyphs nothing already installed carries: the Latin in a
/// Japanese sentence keeps the shape the rest of the panel is drawn in, and the icons keep theirs.
/// `None` is a face that could not be had, which leaves the panel exactly as it was.
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
/// the font size. Positive is downwards, which is what [`egui::FontTweak::y_offset_factor`] means.
///
/// egui does not align the baselines of the faces in a family. It centres them: a glyph is placed
/// at its own face's ascent, plus half of however much shorter that face's line is than the
/// family's. That is the right thing for the emoji faces egui ships, which have no baseline worth
/// speaking of, and the wrong thing for a second text face. A Japanese face reserves far more of
/// its line above the baseline than a Latin one does -- Noto Sans CJK JP asks for 1.16 of the font
/// size where Ubuntu Light, which egui draws the rest of the panel in, asks for 0.93 -- so
/// centring the two drops the Japanese a full point below the Latin beside it, which is the step
/// visible in a line like `ここへ: Chainsaw が必要。`.
///
/// So the offset is measured rather than guessed, once, on the face this device turned out to
/// have: each side reports where it puts its baseline within its own line, and the difference is
/// what closes the step. Guessing is not open to us anyway, since the face differs per platform
/// and each of them has its own metrics.
///
/// Both sides are measured against the proportional family, which is what the panel is drawn in.
/// The face is inserted into the monospace family as well, whose own first face sits a fraction
/// differently; the sixth of a point that leaves behind is not worth a second copy of the face to
/// correct.
///
/// Zero if either side cannot be measured, which leaves the placement exactly as egui had it.
fn lowered(ctx: &egui::Context, face: &[u8], index: u32) -> f32 {
    /// Where a face puts its baseline, measured down from the middle of the line it asks for and
    /// scaled to the font size. The one quantity the two sides can be compared in: egui's
    /// centring aligns the middles of the lines, so what is left over is the difference between
    /// the baselines' distances from those middles.
    fn from_middle(ascent: f32, line: f32) -> f32 {
        ascent - line / 2.0
    }

    let panel = {
        // Asked of egui rather than read off the file egui happens to be built with, so this stays
        // right if the panel is ever given a different face to draw Latin in. A glyph carries the
        // metrics of the family it was placed against, and one already-drawn letter at the size
        // the panel is already drawing costs nothing to lay out again.
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

        // Unscaled and divided by the em, which is how egui reads these too: the same three
        // numbers, in the same units, so that the two sides are comparable.
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
/// Named rather than searched for, because the choice a search leaves is between fonts that are
/// all readable and only one of which is the one the rest of the system draws Japanese in. Every
/// name here is a Japanese face: a platform's Latin interface font is not one and is not wanted,
/// since the panel already has a Latin face and this one is inserted underneath it. Windows draws
/// its own interface in Segoe UI, whose Japanese is Yu Gothic UI, and it is Yu Gothic UI that is
/// asked for here.
///
/// Each list ends in what the platform had before the face it has now, so a device a version or
/// two behind is still answered. Nothing here is guaranteed to exist -- a name that is not
/// installed is simply not found, and a name that is installed but cannot draw Japanese is
/// rejected by [`SAMPLE`] -- which is why the list is walked rather than indexed.
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
/// Linux and the BSDs, which have no face of their own and carry whichever of these the
/// distribution chose to package.
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

/// The system font family Japanese is drawn with, and which face of it.
///
/// Three ways of asking, each one falling through to the next, and every candidate having to draw
/// [`SAMPLE`] before it is taken:
///
/// 1. [`PREFERRED`], which is the face the platform draws its own Japanese in. This is the answer
///    everywhere the platform has an opinion, and the point of asking by name.
/// 2. The platform's script fallback, for a device carrying a Japanese font this app has never
///    heard of. Asked for by `Hira` rather than by `Jpan`: the composite code resolves to a Latin
///    font with no kana in it, and the kana code is what the underlying font databases key on.
/// 3. Every family the system has, sorted. A last resort, and sorted only so that it is the same
///    last resort every run: the names come out of a hash map, so an unsorted sweep picks a
///    different font each time the app starts -- a serif one as readily as the interface one.
///
/// A pan-CJK font reached by 2 or 3 is a font whose Chinese or Korean glyph shapes a Japanese
/// reader will notice, since one file covers all four languages and only the platform knows which
/// of its faces was meant. That is a font this app can read rather than one it cannot, which is
/// the whole of what those two are deciding.
#[cfg(not(target_family = "wasm"))]
fn installed() -> Option<(Vec<u8>, u32)> {
    use fontique::{Collection, CollectionOptions, FallbackKey, Script, SourceCache};

    /// Enough Japanese to tell a font that can draw it from one that cannot: one kana, which no
    /// Latin font carries, and two of the kanji every CJK font does.
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
                // Copied out of the mapping the system holds it in: egui keeps the bytes of a
                // font it is given for as long as it draws with them, and there is no
                // borrowing them out of that mapping without unsafe.
                (blob.as_ref().to_vec(), font.index())
            })
    })
}

/// Where the page gets the face from: Noto Sans JP, at a pinned release, off a host that serves
/// it to another origin.
///
/// The whole face rather than the glyphs this app can name in advance. A subset of exactly the
/// world names in `data.json` is a tenth of the size, but it is also an asset to build and to
/// keep in step with every refresh of the dump, and this is a download a reader makes once and
/// the browser then keeps.
#[cfg(target_family = "wasm")]
const URL: &str = "https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf";

/// Fetches it, or reports why not. `None` leaves the panel drawing boxes, which is what it was
/// already drawing.
#[cfg(target_family = "wasm")]
async fn download() -> Option<Vec<u8>> {
    // Its own client, built inside the request so it lands on the executor [`fetch::spawn`] put
    // this on. The same arrangement, for the same reason, as `detail::download`.
    let response = reqwest::Client::new()
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
