//! Canonical bounded operating-point offer for an isolated negotiation probe.
//! An agreed Mode is not authorization to capture, transmit or decode media.
use super::{
    SpsInfo,
    modes::{Color, Limits, MAX_MODES, Mode, select_exact},
};

pub const ALPN: &[u8] = b"intralink/avc-negotiation/1";
const MAGIC: [u8; 4] = *b"ILA\x01";
const HEADER: usize = 16;
pub const MODE_BYTES: usize = 16;
pub const OFFER_BYTES: usize = HEADER + MAX_MODES * MODE_BYTES;
pub const RESPONSE_BYTES: usize = OFFER_BYTES + MODE_BYTES;
pub const COMPLETE: &[u8; 4] = b"OKA\x01";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Producer,
    Receiver,
}

impl Role {
    fn encode(self) -> u8 {
        match self {
            Self::Producer => 1,
            Self::Receiver => 2,
        }
    }
    fn decode(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Producer),
            2 => Some(Self::Receiver),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    role: Role,
    limits: Limits,
    modes: Vec<Mode>,
}

impl Offer {
    pub fn new(role: Role, limits: Limits, modes: &[Mode]) -> Option<Self> {
        if modes.is_empty()
            || modes.len() > MAX_MODES
            || modes
                .iter()
                .enumerate()
                .any(|(i, mode)| !limits.permits(*mode) || modes[..i].contains(mode))
        {
            return None;
        }
        Some(Self {
            role,
            limits,
            modes: modes.to_vec(),
        })
    }

    pub const fn role(&self) -> Role {
        self.role
    }

    pub fn encode(&self) -> [u8; OFFER_BYTES] {
        let mut bytes = [0; OFFER_BYTES];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4] = self.role.encode();
        bytes[5] = self.modes.len() as u8;
        let (width, height) = self.limits.dimensions();
        bytes[6..8].copy_from_slice(&width.to_be_bytes());
        bytes[8..10].copy_from_slice(&height.to_be_bytes());
        bytes[10] = self.limits.fps();
        bytes[12..16].copy_from_slice(&self.limits.bitrate().to_be_bytes());
        for (chunk, mode) in bytes[HEADER..]
            .chunks_exact_mut(MODE_BYTES)
            .zip(&self.modes)
        {
            chunk.copy_from_slice(&encode_mode(*mode));
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != OFFER_BYTES || bytes[..4] != MAGIC || bytes[11] != 0 {
            return None;
        }
        let role = Role::decode(bytes[4])?;
        let count = usize::from(bytes[5]);
        if !(1..=MAX_MODES).contains(&count) {
            return None;
        }
        let limits = Limits::new(
            u16::from_be_bytes(bytes[6..8].try_into().ok()?),
            u16::from_be_bytes(bytes[8..10].try_into().ok()?),
            bytes[10],
            u32::from_be_bytes(bytes[12..16].try_into().ok()?),
        )?;
        let mut modes = Vec::with_capacity(count);
        for chunk in bytes[HEADER..HEADER + count * MODE_BYTES].chunks_exact(MODE_BYTES) {
            modes.push(decode_mode(chunk)?);
        }
        if bytes[HEADER + count * MODE_BYTES..].iter().any(|b| *b != 0) {
            return None;
        }
        Self::new(role, limits, &modes)
    }
}

pub fn encode_mode(mode: Mode) -> [u8; MODE_BYTES] {
    let sps = mode.sps();
    let mut bytes = [0; MODE_BYTES];
    bytes[..2].copy_from_slice(&sps.width.to_be_bytes());
    bytes[2..4].copy_from_slice(&sps.height.to_be_bytes());
    bytes[4] = sps.profile_idc;
    bytes[5] = sps.level_idc;
    bytes[6] = sps.constraints;
    bytes[7] = sps.references;
    bytes[8] = sps.poc_type;
    bytes[9] = match mode.color() {
        Color::Bt709Limited => 1,
        Color::Bt601Limited => 2,
        Color::Bt709Full => 3,
    };
    bytes[10] = mode.fps();
    bytes[12..].copy_from_slice(&mode.bitrate().to_be_bytes());
    bytes
}

pub fn decode_mode(bytes: &[u8]) -> Option<Mode> {
    if bytes.len() != MODE_BYTES || bytes[11] != 0 {
        return None;
    }
    let color = match bytes[9] {
        1 => Color::Bt709Limited,
        2 => Color::Bt601Limited,
        3 => Color::Bt709Full,
        _ => return None,
    };
    Mode::new(
        SpsInfo {
            width: u16::from_be_bytes(bytes[..2].try_into().ok()?),
            height: u16::from_be_bytes(bytes[2..4].try_into().ok()?),
            profile_idc: bytes[4],
            level_idc: bytes[5],
            constraints: bytes[6],
            references: bytes[7],
            poc_type: bytes[8],
        },
        color,
        bytes[10],
        u32::from_be_bytes(bytes[12..].try_into().ok()?),
    )
}

pub fn choose(producer: &Offer, receiver: &Offer) -> Option<Mode> {
    if producer.role != Role::Producer || receiver.role != Role::Receiver {
        return None;
    }
    select_exact(
        &producer.modes,
        producer.limits,
        &receiver.modes,
        receiver.limits,
    )
    .ok()
    .flatten()
}

pub fn response(producer: &Offer, receiver: &Offer) -> Option<[u8; RESPONSE_BYTES]> {
    let mut bytes = [0; RESPONSE_BYTES];
    bytes[..OFFER_BYTES].copy_from_slice(&producer.encode());
    bytes[OFFER_BYTES..].copy_from_slice(&encode_mode(choose(producer, receiver)?));
    Some(bytes)
}

/// Independently recompute the intersection from the receiver's original
/// local offer. Neither a different mode nor an extra byte is accepted.
pub fn confirm(receiver: &Offer, bytes: &[u8]) -> Option<Mode> {
    if bytes.len() != RESPONSE_BYTES {
        return None;
    }
    let producer = Offer::decode(&bytes[..OFFER_BYTES])?;
    let selected = decode_mode(&bytes[OFFER_BYTES..])?;
    (choose(&producer, receiver)? == selected).then_some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(w: u16, h: u16, fps: u8) -> Mode {
        Mode::new(
            SpsInfo {
                width: w,
                height: h,
                profile_idc: 100,
                level_idc: 42,
                constraints: 0,
                references: 3,
                poc_type: 2,
            },
            Color::Bt709Limited,
            fps,
            8_000_000,
        )
        .unwrap()
    }
    fn limits() -> Limits {
        Limits::new(1920, 1920, 60, 12_000_000).unwrap()
    }

    #[test]
    fn canonical_roundtrip_and_exact_choice() {
        let landscape = mode(1920, 1080, 60);
        let portrait = mode(1080, 1920, 30);
        let p = Offer::new(Role::Producer, limits(), &[landscape, portrait]).unwrap();
        let r = Offer::new(Role::Receiver, limits(), &[portrait, landscape]).unwrap();
        assert_eq!(Offer::decode(&p.encode()), Some(p.clone()));
        assert_eq!(Offer::decode(&r.encode()), Some(r.clone()));
        assert_eq!(confirm(&r, &response(&p, &r).unwrap()), Some(landscape));
        assert!(response(&r, &p).is_none());
        let mut altered = response(&p, &r).unwrap();
        altered[OFFER_BYTES..].copy_from_slice(&encode_mode(portrait));
        assert!(confirm(&r, &altered).is_none());
        let only_portrait = Offer::new(Role::Receiver, limits(), &[portrait]).unwrap();
        assert_eq!(
            confirm(&only_portrait, &response(&p, &only_portrait).unwrap()),
            Some(portrait)
        );
    }

    #[test]
    fn reject_noncanonical_and_outside_limits() {
        let m = mode(1920, 1080, 60);
        let p = Offer::new(Role::Producer, limits(), &[m]).unwrap();
        let bytes = p.encode();
        for end in 0..bytes.len() {
            assert!(Offer::decode(&bytes[..end]).is_none());
        }
        for (index, value) in [
            (0, 0),
            (3, 2),
            (4, 3),
            (5, 0),
            (5, 17),
            (11, 1),
            (16 + 9, 4),
            (16 + 11, 1),
            (OFFER_BYTES - 1, 1),
        ] {
            let mut changed = bytes;
            changed[index] = value;
            assert!(Offer::decode(&changed).is_none());
        }
        let mut duplicated = bytes;
        duplicated[5] = 2;
        duplicated[HEADER + MODE_BYTES..HEADER + 2 * MODE_BYTES].copy_from_slice(&encode_mode(m));
        assert!(Offer::decode(&duplicated).is_none());
        assert!(Offer::new(Role::Receiver, limits(), &[m, m]).is_none());
        assert!(
            Offer::new(
                Role::Receiver,
                Limits::new(1920, 1080, 30, 12_000_000).unwrap(),
                &[m]
            )
            .is_none()
        );
        let mut extended = bytes.to_vec();
        extended.push(0);
        assert!(Offer::decode(&extended).is_none());
    }
}
