//! Off-thread writing-system coverage indexing.
//!
//! Classifying a font's writing systems parses its OS/2 table, which means
//! reading the font file's bytes. Doing that for every installed face is
//! hundreds of `std::fs::read` + parse calls — far too much for the UI
//! thread. [`WritingSystemIndexBuilder`] is a `Send` snapshot of every
//! family's byte-sources, built cheaply on the main thread (it clones paths
//! and shares `Arc`s but reads nothing), then moved to a worker thread
//! where [`build`](WritingSystemIndexBuilder::build) does the expensive
//! parsing and returns a family → coverage map.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use super::registry::SharedFontData;
use super::writing_system::{WritingSystemSet, writing_systems_for_face};

/// Where a face's bytes come from, in a form that can cross threads.
pub(crate) enum FaceBytes {
    /// A font file on disk (system fonts) — read lazily on the worker
    /// thread so the snapshot itself stays cheap.
    Path(PathBuf),
    /// Already-resident bytes (explicitly registered fonts).
    Shared(SharedFontData),
}

/// One face's byte-source plus its index within a font collection.
pub(crate) struct FaceRef {
    pub(crate) bytes: FaceBytes,
    pub(crate) index: u32,
}

/// A `Send` snapshot of every family's face byte-sources, ready to be moved
/// to a background thread and turned into a writing-system coverage map.
///
/// Built by
/// [`FontRegistry::writing_system_index_builder`](crate::font::registry::FontRegistry::writing_system_index_builder).
pub struct WritingSystemIndexBuilder {
    families: Vec<(String, Vec<FaceRef>)>,
}

impl WritingSystemIndexBuilder {
    pub(crate) fn new(families: Vec<(String, Vec<FaceRef>)>) -> Self {
        Self { families }
    }

    /// Number of distinct families captured in the snapshot.
    pub fn family_count(&self) -> usize {
        self.families.len()
    }

    /// Compute the writing-system coverage of every family, unioning across
    /// its faces. Reads and parses each face's bytes — the expensive path,
    /// meant to run off the UI thread.
    pub fn build(self) -> HashMap<String, WritingSystemSet> {
        let mut out = HashMap::with_capacity(self.families.len());
        for (family, faces) in self.families {
            let mut set = WritingSystemSet::new();
            for face in &faces {
                let coverage = match &face.bytes {
                    FaceBytes::Path(path) => match std::fs::read(path) {
                        Ok(bytes) => writing_systems_for_face(&bytes, face.index),
                        Err(_) => WritingSystemSet::new(),
                    },
                    FaceBytes::Shared(data) => {
                        writing_systems_for_face((**data).as_ref(), face.index)
                    }
                };
                set = set.union(coverage);
            }
            out.insert(family, set);
        }
        out
    }
}

/// Helper for [`FontRegistry::writing_system_index_builder`]: group per-face
/// `(family, FaceRef)` pairs into the builder's family-keyed shape.
///
/// [`FontRegistry::writing_system_index_builder`]: crate::font::registry::FontRegistry::writing_system_index_builder
pub(crate) fn build_from_faces(
    faces: impl IntoIterator<Item = (String, FaceRef)>,
) -> WritingSystemIndexBuilder {
    let mut grouped: BTreeMap<String, Vec<FaceRef>> = BTreeMap::new();
    for (family, face) in faces {
        grouped.entry(family).or_default().push(face);
    }
    WritingSystemIndexBuilder::new(grouped.into_iter().collect())
}
