use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use fontdb::{Database, Family, Query, Source, Style, Weight};
use harfrust::{FontRef, ShaperData};

use crate::font::writing_system_index::{
    FaceBytes, FaceRef, WritingSystemIndexBuilder, build_from_faces,
};
use crate::types::FontFaceId;

/// Summary of one installed font family, for building a font picker.
///
/// `name` is the primary (English, where available) family name; faces of
/// different weights/styles are collapsed into one entry. `monospaced` is
/// true when any face of the family is monospaced — read straight from
/// fontdb metadata, so producing a whole [`Vec<FontFamilyInfo>`] loads no
/// font bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFamilyInfo {
    /// Primary family name (e.g. `"DejaVu Sans"`).
    pub name: String,
    /// True if any face of the family is monospaced.
    pub monospaced: bool,
}

/// Byte container owned by a [`FontEntry`]. Erased behind a trait
/// object so the registry can hold any of `Vec<u8>`, `memmap2::Mmap`,
/// or `&'static [u8]` (via a wrapper) without copying. fontdb's
/// `Source::Binary` uses the same shape, so the same `Arc` can be
/// shared with the database.
pub type SharedFontData = Arc<dyn AsRef<[u8]> + Sync + Send>;

/// Backing bytes for a [`FontEntry`].
///
/// Explicitly registered fonts are [`Resident`](FontData::Resident) — the
/// caller handed us the bytes. System fonts discovered by
/// [`load_system_fonts`](FontRegistry::new) are [`Lazy`](FontData::Lazy):
/// we keep only the file path until the face is actually shaped or queried
/// for coverage, then read and cache it. This keeps `new()` cheap even
/// when the OS has hundreds of installed faces.
enum FontData {
    Resident(SharedFontData),
    Lazy {
        path: PathBuf,
        cell: OnceLock<SharedFontData>,
    },
}

pub struct FontEntry {
    pub fontdb_id: fontdb::ID,
    pub face_index: u32,
    data: FontData,
    pub swash_cache_key: swash::CacheKey,
    /// True for faces discovered via the OS font enumeration. Glyph
    /// fallback prefers explicitly-registered faces and only reaches for
    /// system faces as a last resort (see
    /// [`find_fallback_font`](crate::font::resolve::find_fallback_font)).
    pub is_system: bool,
    /// Lazily-built HarfBuzz shaping tables for this face. `ShaperData`
    /// preprocesses the font's GSUB/GPOS/cmap tables and is independent
    /// of any `FontRef` lifetime, so it can be built once and reused
    /// across every shape call. Built on first shape via
    /// [`shaper_data`](Self::shaper_data). `OnceLock` keeps `FontEntry`
    /// `Sync` (its internal caches are atomic).
    shaper_data: OnceLock<ShaperData>,
}

impl FontEntry {
    /// Borrow the underlying font bytes, loading them from disk on first
    /// use for lazily-tracked system faces. Returns an empty slice if a
    /// system font file can no longer be read (uninstalled mid-session);
    /// shaping then degrades gracefully to `None`/`.notdef`.
    pub fn bytes(&self) -> &[u8] {
        match &self.data {
            // Two `as_ref()` hops: Arc<dyn AsRef<[u8]>> → dyn → &[u8].
            FontData::Resident(data) => (**data).as_ref(),
            FontData::Lazy { path, cell } => {
                let arc = cell.get_or_init(|| {
                    let bytes = std::fs::read(path).unwrap_or_default();
                    Arc::new(bytes) as SharedFontData
                });
                (**arc).as_ref()
            }
        }
    }

    /// Borrow this face's shaping tables, building them on first use.
    ///
    /// `ShaperData::new` reprocesses the font tables; doing it once per
    /// face (instead of once per shape call) avoids redundant work on
    /// every relayout/keystroke. The `font` must refer to this same
    /// face — callers already hold a `FontRef` opened from `bytes()`.
    pub fn shaper_data(&self, font: &FontRef) -> &ShaperData {
        self.shaper_data.get_or_init(|| ShaperData::new(font))
    }
}

pub struct FontRegistry {
    fontdb: Database,
    fonts: Vec<Option<FontEntry>>,
    /// fontdb face ID → our `FontFaceId`, so `query_font` doesn't scan
    /// `fonts` linearly (it can hold hundreds of system faces).
    fontdb_index: HashMap<fontdb::ID, FontFaceId>,
    generic_families: HashMap<String, String>,
    default_font: Option<FontFaceId>,
    default_size_px: f32,
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FontRegistry {
    /// Create a registry pre-populated with the operating system's fonts.
    ///
    /// OS faces participate in family lookup and glyph fallback so that
    /// arbitrary documents (CJK, emoji, scripts the host didn't bundle)
    /// still render. Their bytes load lazily on first use, but the
    /// enumeration itself scans the OS font directories — a one-time
    /// startup cost. Use [`new_without_system_fonts`](Self::new_without_system_fonts)
    /// when the host ships a controlled font set and wants neither the
    /// scan nor implicit OS fonts.
    pub fn new() -> Self {
        let mut registry = Self::new_without_system_fonts();
        registry.load_system_fonts();
        registry
    }

    /// Create an empty registry with no system fonts. Fonts enter only
    /// via the `register_font*` methods.
    pub fn new_without_system_fonts() -> Self {
        Self {
            fontdb: Database::new(),
            fonts: Vec::new(),
            fontdb_index: HashMap::new(),
            generic_families: HashMap::new(),
            default_font: None,
            default_size_px: 16.0,
        }
    }

    /// Enumerate OS fonts into `fontdb` and register a lazy [`FontEntry`]
    /// for each so they join family lookup and the fallback chain. Bytes
    /// load on first access; only the file path is kept until then.
    fn load_system_fonts(&mut self) {
        self.fontdb.load_system_fonts();

        // Collect first: `faces()` borrows `self.fontdb` immutably while
        // the loop needs `&mut self` to push entries.
        let discovered: Vec<(fontdb::ID, u32, PathBuf)> = self
            .fontdb
            .faces()
            .filter_map(|f| match &f.source {
                Source::File(path) => Some((f.id, f.index, path.clone())),
                Source::SharedFile(path, _) => Some((f.id, f.index, path.clone())),
                Source::Binary(_) => None,
            })
            .collect();

        for (fontdb_id, face_index, path) in discovered {
            if self.fontdb_index.contains_key(&fontdb_id) {
                continue;
            }
            let entry = FontEntry {
                fontdb_id,
                face_index,
                data: FontData::Lazy {
                    path,
                    cell: OnceLock::new(),
                },
                swash_cache_key: swash::CacheKey::new(),
                is_system: true,
                shaper_data: OnceLock::new(),
            };
            let face_id = FontFaceId(self.fonts.len() as u32);
            self.fonts.push(Some(entry));
            self.fontdb_index.insert(fontdb_id, face_id);
        }
    }

    /// Register a font from raw bytes. Returns IDs for all faces found
    /// (font collections may contain multiple faces).
    ///
    /// This copies the bytes into an owned `Vec<u8>`. For large fonts
    /// where the caller already holds the data in a shareable
    /// container (e.g. an `Arc<Mmap>` for a system emoji font), use
    /// [`register_font_shared`](Self::register_font_shared) instead to
    /// avoid the copy.
    pub fn register_font(&mut self, data: &[u8]) -> Vec<FontFaceId> {
        let arc_data: SharedFontData = Arc::new(data.to_vec());
        self.register_font_shared(arc_data)
    }

    /// Register a font from a pre-built shared byte container,
    /// avoiding the copy that [`register_font`](Self::register_font)
    /// would perform.
    ///
    /// The shared container is held by every resulting [`FontEntry`]
    /// and by fontdb's `Source::Binary`. Drop it after the registry
    /// drops the entries.
    pub fn register_font_shared(&mut self, data: SharedFontData) -> Vec<FontFaceId> {
        let source = Source::Binary(data.clone());
        let fontdb_ids = self.fontdb.load_font_source(source);

        let mut face_ids = Vec::new();
        for fontdb_id in fontdb_ids {
            let face_index = self.fontdb.face(fontdb_id).map(|f| f.index).unwrap_or(0);

            let swash_cache_key = swash::CacheKey::new();
            let entry = FontEntry {
                fontdb_id,
                face_index,
                data: FontData::Resident(data.clone()),
                swash_cache_key,
                is_system: false,
                shaper_data: OnceLock::new(),
            };

            let face_id = FontFaceId(self.fonts.len() as u32);
            self.fonts.push(Some(entry));
            self.fontdb_index.insert(fontdb_id, face_id);
            face_ids.push(face_id);
        }
        face_ids
    }

    /// Register a font with explicit metadata, overriding the font's name table.
    pub fn register_font_as(
        &mut self,
        data: &[u8],
        family: &str,
        weight: u16,
        italic: bool,
    ) -> Vec<FontFaceId> {
        let arc_data: SharedFontData = Arc::new(data.to_vec());
        self.register_font_shared_as(arc_data, family, weight, italic)
    }

    /// Like [`register_font_as`](Self::register_font_as) but takes a
    /// pre-built shared byte container, avoiding the copy.
    pub fn register_font_shared_as(
        &mut self,
        data: SharedFontData,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> Vec<FontFaceId> {
        let source = Source::Binary(data.clone());
        let fontdb_ids = self.fontdb.load_font_source(source);

        let mut face_ids = Vec::new();
        for fontdb_id in fontdb_ids {
            // Override metadata in fontdb
            if let Some(face_info) = self.fontdb.face(fontdb_id) {
                let mut info = face_info.clone();
                info.families = vec![(family.to_string(), fontdb::Language::English_UnitedStates)];
                info.weight = Weight(weight);
                info.style = if italic { Style::Italic } else { Style::Normal };
                // Remove old and re-add with new metadata
                let face_index = info.index;
                self.fontdb.remove_face(fontdb_id);
                let new_id = self.fontdb.push_face_info(info);

                let swash_cache_key = swash::CacheKey::new();
                let entry = FontEntry {
                    fontdb_id: new_id,
                    face_index,
                    data: FontData::Resident(data.clone()),
                    swash_cache_key,
                    is_system: false,
                    shaper_data: OnceLock::new(),
                };

                let face_id = FontFaceId(self.fonts.len() as u32);
                self.fonts.push(Some(entry));
                self.fontdb_index.insert(new_id, face_id);
                face_ids.push(face_id);
            }
        }
        face_ids
    }

    pub fn set_default_font(&mut self, face: FontFaceId, size_px: f32) {
        self.default_font = Some(face);
        self.default_size_px = size_px;
    }

    pub fn default_font(&self) -> Option<FontFaceId> {
        self.default_font
    }

    pub fn default_size_px(&self) -> f32 {
        self.default_size_px
    }

    pub fn set_generic_family(&mut self, generic: &str, family: &str) {
        self.generic_families
            .insert(generic.to_string(), family.to_string());
    }

    /// Resolve a family name, mapping generic names (serif, monospace, etc.)
    /// to their configured concrete family names.
    pub fn resolve_family_name<'a>(&'a self, family: &'a str) -> &'a str {
        self.generic_families
            .get(family)
            .map(|s| s.as_str())
            .unwrap_or(family)
    }

    /// Query fontdb for a font matching the given criteria.
    pub fn query_font(&self, family: &str, weight: u16, italic: bool) -> Option<FontFaceId> {
        let resolved = self.resolve_family_name(family);
        let style = if italic { Style::Italic } else { Style::Normal };

        let query = Query {
            families: &[Family::Name(resolved)],
            weight: Weight(weight),
            style,
            ..Query::default()
        };

        let fontdb_id = self.fontdb.query(&query)?;
        self.fontdb_id_to_face_id(fontdb_id)
    }

    /// Look up the family name of a registered font.
    pub fn font_family_name(&self, face_id: FontFaceId) -> Option<String> {
        let entry = self.get(face_id)?;
        let face_info = self.fontdb.face(entry.fontdb_id)?;
        face_info.families.first().map(|(name, _)| name.clone())
    }

    /// Look up a FontEntry by FontFaceId.
    pub fn get(&self, face_id: FontFaceId) -> Option<&FontEntry> {
        self.fonts
            .get(face_id.0 as usize)
            .and_then(|opt| opt.as_ref())
    }

    /// Query for a variant (different weight/style) of an existing registered font.
    /// Looks up the family name of `base_face` and queries fontdb for a match.
    pub fn query_variant(
        &self,
        base_face: FontFaceId,
        weight: u16,
        italic: bool,
    ) -> Option<FontFaceId> {
        let entry = self.get(base_face)?;
        let face_info = self.fontdb.face(entry.fontdb_id)?;
        let family_name = face_info.families.first().map(|(name, _)| name.as_str())?;
        self.query_font(family_name, weight, italic)
    }

    /// Find our FontFaceId for a fontdb ID.
    fn fontdb_id_to_face_id(&self, fontdb_id: fontdb::ID) -> Option<FontFaceId> {
        self.fontdb_index.get(&fontdb_id).copied()
    }

    /// Enumerate every installed font family, deduplicated (case-insensitive)
    /// and sorted (case-insensitive Unicode).
    ///
    /// Cheap: it reads only fontdb metadata (family name + monospaced flag),
    /// so no font file is opened. This is the item source for a font picker.
    /// Multiple faces of one family (Regular/Bold/Italic) collapse into a
    /// single entry, whose `monospaced` is true if any face is monospaced.
    pub fn families(&self) -> Vec<FontFamilyInfo> {
        // key = lowercased name (dedupe); value = (display name, monospaced).
        let mut map: HashMap<String, (String, bool)> = HashMap::new();
        for face in self.fontdb.faces() {
            // `families[0]` is the English (US) name where present.
            let Some((name, _)) = face.families.first() else {
                continue;
            };
            let entry = map
                .entry(name.to_lowercase())
                .or_insert_with(|| (name.clone(), false));
            entry.1 |= face.monospaced;
        }
        let mut out: Vec<FontFamilyInfo> = map
            .into_values()
            .map(|(name, monospaced)| FontFamilyInfo { name, monospaced })
            .collect();
        out.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    /// Enumerate installed font family names, deduplicated and sorted — the
    /// simple projection of [`families`](Self::families).
    pub fn family_names(&self) -> Vec<String> {
        self.families().into_iter().map(|f| f.name).collect()
    }

    /// True if any face of the named family is monospaced. Reads only fontdb
    /// metadata (no bytes loaded). Case-insensitive on the family name.
    pub fn family_is_monospaced(&self, family: &str) -> bool {
        let target = family.to_lowercase();
        self.fontdb.faces().any(|f| {
            f.monospaced
                && f.families
                    .first()
                    .is_some_and(|(n, _)| n.to_lowercase() == target)
        })
    }

    /// Build a `Send` snapshot of every family's face byte-sources, ready to
    /// be moved to a background thread and turned into a writing-system
    /// coverage map (see [`WritingSystemIndexBuilder`]).
    ///
    /// Cheap on the calling thread: it clones file paths / shares `Arc`s but
    /// reads no font bytes. The expensive OS/2 parsing happens in
    /// [`WritingSystemIndexBuilder::build`], which the caller runs
    /// off-thread.
    /// The coverage map is keyed by the **lowercased** family name so it
    /// aligns with [`families`](Self::families)' case-insensitive dedup —
    /// callers look up by `name.to_lowercase()`. (Faces of one family can
    /// disagree on name casing; a case-sensitive key would split or miss.)
    pub fn writing_system_index_builder(&self) -> WritingSystemIndexBuilder {
        build_from_faces(self.fontdb.faces().filter_map(|face| {
            let (family, _) = face.families.first()?;
            let bytes = match &face.source {
                Source::File(path) => FaceBytes::Path(path.clone()),
                Source::SharedFile(_, data) => FaceBytes::Shared(data.clone()),
                Source::Binary(data) => FaceBytes::Shared(data.clone()),
            };
            Some((
                family.to_lowercase(),
                FaceRef {
                    bytes,
                    index: face.index,
                },
            ))
        }))
    }

    /// Iterate all registered font entries for glyph fallback.
    pub fn all_entries(&self) -> impl Iterator<Item = (FontFaceId, &FontEntry)> {
        self.fonts
            .iter()
            .enumerate()
            .filter_map(|(i, opt)| opt.as_ref().map(|entry| (FontFaceId(i as u32), entry)))
    }
}
