//! Developer-only export of the exact generated fixture after queue recovery.
//! No input file, network, real screen or arbitrary decoder input.
use intralink_core::h264::{
    Framing, Profile, StreamState,
    producer::{Poll, Producer, Push},
};
use std::{
    io::{self, Write},
    time::Instant,
};

fn main() -> io::Result<()> {
    let fixture =
        include_bytes!("../../intralink-d3d11/examples/fixtures/headless-bands-1080p.h264");
    let sizes = [895, 2642, 2563, 866, 2581, 2579];
    let offsets = [0, 895, 3537, 6100, 6966, 9547];
    assert_eq!(fixture.len(), sizes.iter().sum());
    let profile = Profile::new(1080, 60, 3, 8_000_000).unwrap();
    let start = Instant::now();
    let mut source = Producer::new(profile, start);
    let mut receiver = StreamState::new(profile);
    let mut output = Vec::new();
    let mut stdout = io::stdout().lock();

    for i in 0..sizes.len() {
        let data = &fixture[offsets[i]..offsets[i] + sizes[i]];
        assert_eq!(
            source.push(
                Framing::AnnexB,
                matches!(i, 0 | 3),
                i as i64 * 16_667,
                data.into(),
                start
            ),
            Ok(Push::Queued)
        );
        // Deliberately withhold demand for the two P pictures from the first
        // GOP. Arrival of the second IDR saturates and replaces that queue.
        if matches!(i, 1 | 2) {
            continue;
        }
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        let au = receiver.accept(&output).expect("Receiver validation");
        stdout.write_all(au.annex_b)?;
    }
    assert_eq!(source.pending_count(), 0);
    stdout.flush()
}
