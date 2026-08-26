//! mDNS/Avahi service advertisement for local network discovery.

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use tracing::{error, info};

pub struct DiscoveryHandle {
    daemon: ServiceDaemon,
    fullname: String,
}

/// Register the HyperLink service over mDNS so clients can locate the host on the LAN.
pub fn start_advertisement(device_name: &str, port: u16) -> anyhow::Result<DiscoveryHandle> {
    let daemon = ServiceDaemon::new()?;

    // IPv6-only addresses resolved here are almost always link-local (fe80::...),
    // which need a scope/zone ID to actually be routable — something Android's
    // NsdManager doesn't surface through its resolved-address string, and Rust's
    // SocketAddr parser has no syntax for anyway. Confirmed on a real device:
    // the client would resolve a link-local IPv6 address, "connect" would hang
    // indefinitely with no error (not even a timeout surfaced), because the OS
    // has no way to know which interface to send on. Advertising IPv4-only
    // sidesteps the whole problem for what's fundamentally a same-subnet LAN
    // pairing tool, not something that needs to work over IPv6.
    daemon.disable_interface(IfKind::IPv6)?;

    // Service type for HyperLink: _hyperlink._udp.local.
    let service_type = "_hyperlink._udp.local.".to_string();
    let instance_name = format!("{}-host", device_name);
    let host_name = "hyperlink-host.local.".to_string();
    let fullname = format!("{}.{}", instance_name, service_type);

    let mut properties = HashMap::new();
    properties.insert("device_name".to_string(), device_name.to_string());
    properties.insert("version".to_string(), "1".to_string());

    info!(
        instance_name = %instance_name,
        service_type = %service_type,
        port,
        "registering mDNS service advertisement"
    );

    // Passing "" for the address here does NOT auto-resolve local IPs (a wrong
    // assumption in the original comment) — ServiceInfo::new always sets
    // addr_auto: false internally, so with no explicit address the daemon has
    // nothing to announce on any interface: `prepare_announce` logs "no valid
    // addrs" for every single interface, including the real one, and the
    // service is silently unadvertisable. Confirmed by actually running the
    // daemon and browsing for it with an independent mDNS client (Python's
    // zeroconf) on the same machine — it found nothing. `.enable_addr_auto()`
    // is the real opt-in for automatic local-address resolution.
    let service_info = ServiceInfo::new(
        &service_type,
        &instance_name,
        &host_name,
        "",
        port,
        properties,
    )?
    .enable_addr_auto();

    daemon.register(service_info)?;

    Ok(DiscoveryHandle { daemon, fullname })
}

impl Drop for DiscoveryHandle {
    fn drop(&mut self) {
        info!("unregistering mDNS service advertisement");
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            error!("failed to unregister mDNS service: {}", e);
        }
    }
}
