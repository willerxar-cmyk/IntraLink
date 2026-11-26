//! Validated local resource snapshots; no device identifiers or network data.

/// Keep both values private so that used bytes cannot underflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    total: u64,
    available: u64,
}

impl Memory {
    pub fn new(total: u64, available: u64) -> Option<Self> {
        (total != 0 && available <= total).then_some(Self { total, available })
    }

    pub const fn total(self) -> u64 {
        self.total
    }

    pub const fn available(self) -> u64 {
        self.available
    }

    pub const fn used(self) -> u64 {
        self.total - self.available
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cpu {
    logical_processors: u32,
    usage_percent: f32,
}

impl Cpu {
    pub fn new(logical_processors: usize, usage_percent: f32) -> Option<Self> {
        let logical_processors = u32::try_from(logical_processors).ok()?;
        if logical_processors == 0
            || !usage_percent.is_finite()
            || !(0.0..=100.0).contains(&usage_percent)
        {
            return None;
        }
        Some(Self {
            logical_processors,
            usage_percent,
        })
    }

    pub const fn logical_processors(self) -> u32 {
        self.logical_processors
    }

    pub const fn usage_percent(self) -> f32 {
        self.usage_percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_is_checked_and_used_cannot_underflow() {
        let memory = Memory::new(1024, 256).unwrap();
        assert_eq!(memory.total(), 1024);
        assert_eq!(memory.available(), 256);
        assert_eq!(memory.used(), 768);
        assert_eq!(Memory::new(0, 0), None);
        assert_eq!(Memory::new(1, 2), None);
        assert_eq!(Memory::new(1, 0).unwrap().used(), 1);
        assert_eq!(Memory::new(u64::MAX, u64::MAX).unwrap().used(), 0);
    }

    #[test]
    fn cpu_never_reports_unavailable_or_nonfinite_data_as_a_measurement() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 101.0] {
            assert_eq!(Cpu::new(4, invalid), None);
        }
        assert_eq!(Cpu::new(0, 0.0), None);
        assert_eq!(Cpu::new(8, 0.0).unwrap().usage_percent(), 0.0);
        let cpu = Cpu::new(8, 100.0).unwrap();
        assert_eq!(cpu.usage_percent(), 100.0);
        assert_eq!(cpu.logical_processors(), 8);
    }
}
