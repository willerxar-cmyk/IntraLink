//! Owner-operated DuckDNS updates. No account token is distributed to enrolled hosts.
use std::{
    io::{self, IsTerminal},
    net::{IpAddr, Ipv6Addr},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, ensure};
use intralink_core::duckdns_label;
use reqwest::{Client, redirect::Policy};
use zeroize::Zeroizing;

const DUCKDNS_UPDATE: &str = "https://www.duckdns.org/update";

#[derive(Clone, Copy)]
struct Addresses {
    ip: Option<IpAddr>,
    ipv6: Option<Ipv6Addr>,
}

impl Addresses {
    fn validate(self) -> Result<()> {
        ensure!(
            self.ipv6.is_none() || !self.ip.is_some_and(|addr| addr.is_ipv6()),
            "Use an IPv4 --ip with --ipv6"
        );
        ensure!(
            !self.ip.is_some_and(|addr| addr.is_unspecified()
                || addr.is_multicast()
                || addr.is_loopback()),
            "Invalid DDNS address"
        );
        ensure!(
            !self.ipv6.is_some_and(|addr| addr.is_unspecified()
                || addr.is_multicast()
                || addr.is_loopback()),
            "Invalid DDNS IPv6 address"
        );
        Ok(())
    }

    fn auto_v4(self) -> bool {
        self.ip.is_none() && self.ipv6.is_none()
    }
}

/// Provider-specific HTTP implementation; routing and vault code know nothing about it.
struct DuckDns {
    client: Client,
    label: String,
    token: Zeroizing<String>,
}

impl DuckDns {
    fn interactive(domain: &str) -> Result<Self> {
        let label = duckdns_label(domain).context("Invalid DuckDNS domain")?;
        ensure!(
            io::stdin().is_terminal(),
            "Interactive token entry is required"
        );
        println!("Updating the existing record {label}.duckdns.org");
        let token = Zeroizing::new(rpassword::prompt_password("DuckDNS account token: ")?);
        ensure!(
            !token.trim().is_empty() && token.len() <= 256,
            "Invalid DuckDNS token length"
        );
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| anyhow!("Could not initialize HTTPS client"))?;
        Ok(Self {
            client,
            label,
            token,
        })
    }

    async fn update(&self, addresses: Addresses) -> Result<()> {
        let mut request = self.client.get(DUCKDNS_UPDATE).query(&[
            ("domains", self.label.as_str()),
            ("token", self.token.trim()),
        ]);
        if let Some(value) = addresses.ip {
            request = request.query(&[("ip", value.to_string())]);
        }
        if let Some(value) = addresses.ipv6 {
            request = request.query(&[("ipv6", value.to_string())]);
        }
        // Never expose reqwest's token-bearing URLs or provider response text in errors.
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow!("DuckDNS HTTPS request failed"))?;
        ensure!(
            response.status().is_success(),
            "DuckDNS returned an HTTP error"
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("DuckDNS response failed"))?
        {
            ensure!(
                body.len().saturating_add(chunk.len()) <= 128,
                "Unexpected DuckDNS response length"
            );
            body.extend_from_slice(&chunk);
        }
        ensure!(
            std::str::from_utf8(&body).is_ok_and(|text| text.trim() == "OK"),
            "DuckDNS did not accept the update"
        );
        println!("DuckDNS accepted the update. DNS caching may delay visibility.");
        Ok(())
    }
}

pub async fn update(
    domain: &str,
    ip: Option<IpAddr>,
    ipv6: Option<Ipv6Addr>,
    interval: Option<u64>,
) -> Result<()> {
    let addresses = Addresses { ip, ipv6 };
    addresses.validate()?;
    if let Some(seconds) = interval {
        ensure!(
            (60..=3600).contains(&seconds),
            "Refresh interval must be 60-3600 seconds"
        );
        ensure!(
            addresses.auto_v4(),
            "Periodic refresh supports provider-detected IPv4 only; omit --ip and --ipv6"
        );
    }
    let provider = DuckDns::interactive(domain)?;
    provider.update(addresses).await?;
    if let Some(seconds) = interval {
        println!(
            "Foreground refresh every {seconds} seconds. Ctrl+C stops updates; the token is not saved."
        );
        loop {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            if let Err(error) = provider.update(addresses).await {
                eprintln!("Refresh failed: {error}. Will retry at the next configured interval.");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_address_modes() -> Result<()> {
        assert!(
            Addresses {
                ip: None,
                ipv6: None
            }
            .auto_v4()
        );
        let dual = Addresses {
            ip: Some("192.0.2.1".parse()?),
            ipv6: Some("2001:db8::1".parse()?),
        };
        dual.validate()?;
        assert!(!dual.auto_v4());
        assert!(
            Addresses {
                ip: Some("2001:db8::2".parse()?),
                ipv6: dual.ipv6
            }
            .validate()
            .is_err()
        );
        assert!(
            Addresses {
                ip: Some("127.0.0.1".parse()?),
                ipv6: None
            }
            .validate()
            .is_err()
        );
        Ok(())
    }
}
