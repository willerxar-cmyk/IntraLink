//! Immutable, bounded presentation snapshots of an ordered decoded canvas.
use std::{fmt, sync::Arc};

const MAX_WIDTH: usize = 1920;
const MAX_HEIGHT: usize = 1080;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    InvalidMetadata,
    InvalidPixelCount,
    Invalidated,
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata => f.write_str("Invalid or out-of-order presentation metadata"),
            Self::InvalidPixelCount => {
                f.write_str("Decoded pixel count does not match the surface")
            }
            Self::Invalidated => {
                f.write_str("Presentation surface was invalidated by a decode failure")
            }
        }
    }
}

impl std::error::Error for SurfaceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Metadata {
    sequence: usize,
    dimensions: (usize, usize),
}

/// A snapshot may outlive the surface lock; its pixels never change.
#[derive(Debug)]
pub struct Snapshot {
    metadata: Metadata,
    pixels: Arc<Vec<u32>>,
}

impl Snapshot {
    pub fn sequence(&self) -> usize {
        self.metadata.sequence
    }
    pub fn dimensions(&self) -> (usize, usize) {
        self.metadata.dimensions
    }
    pub fn pixels(&self) -> &[u32] {
        self.pixels.as_slice()
    }
}

/// One ordered decoder owns this surface. The presenter may hold one transient
/// snapshot while drawing. Copy-on-write occurs only if such a snapshot still
/// shares the current canvas when the next update begins.
#[derive(Debug, Default)]
pub struct PresentationSurface {
    metadata: Option<Metadata>,
    pixels: Arc<Vec<u32>>,
    invalidated: bool,
}

impl PresentationSurface {
    /// Apply a fully validated, ordered record. Gaps are allowed for repeat
    /// records. An actual decode failure invalidates this stream until restart.
    pub fn update<E: From<SurfaceError>>(
        &mut self,
        sequence: usize,
        dimensions: (usize, usize),
        decode: impl FnOnce(&mut Vec<u32>) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.invalidated {
            return Err(SurfaceError::Invalidated.into());
        }
        let (width, height) = dimensions;
        if sequence == 0
            || !(1..=MAX_WIDTH).contains(&width)
            || !(1..=MAX_HEIGHT).contains(&height)
            || self
                .metadata
                .is_some_and(|old| sequence <= old.sequence || dimensions != old.dimensions)
        {
            return Err(SurfaceError::InvalidMetadata.into());
        }
        let pixels = Arc::make_mut(&mut self.pixels);
        self.metadata = None;
        // A partial failed decode is never made available to the presenter.
        self.invalidated = true;
        decode(pixels)?;
        if pixels.len() != width * height {
            return Err(SurfaceError::InvalidPixelCount.into());
        }
        self.metadata = Some(Metadata {
            sequence,
            dimensions,
        });
        self.invalidated = false;
        Ok(())
    }

    pub fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    /// Skip already-presented sequences without acquiring another Arc reference.
    pub fn snapshot_after(&self, sequence: usize) -> Option<Snapshot> {
        let metadata = self.metadata.filter(|meta| meta.sequence > sequence)?;
        Some(Snapshot {
            metadata,
            pixels: self.pixels.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(
        surface: &mut PresentationSurface,
        sequence: usize,
        value: u32,
    ) -> Result<(), SurfaceError> {
        surface.update(sequence, (2, 2), |pixels| {
            pixels.resize(4, 0);
            pixels.fill(value);
            Ok(())
        })
    }

    #[test]
    fn allocation_is_reused_when_no_snapshot_is_held() {
        let mut surface = PresentationSurface::default();
        fill(&mut surface, 1, 1).unwrap();
        let before = surface.snapshot_after(0).unwrap().pixels().as_ptr();
        fill(&mut surface, 2, 2).unwrap();
        let after = surface.snapshot_after(1).unwrap();
        assert_eq!(after.pixels().as_ptr(), before);
        assert_eq!(after.pixels(), &[2; 4]);
    }

    #[test]
    fn concurrent_presentation_keeps_immutable_pixels() {
        let mut surface = PresentationSurface::default();
        fill(&mut surface, 1, 10).unwrap();
        let old = surface.snapshot_after(0).unwrap();
        surface
            .update::<SurfaceError>(3, (2, 2), |pixels| {
                pixels[0] = 20;
                Ok(())
            })
            .unwrap();
        let new = surface.snapshot_after(1).unwrap();
        assert_eq!(old.pixels(), &[10; 4]);
        assert_eq!(new.pixels(), &[20, 10, 10, 10]);
        assert_eq!(new.sequence(), 3);
        assert_eq!(new.dimensions(), (2, 2));
        assert_ne!(old.pixels().as_ptr(), new.pixels().as_ptr());
    }

    #[test]
    fn unchanged_or_older_sequence_never_creates_a_snapshot() {
        let mut surface = PresentationSurface::default();
        assert!(surface.snapshot_after(0).is_none());
        fill(&mut surface, 1, 1).unwrap();
        assert!(surface.snapshot_after(1).is_none());
        assert!(surface.snapshot_after(2).is_none());
        assert_eq!(fill(&mut surface, 1, 2), Err(SurfaceError::InvalidMetadata));
        assert_eq!(surface.snapshot_after(0).unwrap().pixels(), &[1; 4]);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_decoding() {
        let mut surface = PresentationSurface::default();
        for dimensions in [
            (0, 2),
            (2, 0),
            (1921, 1080),
            (1920, 1081),
            (usize::MAX, usize::MAX),
        ] {
            assert_eq!(
                surface.update::<SurfaceError>(1, dimensions, |_| panic!("Invalid geometry")),
                Err(SurfaceError::InvalidMetadata)
            );
        }
        fill(&mut surface, 1, 1).unwrap();
        assert_eq!(
            surface.update::<SurfaceError>(2, (4, 1), |_| panic!("Geometry changed")),
            Err(SurfaceError::InvalidMetadata)
        );
    }

    #[test]
    fn failed_decode_poisoning_preserves_old_snapshot_but_hides_partial_update() {
        let mut surface = PresentationSurface::default();
        fill(&mut surface, 1, 1).unwrap();
        let old = surface.snapshot_after(0).unwrap();
        let failure = surface.update(2, (2, 2), |pixels| {
            pixels[0] = 3;
            Err(SurfaceError::InvalidPixelCount)
        });
        assert_eq!(failure, Err(SurfaceError::InvalidPixelCount));
        assert!(surface.snapshot_after(0).is_none());
        assert!(surface.is_invalidated());
        assert_eq!(old.pixels(), &[1; 4]);
        assert_eq!(fill(&mut surface, 3, 3), Err(SurfaceError::Invalidated));
    }

    #[test]
    fn short_decode_cannot_be_published() {
        let mut surface = PresentationSurface::default();
        assert_eq!(
            surface.update::<SurfaceError>(1, (2, 2), |pixels| {
                pixels.push(1);
                Ok(())
            }),
            Err(SurfaceError::InvalidPixelCount)
        );
        assert!(surface.snapshot_after(0).is_none());
    }
}
