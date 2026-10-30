//! The HTTP client for DropLaunch's downloads (SteamCMD, Carbon/Oxide, updates, item pictures, the
//! branch list). On Windows HTTPS goes through Windows' own TLS (SChannel, with the certificates
//! Windows trusts) instead of a TLS library and certificate list built into the exe.

use std::time::Duration;

/// An agent with these timeouts and DropLaunch's name as the user agent.
pub fn agent(connect: Duration, read: Duration) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout_read(read)
        .user_agent(concat!("DropLaunch/", env!("CARGO_PKG_VERSION")));
    #[cfg(windows)]
    let builder = match tls() {
        Some(tls) => builder.tls_connector(tls),
        None => builder,
    };
    builder.build()
}

/// Made once; None only if Windows can't give one (HTTPS then says so).
#[cfg(windows)]
fn tls() -> Option<std::sync::Arc<native_tls::TlsConnector>> {
    use std::sync::{Arc, OnceLock};
    static TLS: OnceLock<Option<Arc<native_tls::TlsConnector>>> = OnceLock::new();
    TLS.get_or_init(|| native_tls::TlsConnector::new().ok().map(Arc::new)).clone()
}

#[cfg(test)]
mod tests {
    /// Over the internet: cargo test https_works -- --ignored
    #[test]
    #[ignore]
    fn https_works() {
        let agent = super::agent(std::time::Duration::from_secs(10), std::time::Duration::from_secs(20));
        // GitHub (updates, Carbon, Oxide; its downloads redirect to another host), Valve's CDN
        // (SteamCMD) and the branch list's mirror.
        for url in [
            "https://api.github.com/",
            "https://github.com/OxideMod/Oxide.Rust/releases/latest/download/Oxide.Rust.zip",
            "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip",
            "https://api.steamcmd.net/v1/info/258550",
        ] {
            // Any HTTP answer means the secure connection worked (the mirror doesn't do HEAD).
            match agent.head(url).call() {
                Ok(_) | Err(ureq::Error::Status(..)) => {}
                Err(e) => panic!("{url}: {e}"),
            }
        }
    }
}
