//! Versioned, bounded software-KVM input records. No screen or OS APIs.
use std::fmt;

pub const ALPN: &[u8] = b"intralink/input/1";
pub const HELLO: &[u8; 8] = b"ILKVM1\0\0";
pub const READY: &[u8; 8] = b"ILKVMOK1";
/// Explicit geometry-aware capability; never silently downgrade to input/1.
pub const GEOMETRY_ALPN: &[u8] = b"intralink/input/2";
pub const GEOMETRY_HELLO: &[u8; 8] = b"ILKVM2\0\0";
pub const GEOMETRY_READY: &[u8; 8] = b"ILKVMOK2";
pub const RECORD_BYTES: usize = 12;
pub const MAX_SECONDS: u16 = 3600;
pub const MAX_SLOTS: usize = 8;
pub const MAX_EVENTS_PER_SECOND: u32 = 512;
pub const MAX_EVENTS_PER_SESSION: u32 = MAX_SECONDS as u32 * MAX_EVENTS_PER_SECOND;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidRecord,
    Sequence,
    Inactive,
    AlreadyActive,
    DuplicateTransition,
    Ended,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Input protocol: {self:?}")
    }
}
impl std::error::Error for Error {}

/// USB HID keyboard-page usages; only the explicitly supported physical
/// keys are accepted. No text, executable, arbitrary virtual-key or command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key(u8);

impl Key {
    pub const fn new(usage: u8) -> Option<Self> {
        match usage {
            4..=49 | 51..=69 | 71 | 73..=99 | 224..=231 => Some(Self(usage)),
            _ => None,
        }
    }

    pub const fn usage(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Button {
    Left = 1,
    Right = 2,
    Middle = 3,
}
impl Button {
    fn parse(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Left),
            2 => Some(Self::Right),
            3 => Some(Self::Middle),
            _ => None,
        }
    }
    const fn index(self) -> usize {
        self as usize - 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Activate,
    Deactivate,
    Heartbeat,
    End,
    /// Normalized absolute position on the selected primary display.
    Move {
        x: u16,
        y: u16,
    },
    Key {
        key: Key,
        down: bool,
    },
    Button {
        button: Button,
        down: bool,
    },
    Wheel {
        horizontal: bool,
        steps: i16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub sequence: u32,
    pub event: Event,
}

impl Record {
    pub fn encode(self) -> Result<[u8; RECORD_BYTES], Error> {
        if self.sequence == 0 || self.sequence > MAX_EVENTS_PER_SESSION {
            return Err(Error::Sequence);
        }
        let (tag, flag, a, b) = match self.event {
            Event::Activate => (1, 0, 0, 0),
            Event::Deactivate => (2, 0, 0, 0),
            Event::Heartbeat => (3, 0, 0, 0),
            Event::End => (4, 0, 0, 0),
            Event::Move { x, y } => (5, 0, x, y),
            Event::Key { key, down } => (6, u8::from(down), u16::from(key.usage()), 0),
            Event::Button { button, down } => (7, u8::from(down), button as u16, 0),
            Event::Wheel { horizontal, steps } if steps != 0 && (-8..=8).contains(&steps) => {
                (8, u8::from(horizontal), steps as u16, 0)
            }
            Event::Wheel { .. } => return Err(Error::InvalidRecord),
        };
        let mut bytes = [0; RECORD_BYTES];
        bytes[..4].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[4] = tag;
        bytes[5] = flag;
        bytes[6..8].copy_from_slice(&a.to_be_bytes());
        bytes[8..10].copy_from_slice(&b.to_be_bytes());
        Ok(bytes)
    }

    pub fn decode(bytes: [u8; RECORD_BYTES]) -> Result<Self, Error> {
        let sequence = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| Error::InvalidRecord)?);
        let flag = bytes[5];
        let a = u16::from_be_bytes([bytes[6], bytes[7]]);
        let b = u16::from_be_bytes([bytes[8], bytes[9]]);
        let event = match bytes[4] {
            1 => Event::Activate,
            2 => Event::Deactivate,
            3 => Event::Heartbeat,
            4 => Event::End,
            5 => Event::Move { x: a, y: b },
            6 => Event::Key {
                key: Key::new(u8::try_from(a).map_err(|_| Error::InvalidRecord)?)
                    .ok_or(Error::InvalidRecord)?,
                down: flag == 1,
            },
            7 => Event::Button {
                button: Button::parse(a).ok_or(Error::InvalidRecord)?,
                down: flag == 1,
            },
            8 => Event::Wheel {
                horizontal: flag == 1,
                steps: a as i16,
            },
            _ => return Err(Error::InvalidRecord),
        };
        let record = Self { sequence, event };
        // Re-encoding rejects every unused field, noncanonical flag and
        // out-of-range value, including reserved bytes and trailing payload.
        if record.encode()? != bytes {
            return Err(Error::InvalidRecord);
        }
        Ok(record)
    }
}

/// Tracks the exact ordered input state. A native sink must first perform
/// a validated event successfully, then commit it. On failure release keys.
#[derive(Clone)]
pub struct State {
    active: bool,
    ended: bool,
    keys: [bool; 256],
    buttons: [bool; 3],
    sequence: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            active: false,
            ended: false,
            keys: [false; 256],
            buttons: [false; 3],
            sequence: 0,
        }
    }
}

impl State {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn next_record(&self, event: Event) -> Result<Record, Error> {
        let record = Record {
            sequence: self.sequence.checked_add(1).ok_or(Error::Sequence)?,
            event,
        };
        self.validate(record)?;
        Ok(record)
    }

    pub fn validate(&self, record: Record) -> Result<(), Error> {
        record.encode()?;
        if self.ended {
            return Err(Error::Ended);
        }
        if self.sequence.checked_add(1) != Some(record.sequence) {
            return Err(Error::Sequence);
        }
        match record.event {
            Event::Activate if self.active => Err(Error::AlreadyActive),
            Event::Deactivate if !self.active => Err(Error::Inactive),
            Event::Activate | Event::Deactivate | Event::Heartbeat | Event::End => Ok(()),
            _ if !self.active => Err(Error::Inactive),
            Event::Key { key, down } if self.keys[key.usage() as usize] == down => {
                Err(Error::DuplicateTransition)
            }
            Event::Button { button, down } if self.buttons[button.index()] == down => {
                Err(Error::DuplicateTransition)
            }
            _ => Ok(()),
        }
    }

    pub fn commit(&mut self, record: Record) -> Result<(), Error> {
        self.validate(record)?;
        match record.event {
            Event::Activate => self.active = true,
            Event::Deactivate | Event::End => {
                self.active = false;
                self.keys.fill(false);
                self.buttons.fill(false);
                self.ended = record.event == Event::End;
            }
            Event::Key { key, down } => self.keys[key.usage() as usize] = down,
            Event::Button { button, down } => self.buttons[button.index()] = down,
            Event::Move { .. } | Event::Wheel { .. } | Event::Heartbeat => {}
        }
        self.sequence = record.sequence;
        Ok(())
    }

    /// Releases only keys/buttons actually owned by this session, not
    /// arbitrary keys pressed locally. Nonmodifiers precede modifiers.
    pub fn releases(&self) -> Vec<Event> {
        let mut events = Vec::new();
        for (usage, held) in self.keys.iter().enumerate() {
            if *held && let Some(key) = Key::new(usage as u8) {
                events.push(Event::Key { key, down: false });
            }
        }
        for button in [Button::Left, Button::Right, Button::Middle] {
            if self.buttons[button.index()] {
                events.push(Event::Button {
                    button,
                    down: false,
                });
            }
        }
        events
    }
}

/// One central focus: switching is two-phase so no second device gets
/// input until the preceding session acknowledges release/deactivation.
#[derive(Debug, Default)]
pub struct Focus {
    active: Option<usize>,
    pending: Option<usize>,
    switching: bool,
    release_confirmed: bool,
}
impl Focus {
    pub fn active(&self) -> Option<usize> {
        self.active
    }
    pub fn begin(&mut self, target: Option<usize>) -> Result<Option<usize>, Error> {
        if self.switching || target.is_some_and(|slot| slot >= MAX_SLOTS) {
            return Err(Error::InvalidRecord);
        }
        if self.active == target {
            return Err(Error::AlreadyActive);
        }
        self.pending = target;
        self.switching = true;
        self.release_confirmed = false;
        Ok(self.active)
    }
    pub fn released(&mut self) -> Result<Option<usize>, Error> {
        if !self.switching || self.release_confirmed {
            return Err(Error::InvalidRecord);
        }
        self.active = None;
        self.release_confirmed = true;
        Ok(self.pending)
    }
    pub fn activated(&mut self, target: Option<usize>) -> Result<(), Error> {
        if !self.switching
            || !self.release_confirmed
            || self.active.is_some()
            || self.pending != target
        {
            return Err(Error::InvalidRecord);
        }
        self.active = target;
        self.pending = None;
        self.switching = false;
        self.release_confirmed = false;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.active = None;
        self.pending = None;
        self.switching = false;
        self.release_confirmed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> Key {
        Key::new(4).unwrap()
    }
    fn apply(s: &mut State, event: Event) {
        let record = s.next_record(event).unwrap();
        assert_eq!(Record::decode(record.encode().unwrap()), Ok(record));
        s.commit(record).unwrap();
    }

    #[test]
    fn canonical_records_reject_reserved_fields_and_hostile_values() {
        for event in [
            Event::Activate,
            Event::Deactivate,
            Event::Heartbeat,
            Event::End,
            Event::Move { x: 65535, y: 0 },
            Event::Key {
                key: key(),
                down: true,
            },
            Event::Button {
                button: Button::Middle,
                down: false,
            },
            Event::Wheel {
                horizontal: true,
                steps: -8,
            },
        ] {
            let record = Record { sequence: 1, event };
            let mut bytes = record.encode().unwrap();
            assert_eq!(Record::decode(bytes), Ok(record));
            bytes[11] = 1;
            assert!(Record::decode(bytes).is_err());
        }
        assert!(
            Record {
                sequence: 0,
                event: Event::Activate
            }
            .encode()
            .is_err()
        );
        assert!(
            Record {
                sequence: 1,
                event: Event::Wheel {
                    horizontal: false,
                    steps: 9
                }
            }
            .encode()
            .is_err()
        );
        let mut bytes = Record {
            sequence: 1,
            event: Event::Key {
                key: key(),
                down: true,
            },
        }
        .encode()
        .unwrap();
        bytes[5] = 2;
        assert!(Record::decode(bytes).is_err());
        bytes[5] = 1;
        bytes[7] = 0;
        assert!(Record::decode(bytes).is_err());
    }

    #[test]
    fn ordered_state_requires_activation_and_releases_held_input() {
        let mut s = State::default();
        assert_eq!(
            s.next_record(Event::Key {
                key: key(),
                down: true
            }),
            Err(Error::Inactive)
        );
        apply(&mut s, Event::Heartbeat);
        apply(&mut s, Event::Activate);
        apply(
            &mut s,
            Event::Key {
                key: key(),
                down: true,
            },
        );
        apply(
            &mut s,
            Event::Button {
                button: Button::Left,
                down: true,
            },
        );
        assert_eq!(
            s.next_record(Event::Key {
                key: key(),
                down: true
            }),
            Err(Error::DuplicateTransition)
        );
        assert_eq!(
            s.releases(),
            vec![
                Event::Key {
                    key: key(),
                    down: false
                },
                Event::Button {
                    button: Button::Left,
                    down: false
                }
            ]
        );
        assert_eq!(
            s.validate(Record {
                sequence: 2,
                event: Event::Heartbeat
            }),
            Err(Error::Sequence)
        );
        apply(&mut s, Event::Deactivate);
        assert!(s.releases().is_empty() && !s.active());
        apply(&mut s, Event::End);
        assert_eq!(s.next_record(Event::Activate), Err(Error::Ended));
    }

    #[test]
    fn focus_never_activates_second_slot_before_release() {
        let mut focus = Focus::default();
        assert_eq!(focus.begin(Some(0)), Ok(None));
        assert!(focus.activated(Some(0)).is_err());
        assert_eq!(focus.released(), Ok(Some(0)));
        assert!(focus.released().is_err());
        focus.activated(Some(0)).unwrap();
        assert_eq!(focus.begin(Some(2)), Ok(Some(0)));
        assert!(focus.activated(Some(2)).is_err());
        assert!(focus.begin(Some(1)).is_err());
        assert_eq!(focus.released(), Ok(Some(2)));
        assert_eq!(focus.active(), None);
        focus.activated(Some(2)).unwrap();
        assert_eq!(focus.active(), Some(2));
        assert!(focus.begin(Some(8)).is_err());
        focus.cancel();
        assert_eq!(focus.active(), None);
    }
}
