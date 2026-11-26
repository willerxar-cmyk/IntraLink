//! Version 8: bounded, read-only desktop records with a fixed credit window.

pub const ALPN: &[u8] = b"intralink/view/8";
const REQUEST_PREFIX: &[u8; 16] = b"INTRALINK/8 VIEW";
pub const REQUEST_BYTES: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecMode {
    Jpeg = 0,
    Tiles = 1,
    Compact = 2,
}

impl CodecMode {
    pub const fn supports_tiles(self) -> bool {
        matches!(self, Self::Tiles | Self::Compact)
    }

    pub const fn supports_compact(self) -> bool {
        matches!(self, Self::Compact)
    }

    pub const fn decode(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Jpeg),
            1 => Some(Self::Tiles),
            2 => Some(Self::Compact),
            _ => None,
        }
    }
}

pub fn request(mode: CodecMode) -> [u8; REQUEST_BYTES] {
    let mut bytes = [0; REQUEST_BYTES];
    bytes[..16].copy_from_slice(REQUEST_PREFIX);
    bytes[16] = mode as u8;
    bytes
}

pub fn parse_request(bytes: &[u8]) -> Option<CodecMode> {
    if bytes.len() != REQUEST_BYTES || &bytes[..16] != REQUEST_PREFIX {
        return None;
    }
    CodecMode::decode(bytes[16])
}
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SECONDS: u64 = 3600;
pub const MAX_FPS: u32 = 60;
pub const MAX_FRAMES: usize = MAX_SECONDS as usize * MAX_FPS as usize;
pub const CONFIG_BYTES: usize = 8;
/// At most eight unacknowledged records on a conforming peer.
pub const MAX_IN_FLIGHT_RECORDS: usize = 8;
pub const REPEAT: [u8; 4] = u32::MAX.to_be_bytes();
/// The source did not observe a new native frame. Not a freshness or health claim.
pub const NO_UPDATE: [u8; 4] = (u32::MAX - 1).to_be_bytes();

/// Private fields ensure that every usable configuration has passed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoConfig {
    height: u16,
    fps: u8,
    quality: u8,
    seconds: u16,
    mode: CodecMode,
}

impl VideoConfig {
    pub fn new(height: u16, fps: u32, quality: u8, seconds: u64) -> Option<Self> {
        if !matches!(height, 540 | 720 | 1080)
            || !(1..=MAX_FPS).contains(&fps)
            || !(35..=90).contains(&quality)
            || !(1..=MAX_SECONDS).contains(&seconds)
        {
            return None;
        }
        Some(Self {
            height,
            fps: fps as u8,
            quality,
            seconds: seconds as u16,
            mode: CodecMode::Jpeg,
        })
    }

    pub const fn with_mode(mut self, mode: CodecMode) -> Self {
        self.mode = mode;
        self
    }

    pub const fn mode(self) -> CodecMode {
        self.mode
    }
    pub const fn width(self) -> u32 {
        self.height as u32 * 16 / 9
    }
    pub const fn height(self) -> u32 {
        self.height as u32
    }
    pub const fn fps(self) -> u32 {
        self.fps as u32
    }
    pub const fn quality(self) -> u8 {
        self.quality
    }
    pub const fn seconds(self) -> u64 {
        self.seconds as u64
    }
    pub const fn max_frames(self) -> usize {
        self.seconds as usize * self.fps as usize
    }

    pub fn encode(self) -> [u8; CONFIG_BYTES] {
        let [h0, h1] = self.height.to_be_bytes();
        let [s0, s1] = self.seconds.to_be_bytes();
        [
            h0,
            h1,
            self.fps,
            self.quality,
            s0,
            s1,
            MAX_IN_FLIGHT_RECORDS as u8,
            self.mode as u8,
        ]
    }

    pub fn decode(bytes: [u8; CONFIG_BYTES]) -> Option<Self> {
        if bytes[6] != MAX_IN_FLIGHT_RECORDS as u8 {
            return None;
        }
        let mode = CodecMode::decode(bytes[7])?;
        Self::new(
            u16::from_be_bytes([bytes[0], bytes[1]]),
            u32::from(bytes[2]),
            bytes[3],
            u64::from(u16::from_be_bytes([bytes[4], bytes[5]])),
        )
        .map(|config| config.with_mode(mode))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record {
    End,
    Repeat,
    NoUpdate,
    Frame(usize),
}

pub fn parse_length(header: [u8; 4]) -> Option<Record> {
    let count = u32::from_be_bytes(header);
    match count {
        0 => Some(Record::End),
        u32::MAX => Some(Record::Repeat),
        n if n == u32::MAX - 1 => Some(Record::NoUpdate),
        n if n as u64 <= MAX_FRAME_BYTES as u64 => Some(Record::Frame(n as usize)),
        _ => None,
    }
}

pub fn frame_header(len: usize) -> Option<[u8; 4]> {
    (1..=MAX_FRAME_BYTES)
        .contains(&len)
        .then(|| (len as u32).to_be_bytes())
}

/// One acknowledgment per data/repeat/no-update record, numbered from one.
/// This is receiver backpressure, not proof of rendering or remote authorization.
pub fn ack(sequence: usize) -> Option<[u8; 4]> {
    (1..=MAX_FRAMES)
        .contains(&sequence)
        .then(|| (sequence as u32).to_be_bytes())
}

/// Background priority reduces host sampling; it never changes authorization.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ViewPriority {
    #[default]
    Foreground,
    Background,
}

const BACKGROUND_BIT: u32 = 1 << 31;

pub fn ack_with_priority(sequence: usize, priority: ViewPriority) -> Option<[u8; 4]> {
    let raw = u32::from_be_bytes(ack(sequence)?);
    Some(
        (raw | if priority == ViewPriority::Background {
            BACKGROUND_BIT
        } else {
            0
        })
        .to_be_bytes(),
    )
}

/// Ordered receipt credit. This limits application backlog, not authorization.
/// The negotiated frame budget is separately enforced by the transport.
#[derive(Debug, Default)]
pub struct CreditWindow {
    sent: usize,
    received: usize,
    priority: ViewPriority,
}

impl CreditWindow {
    pub fn outstanding(&self) -> usize {
        self.sent - self.received
    }

    pub fn has_credit(&self) -> bool {
        self.outstanding() < MAX_IN_FLIGHT_RECORDS && self.sent < MAX_FRAMES
    }

    /// Called exactly once after a complete record is written.
    pub fn record_sent(&mut self) -> bool {
        if !self.has_credit() {
            return false;
        }
        self.sent += 1;
        true
    }

    pub fn priority(&self) -> ViewPriority {
        self.priority
    }

    /// Duplicate, skipped, unsolicited and out-of-range receipts are invalid.
    pub fn acknowledge(&mut self, bytes: [u8; 4]) -> bool {
        let raw = u32::from_be_bytes(bytes);
        if self.outstanding() == 0
            || ack(self.received + 1) != Some((raw & !BACKGROUND_BIT).to_be_bytes())
        {
            return false;
        }
        self.received += 1;
        self.priority = if raw & BACKGROUND_BIT != 0 {
            ViewPriority::Background
        } else {
            ViewPriority::Foreground
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_bounded_before_allocation() {
        assert_eq!(parse_length([0; 4]), Some(Record::End));
        assert_eq!(parse_length(REPEAT), Some(Record::Repeat));
        assert_eq!(parse_length(1u32.to_be_bytes()), Some(Record::Frame(1)));
        assert_eq!(
            parse_length((MAX_FRAME_BYTES as u32).to_be_bytes()),
            Some(Record::Frame(MAX_FRAME_BYTES))
        );
        assert_eq!(
            parse_length((MAX_FRAME_BYTES as u32 + 1).to_be_bytes()),
            None
        );
        assert_eq!(parse_length(NO_UPDATE), Some(Record::NoUpdate));
        assert_eq!(parse_length((u32::MAX - 2).to_be_bytes()), None);
        assert_eq!(frame_header(0), None);
        assert_eq!(frame_header(MAX_FRAME_BYTES + 1), None);
        assert_eq!(
            frame_header(42).and_then(parse_length),
            Some(Record::Frame(42))
        );
    }

    #[test]
    fn configuration_is_bounded_and_versioned() {
        for h in [540, 720, 1080] {
            let config = VideoConfig::new(h, MAX_FPS, 75, MAX_SECONDS).unwrap();
            assert_eq!(VideoConfig::decode(config.encode()), Some(config));
            assert_eq!(config.width() * 9, config.height() * 16);
            assert_eq!(config.max_frames(), MAX_FRAMES);
        }
        for config in [
            VideoConfig::new(2160, 30, 75, 60),
            VideoConfig::new(540, 61, 75, 60),
            VideoConfig::new(540, 0, 75, 60),
            VideoConfig::new(540, 10, 91, 60),
            VideoConfig::new(540, 10, 34, 60),
            VideoConfig::new(540, 10, 75, MAX_SECONDS + 1),
            VideoConfig::new(540, 10, 75, 0),
        ] {
            assert!(config.is_none());
        }
        let mut bytes = VideoConfig::new(540, 1, 35, 1).unwrap().encode();
        bytes[7] = 3;
        assert!(VideoConfig::decode(bytes).is_none());
        bytes[7] = 0;
        bytes[6] = 1;
        assert!(VideoConfig::decode(bytes).is_none());
    }

    #[test]
    fn requests_and_acknowledgments_are_exact() {
        for mode in [CodecMode::Jpeg, CodecMode::Tiles, CodecMode::Compact] {
            assert_eq!(parse_request(&request(mode)), Some(mode));
            let config = VideoConfig::new(540, 15, 75, 10).unwrap().with_mode(mode);
            assert_eq!(VideoConfig::decode(config.encode()), Some(config));
        }
        assert_eq!(parse_request(b"INTRALINK/3 VIEW"), None);
        assert_eq!(parse_request(b"INTRALINK/4 VIEW\x01"), None);
        assert_eq!(parse_request(b"INTRALINK/5 VIEW\x02"), None);
        assert_eq!(parse_request(b"INTRALINK/6 VIEW\x02"), None);
        assert_eq!(parse_request(b"INTRALINK/7 VIEW\x02"), None);
        assert_eq!(parse_request(b"INTRALINK/8 VIEW\xff"), None);
        assert_eq!(parse_request(b"INTRALINK/8 VIEW\0\0"), None);
        assert_eq!(ack(1), Some(1u32.to_be_bytes()));
        assert_eq!(ack(0), None);
        assert_eq!(ack(MAX_FRAMES + 1), None);
    }

    #[test]
    fn credit_is_bounded_and_receipts_are_ordered() {
        let mut credit = CreditWindow::default();
        assert!(!credit.acknowledge(1u32.to_be_bytes()));
        for _ in 0..MAX_IN_FLIGHT_RECORDS {
            assert!(credit.record_sent());
        }
        assert_eq!(credit.outstanding(), MAX_IN_FLIGHT_RECORDS);
        assert!(!credit.has_credit());
        assert!(!credit.record_sent());
        assert!(!credit.acknowledge(2u32.to_be_bytes()));
        assert_eq!(credit.outstanding(), MAX_IN_FLIGHT_RECORDS);
        assert!(credit.acknowledge(1u32.to_be_bytes()));
        assert!(!credit.acknowledge(1u32.to_be_bytes()));
        assert!(credit.record_sent());
        for sequence in 2..=MAX_IN_FLIGHT_RECORDS + 1 {
            assert!(credit.acknowledge(ack(sequence).unwrap()));
        }
        assert_eq!(credit.outstanding(), 0);
        assert!(!credit.acknowledge(ack(MAX_IN_FLIGHT_RECORDS + 2).unwrap()));
    }

    #[test]
    fn priority_feedback_is_ordered_and_cannot_grant_credit() {
        let mut credit = CreditWindow::default();
        assert!(credit.record_sent());
        assert!(!credit.acknowledge(ack_with_priority(2, ViewPriority::Background).unwrap()));
        assert_eq!(credit.priority(), ViewPriority::Foreground);
        assert!(credit.acknowledge(ack_with_priority(1, ViewPriority::Background).unwrap()));
        assert_eq!(credit.priority(), ViewPriority::Background);
        assert!(credit.record_sent());
        assert!(credit.acknowledge(ack(2).unwrap()));
        assert_eq!(credit.priority(), ViewPriority::Foreground);
    }

    #[test]
    fn credit_cannot_exceed_absolute_sequence_budget() {
        let mut credit = CreditWindow::default();
        for sequence in 1..=MAX_FRAMES {
            assert!(credit.record_sent());
            assert!(credit.acknowledge(ack(sequence).unwrap()));
        }
        assert_eq!(credit.outstanding(), 0);
        assert!(!credit.record_sent());
    }
}
