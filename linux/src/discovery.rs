//! mDNS/Avahi service advertisement for local network discovery.

use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use tracing::{error, info};

pub struct DiscoveryHandle {
    daemon: ServiceDaemon,
    fullname: String,
}

/// Register the HyperLink service over mDNS so clients can locate the host on the LAN.
pub fn start_advertisement(device_name: &str, port: u16) -> anyhow::Result<DiscoveryHandle> {
    let daemon = ServiceDaemon::new()?;

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

    let service_info = ServiceInfo::new(
        &service_type,
        &instance_name,
        &host_name,
        "", // Auto-resolves local IPs
        port,
        properties,
    )?;

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
