//! [ArceOS](https://github.com/rcore-os/arceos) network module.
//!
//! It provides unified networking primitives for TCP/UDP communication
//! using various underlying network stacks. Currently, only [smoltcp] is
//! supported.

#![no_std]

#[macro_use]
extern crate log;
extern crate alloc;
#[cfg(test)]
extern crate std;

mod consts;
mod device;
mod general;
mod listen_table;
/// Per-namespace network stack.
pub mod net_stack;
/// Socket option types and the [`Configurable`](options::Configurable) trait.
pub mod options;
/// Raw socket implementation.
pub mod raw;
mod router;
mod service;
mod socket;
pub(crate) mod state;
/// TCP socket implementation.
pub mod tcp;
/// UDP socket implementation.
pub mod udp;
/// Unix domain socket implementation.
pub mod unix;
/// Vsock socket implementation.
#[cfg(feature = "vsock")]
pub mod vsock;
mod wrapper;

use alloc::{borrow::ToOwned, boxed::Box, vec::Vec};
use core::time::Duration;

use ax_sync::Mutex;
use smoltcp::wire::{EthernetAddress, Ipv4Address, Ipv4Cidr};

#[cfg(feature = "vsock")]
pub use self::device::{VsockDevice, VsockDeviceList};
use self::{
    consts::{GATEWAY, IP, IP_PREFIX},
    device::{EthernetDevice, LoopbackDevice},
    net_stack::ROOT_NET_STACK,
    router::{Router, Rule},
    service::Service,
};
pub use self::{
    device::{
        ArpEntry, EthernetDeviceList, EthernetDriver, NetDeviceError, NetDeviceResult,
        NetIrqEvents, NetRxBuffer, NetTxBuffer, RdNetDriver,
    },
    net_stack::NetStack,
    socket::*,
};

const DHCP_BOOTSTRAP_ATTEMPTS: usize = 200;
const DHCP_BOOTSTRAP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Initializes the network subsystem by NIC devices.
pub fn init_network(mut net_devs: EthernetDeviceList) {
    info!("Initialize network subsystem...");

    let mut router = Router::new();
    let lo_dev = router.add_device(Box::new(LoopbackDevice::new()));

    let lo_ip = Ipv4Cidr::new(Ipv4Address::new(127, 0, 0, 1), 8);
    router.add_rule(Rule::new(
        lo_ip.into(),
        None,
        lo_dev,
        lo_ip.address().into(),
    ));

    let static_network = !IP.is_empty() && !GATEWAY.is_empty();
    let mut dhcp_dev = None;
    let mut dhcp_mac = None;

    let eth0_ip = if !net_devs.is_empty() {
        let dev = net_devs.remove(0);
        info!("  use NIC 0: {:?}", dev.device_name());

        let eth0_address = EthernetAddress(dev.mac_address());
        let eth0_ip = static_network
            .then(|| Ipv4Cidr::new(IP.parse().expect("Invalid IPv4 address"), IP_PREFIX));

        let eth0_dev = router.add_device(Box::new(EthernetDevice::new(
            "eth0".to_owned(),
            dev,
            eth0_ip,
        )));

        info!("eth0:");
        info!("  mac:  {}", eth0_address);
        if let Some(eth0_ip) = eth0_ip {
            let gateway = GATEWAY.parse().expect("Invalid gateway address");
            router.add_rule(Rule::new(
                Ipv4Cidr::new(Ipv4Address::UNSPECIFIED, 0).into(),
                Some(gateway),
                eth0_dev,
                eth0_ip.address().into(),
            ));
            info!("  mode: static");
            info!("  ip:   {}", eth0_ip);
            info!("  gw:   {}", gateway);
        } else {
            dhcp_dev = Some(eth0_dev);
            dhcp_mac = Some(eth0_address);
            info!("  mode: dhcp");
        }

        eth0_ip
    } else {
        warn!("  No network device found!");
        None
    };

    for dev in &router.devices {
        info!("Device: {}", dev.name());
    }

    let mut service = Service::new(router);
    service.iface.update_ip_addrs(|ip_addrs| {
        ip_addrs.push(lo_ip.into()).unwrap();
        if let Some(eth0_ip) = eth0_ip {
            ip_addrs.push(eth0_ip.into()).unwrap();
        }
    });
    if let (Some(dhcp_dev), Some(dhcp_mac)) = (dhcp_dev, dhcp_mac) {
        service.enable_dhcp(dhcp_dev, dhcp_mac);
    }
    let dhcp_enabled = service.dhcp_enabled();
    ROOT_NET_STACK.service.call_once(|| Mutex::new(service));
    if dhcp_enabled {
        ax_task::spawn_with_name(dhcp_bootstrap, "dhcp-bootstrap".to_owned());
    }
}

/// Init vsock subsystem by vsock devices.
#[cfg(feature = "vsock")]
pub fn init_vsock(mut vsock_devs: device::VsockDeviceList) {
    use self::device::register_vsock_device;
    info!("Initialize vsock subsystem...");
    if let Some(dev) = vsock_devs.pop() {
        info!("  use vsock 0: {:?}", dev.name());
        if let Err(e) = register_vsock_device(dev) {
            warn!("Failed to initialize vsock device: {:?}", e);
        }
    } else {
        warn!("  No vsock device found!");
    }
}

/// Poll all network interfaces for new events (root namespace only).
pub fn poll_interfaces() {
    ROOT_NET_STACK.poll_interfaces();
}

pub fn arp_entries() -> Vec<ArpEntry> {
    ROOT_NET_STACK.get_service().arp_entries()
}

fn dhcp_bootstrap() {
    for _ in 0..DHCP_BOOTSTRAP_ATTEMPTS {
        poll_interfaces();
        if ROOT_NET_STACK.get_service().dhcp_configured() {
            return;
        }
        ax_task::sleep(DHCP_BOOTSTRAP_POLL_INTERVAL);
    }
    warn!("eth0: DHCP bootstrap timed out");
}

#[cfg(test)]
pub(crate) mod test_support {
    use alloc::{boxed::Box, sync::Arc};
    use std::sync::{Mutex as StdMutex, MutexGuard, Once};

    use ax_sync::Mutex;
    use smoltcp::wire::{IpAddress, Ipv4Address, Ipv4Cidr};

    use crate::{
        NetStack,
        device::LoopbackDevice,
        router::{Router, Rule},
        service::Service,
    };

    pub(crate) const LOCAL_MASK: u32 = 1 << 0;
    pub(crate) const PEER_MASK: u32 = 1 << 1;
    pub(crate) const LOCAL_ADDR: Ipv4Address = Ipv4Address::new(192, 0, 2, 10);
    pub(crate) const PEER_ADDR: Ipv4Address = Ipv4Address::new(198, 51, 100, 20);

    static NETWORK_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    pub(crate) fn network_test_guard() -> MutexGuard<'static, ()> {
        NETWORK_TEST_LOCK.lock().unwrap()
    }

    pub(crate) fn init_split_route_network() -> Arc<NetStack> {
        let stack = Arc::new(NetStack {
            listen_table: crate::listen_table::ListenTable::new(),
            socket_set: crate::wrapper::SocketSetWrapper::new(),
            service: spin::Once::new(),
            polling: core::sync::atomic::AtomicBool::new(false),
            poll_again: core::sync::atomic::AtomicBool::new(false),
            udp_ephemeral_port: Mutex::new(0xc000),
            tcp_ephemeral_port: Mutex::new(0xc000),
            tcp_bound_ports: Mutex::new(hashbrown::HashMap::new()),
        });

        let mut router = Router::new();
        let local_dev = router.add_device(Box::new(LoopbackDevice::new()));
        let peer_dev = router.add_device(Box::new(LoopbackDevice::new()));
        let local_cidr = Ipv4Cidr::new(LOCAL_ADDR, 24);
        let peer_cidr = Ipv4Cidr::new(PEER_ADDR, 24);

        router.add_rule(Rule::new(
            local_cidr.into(),
            None,
            local_dev,
            IpAddress::Ipv4(LOCAL_ADDR),
        ));
        router.add_rule(Rule::new(
            peer_cidr.into(),
            None,
            peer_dev,
            IpAddress::Ipv4(PEER_ADDR),
        ));

        let mut service = Service::new(router);
        service.iface.update_ip_addrs(|ip_addrs| {
            ip_addrs.push(local_cidr.into()).unwrap();
            ip_addrs.push(peer_cidr.into()).unwrap();
        });
        stack.service.call_once(|| Mutex::new(service));
        stack
    }
}
