use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicBool, Ordering};

use ax_sync::Mutex;
use smoltcp::wire::{Ipv4Address, Ipv4Cidr};
use spin::{LazyLock, Once};

use crate::{
    device::LoopbackDevice,
    listen_table::ListenTable,
    router::{Router, Rule},
    service::Service,
    wrapper::SocketSetWrapper,
};

/// A self-contained per-namespace network stack.
///
/// Root namespace: [`ROOT_NET_STACK`] (initialized by `init_network`).
/// New namespaces: [`NetStack::new_loopback`] (loopback-only, isolated).
pub struct NetStack {
    pub listen_table: ListenTable,
    pub socket_set: SocketSetWrapper<'static>,
    pub service: Once<Mutex<Service>>,
    pub polling: AtomicBool,
    pub poll_again: AtomicBool,
    /// Per-stack ephemeral port counter for UDP.
    pub udp_ephemeral_port: Mutex<u16>,
    /// Per-stack ephemeral port counter for TCP.
    pub tcp_ephemeral_port: Mutex<u16>,
    /// Per-stack TCP bound-port conflict table.
    pub tcp_bound_ports:
        Mutex<hashbrown::HashMap<u16, alloc::vec::Vec<Option<smoltcp::wire::IpAddress>>>>,
}

// Safety: all fields are Send+Sync.
unsafe impl Sync for NetStack {}
unsafe impl Send for NetStack {}

const EPHEMERAL_PORT_START: u16 = 0xc000;

impl NetStack {
    /// Create a loopback-only stack for a new (non-root) network namespace.
    pub fn new_loopback() -> Arc<Self> {
        let stack = Arc::new(NetStack {
            listen_table: ListenTable::new(),
            socket_set: SocketSetWrapper::new(),
            service: Once::new(),
            polling: AtomicBool::new(false),
            poll_again: AtomicBool::new(false),
            udp_ephemeral_port: Mutex::new(EPHEMERAL_PORT_START),
            tcp_ephemeral_port: Mutex::new(EPHEMERAL_PORT_START),
            tcp_bound_ports: Mutex::new(hashbrown::HashMap::new()),
        });

        let mut router = Router::new();
        let lo_dev = router.add_device(Box::new(LoopbackDevice::new()));
        let lo_ip = Ipv4Cidr::new(Ipv4Address::new(127, 0, 0, 1), 8);
        router.add_rule(Rule::new(
            lo_ip.into(),
            None,
            lo_dev,
            lo_ip.address().into(),
        ));
        let mut service = Service::new(router);
        service.iface.update_ip_addrs(|addrs| {
            addrs.push(lo_ip.into()).unwrap();
        });
        stack.service.call_once(|| Mutex::new(service));
        stack
    }

    pub fn get_service(&self) -> ax_sync::MutexGuard<'_, Service> {
        self.service
            .get()
            .expect("network service not initialized")
            .lock()
    }

    pub fn poll_interfaces(&self) {
        self.poll_again.store(true, Ordering::Release);
        loop {
            if self
                .polling
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Acquire)
                .is_err()
            {
                return;
            }
            while self.poll_again.swap(false, Ordering::AcqRel) {
                while self
                    .get_service()
                    .poll(&self.socket_set, &self.listen_table)
                {}
            }
            self.polling.store(false, Ordering::Release);
            if !self.poll_again.load(Ordering::Acquire) {
                return;
            }
        }
    }
}

/// Root network namespace stack. Initialized by `init_network`.
pub static ROOT_NET_STACK: LazyLock<Arc<NetStack>> = LazyLock::new(|| {
    Arc::new(NetStack {
        listen_table: ListenTable::new(),
        socket_set: SocketSetWrapper::new(),
        service: Once::new(),
        polling: AtomicBool::new(false),
        poll_again: AtomicBool::new(false),
        udp_ephemeral_port: Mutex::new(EPHEMERAL_PORT_START),
        tcp_ephemeral_port: Mutex::new(EPHEMERAL_PORT_START),
        tcp_bound_ports: Mutex::new(hashbrown::HashMap::new()),
    })
});
