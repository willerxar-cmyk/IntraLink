//! Owner-selected generated app canvas acceptance; no general input capture.
use anyhow::{Context, Result, ensure};
use intralink_core::input::{Button, Event, Key};
use intralink_transport::{bind_controller, input::InputConnection};
use iroh::{EndpointId, SecretKey};
use std::{
    io::{self, Write},
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::time::timeout;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 1,
        "Supply the explicit private app test endpoint"
    );
    let addr: SocketAddr = args[0].parse()?;
    ensure!(
        matches!(addr,SocketAddr::V4(a) if a.ip().is_private() && a.port()>=1024),
        "Private test endpoint required"
    );
    let endpoint = bind_controller(SecretKey::generate(), &[addr]).await?;
    println!("INPUT_WINDOWS_ID {}", endpoint.id());
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let expected: EndpointId = line.trim().parse()?;
    ensure!(
        expected.to_string() == line.trim() && expected != endpoint.id(),
        "Invalid peer identity"
    );
    let result = timeout(Duration::from_secs(15), async {
        let mut conn = InputConnection::connect(&endpoint, expected, &[addr]).await?;
        let mut records = 0u32;
        let mut max_us = 0u128;
        for event in [
            Event::Activate,
            Event::Move { x: 32768, y: 32768 },
            Event::Key {
                key: Key::new(225).unwrap(),
                down: true,
            },
            Event::Key {
                key: Key::new(4).unwrap(),
                down: true,
            },
            Event::Key {
                key: Key::new(4).unwrap(),
                down: false,
            },
            Event::Key {
                key: Key::new(225).unwrap(),
                down: false,
            },
            Event::Key {
                key: Key::new(5).unwrap(),
                down: true,
            },
            Event::Key {
                key: Key::new(5).unwrap(),
                down: false,
            },
            Event::Button {
                button: Button::Left,
                down: true,
            },
            Event::Move { x: 48000, y: 41000 },
            Event::Button {
                button: Button::Left,
                down: false,
            },
            Event::Wheel {
                horizontal: false,
                steps: 3,
            },
            Event::Deactivate,
            Event::Activate,
            Event::Key {
                key: Key::new(225).unwrap(),
                down: true,
            },
            Event::End,
        ] {
            let began = Instant::now();
            conn.send_with_budget(event, Duration::from_millis(500))
                .await?;
            max_us = max_us.max(began.elapsed().as_micros());
            records += 1;
            if records == 2 {
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        }
        println!("INPUT_WINDOWS_PASSED records={records} max_ack_us={max_us}");
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("Input fixture deadline");
    endpoint.close().await;
    result?
}
