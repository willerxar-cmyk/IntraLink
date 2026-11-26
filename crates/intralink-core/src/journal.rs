//! Bounded, local-only metadata for foreground session attempts.
//! No network payload, peer identity, address or credentials are representable.
use std::fmt;

pub const MAX_HISTORY_BYTES: usize = 256 * 1024;
pub const MAX_LINE_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    ProbeHost,
    ProbeController,
    ViewHost,
    ViewController,
}

impl Operation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProbeHost => "probe-host",
            Self::ProbeController => "probe-controller",
            Self::ViewHost => "view-host",
            Self::ViewController => "view-controller",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "probe-host" => Some(Self::ProbeHost),
            "probe-controller" => Some(Self::ProbeController),
            "view-host" => Some(Self::ViewHost),
            "view-controller" => Some(Self::ViewController),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Cancelled,
    Failed,
}

impl Outcome {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Completed => "protocol-completed",
            Self::Cancelled => "local-cancel",
            Self::Failed => "command-error",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "protocol-completed" => Some(Self::Completed),
            "local-cancel" => Some(Self::Cancelled),
            "command-error" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    pub id: u64,
    pub started_unix: u64,
    pub operation: Operation,
    pub end: Option<(Outcome, u64)>,
}

impl Attempt {
    pub fn start_line(self) -> String {
        format!(
            "IL1\t{}\t{}\t{}\tbegin\n",
            self.id,
            self.started_unix,
            self.operation.label()
        )
    }

    pub fn end_line(self, outcome: Outcome, elapsed_ms: u64) -> String {
        format!(
            "IL1\t{}\t{}\t{}\t{}\t{}\n",
            self.id,
            self.started_unix,
            self.operation.label(),
            outcome.label(),
            elapsed_ms
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    TooLarge,
    Malformed,
    Sequence,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "history exceeds its size limit",
            Self::Malformed => "malformed or incomplete history record",
            Self::Sequence => "invalid history record sequence",
        })
    }
}

impl std::error::Error for HistoryError {}

fn number(s: Option<&str>) -> Result<u64, HistoryError> {
    let s = s.ok_or(HistoryError::Malformed)?;
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(HistoryError::Malformed);
    }
    s.parse().map_err(|_| HistoryError::Malformed)
}

/// Parse the complete bounded journal. A begin without a later end means
/// unknown outcome (e.g. interrupted process); it is not evidence of access.
/// A corrupt or partial record is never silently removed.
pub fn parse(bytes: &[u8]) -> Result<Vec<Attempt>, HistoryError> {
    if bytes.len() > MAX_HISTORY_BYTES {
        return Err(HistoryError::TooLarge);
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(HistoryError::Malformed);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| HistoryError::Malformed)?;
    let mut attempts: Vec<Attempt> = Vec::new();
    for line in text.split_terminator('\n') {
        if line.len() >= MAX_LINE_BYTES {
            return Err(HistoryError::Malformed);
        }
        let mut fields = line.split('\t');
        if fields.next() != Some("IL1") {
            return Err(HistoryError::Malformed);
        }
        let id = number(fields.next())?;
        let started_unix = number(fields.next())?;
        let operation = fields
            .next()
            .and_then(Operation::parse)
            .ok_or(HistoryError::Malformed)?;
        let state = fields.next().ok_or(HistoryError::Malformed)?;
        if state == "begin" {
            if fields.next().is_some() {
                return Err(HistoryError::Malformed);
            }
            let next = attempts.last().map_or(Some(1), |a| a.id.checked_add(1));
            if Some(id) != next {
                return Err(HistoryError::Sequence);
            }
            attempts.push(Attempt {
                id,
                started_unix,
                operation,
                end: None,
            });
        } else {
            let outcome = Outcome::parse(state).ok_or(HistoryError::Malformed)?;
            let elapsed = number(fields.next())?;
            if fields.next().is_some() {
                return Err(HistoryError::Malformed);
            }
            let last = attempts.last_mut().ok_or(HistoryError::Sequence)?;
            if last.id != id
                || last.started_unix != started_unix
                || last.operation != operation
                || last.end.is_some()
            {
                return Err(HistoryError::Sequence);
            }
            last.end = Some((outcome, elapsed));
        }
    }
    Ok(attempts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(id: u64) -> Attempt {
        Attempt {
            id,
            started_unix: 1700000000,
            operation: Operation::ViewHost,
            end: None,
        }
    }

    #[test]
    fn roundtrip_preserves_unknown_cancelled_and_completed_outcomes() {
        let a = attempt(1);
        let b = attempt(2);
        let c = attempt(3);
        let input = a.start_line()
            + &b.start_line()
            + &b.end_line(Outcome::Cancelled, 0)
            + &c.start_line()
            + &c.end_line(Outcome::Completed, 250);
        let records = parse(input.as_bytes()).unwrap();
        assert_eq!(records[0], a);
        assert_eq!(records[1].end, Some((Outcome::Cancelled, 0)));
        assert_eq!(records[2].end, Some((Outcome::Completed, 250)));
        assert!(parse(b"").unwrap().is_empty());
    }

    #[test]
    fn rejects_duplicate_wrong_and_out_of_order_end_records() {
        let a = attempt(1);
        let b = attempt(2);
        let end = a.end_line(Outcome::Failed, 9);
        assert_eq!(parse(end.as_bytes()), Err(HistoryError::Sequence));
        let duplicate = a.start_line() + &end + &end;
        assert_eq!(parse(duplicate.as_bytes()), Err(HistoryError::Sequence));
        let late = a.start_line() + &b.start_line() + &end;
        assert_eq!(parse(late.as_bytes()), Err(HistoryError::Sequence));
        let wrong = a.start_line()
            + &Attempt {
                operation: Operation::ProbeHost,
                ..a
            }
            .end_line(Outcome::Failed, 9);
        assert_eq!(parse(wrong.as_bytes()), Err(HistoryError::Sequence));
        assert_eq!(
            parse(b.start_line().as_bytes()),
            Err(HistoryError::Sequence)
        );
    }

    #[test]
    fn rejects_truncation_control_fields_and_unbounded_input() {
        for data in [
            b"IL1\t1\t1\tview-host\tbegin".as_slice(),
            b"\n",
            b"IL1\t01\t1\tview-host\tbegin\n",
            b"IL1\t1\t1\tview-host\tbegin\r\n",
            b"IL1\t1\t1\tsecret\tbegin\n",
            b"IL1\t1\t1\tview-host\tbegin\textra\n",
            b"\xff\n",
        ] {
            assert_eq!(parse(data), Err(HistoryError::Malformed));
        }
        assert_eq!(
            parse(&vec![0; MAX_HISTORY_BYTES + 1]),
            Err(HistoryError::TooLarge)
        );
    }

    #[test]
    fn maximal_numeric_values_fit_reserved_line_budget() {
        let a = Attempt {
            id: u64::MAX,
            started_unix: u64::MAX,
            operation: Operation::ViewController,
            end: None,
        };
        assert!(a.start_line().len() <= MAX_LINE_BYTES);
        for outcome in [Outcome::Completed, Outcome::Cancelled, Outcome::Failed] {
            assert!(a.end_line(outcome, u64::MAX).len() <= MAX_LINE_BYTES);
        }
    }
}
