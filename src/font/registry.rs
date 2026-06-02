use std::sync::Arc;
use std::sync::OnceLock;

use fontdb::{Database, Family, Query, Source, Style, Weight};
use harfrust::{FontRef, ShaperData};

use crate::types::FontFaceId;

/// Byte container owned by a [`FontEntry`]. Erased behind a trait
/// object so the registry can hold any of `Vec<u8>`, `memmap2::Mmap`,
/// or `&'static [u8]` (via a wrapper) without copying. fontdb's
/// `Source::Binary` uses the same shape, so the same `Arc` can be
/// shared with the database.
pub type SharedFontData = Arc<dyn AsRef<[u8]> + Sync + Send>;

pub struct FontEntry {
    pub fontdb_id: fontdb::ID,
    pub face_index: u32,
    pub data: SharedFontData,
    pub swash_cache_key: swash::CacheKey,
    /// Lazily-built HarfBuzz shaping tables for this face. `ShaperData`
    /// preprocesses the font's GSUB/GPOS/cmap tables and is independent
    /// of any `FontRef` lifetime, so it can be built once and reused
    /// across every shape call. Built on first shape via
    /// [`shaper_data`](Self::shaper_data). `OnceLock` keeps `FontEntry`
    /// `Sync` (its internal caches are atomic).
    shaper_data: OnceLock<ShaperData>,
}

impl FontEntry {
    /// Borrow the underlying font bytes. Two `as_ref()` hops:
    /// `Arc<dyn AsRef<[u8]>>` → `&(dyn AsRef<[u8]>)` → `&[u8]`.
    pub fn bytes(&self) -> &[u8] {
        (*self.data).as_ref()
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
    generic_families: std::collections::HashMap<String, String>,
    default_font: Option<FontFaceId>,
    default_size_px: f32,
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FontRegistry {
    pub fn new() -> Self {
        Self {
            fontdb: Database::new(),
            fonts: Vec::new(),
            generic_families: std::collections::HashMap::new(),
            default_font: None,
            default_size_px: 16.0,
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
                data: data.clone(),
                swash_cache_key,
                shaper_data: OnceLock::new(),
            };

            let face_id = FontFaceId(self.fonts.len() as u32);
            self.fonts.push(Some(entry));
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
                    data: data.clone(),
                    swash_cache_key,
                    shaper_data: OnceLock::new(),
                };

                let face_id = FontFaceId(self.fonts.len() as u32);
                self.fonts.push(Some(entry));
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
        self.fonts.iter().enumerate().find_map(|(i, entry)| {
            entry
                .as_ref()
                .filter(|e| e.fontdb_id == fontdb_id)
                .map(|_| FontFaceId(i as u32))
        })
    }

    /// Iterate all registered font entries for glyph fallback.
    pub fn all_entries(&self) -> impl Iterator<Item = (FontFaceId, &FontEntry)> {
        self.fonts
            .iter()
            .enumerate()
            .filter_map(|(i, opt)| opt.as_ref().map(|entry| (FontFaceId(i as u32), entry)))
    }
}
