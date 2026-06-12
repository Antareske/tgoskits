use alloc::sync::Arc;
use core::sync::atomic::AtomicU64;

use ax_kspin::SpinNoIrq;
use axnet::NetStack;

static NEXT_NET_NS_ID: AtomicU64 = AtomicU64::new(0);

/// The initial root network namespace, shared by all processes until
/// they call `unshare(CLONE_NEWNET)` or `clone(CLONE_NEWNET)`.
pub static ROOT_NET_NS: spin::LazyLock<Arc<SpinNoIrq<NetNamespace>>> = spin::LazyLock::new(|| {
    Arc::new(SpinNoIrq::new(NetNamespace {
        ns_id: NEXT_NET_NS_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
        stack: axnet::net_stack::ROOT_NET_STACK.clone(),
    }))
});

/// Per-process network namespace.
///
/// Isolates network interfaces, routing tables, and sockets so that
/// processes in different namespaces see independent network stacks.
pub struct NetNamespace {
    pub ns_id: u64,
    /// The network stack backing this namespace.
    pub stack: Arc<NetStack>,
}

impl NetNamespace {
    pub fn new_root() -> Self {
        Self {
            ns_id: NEXT_NET_NS_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            stack: axnet::net_stack::ROOT_NET_STACK.clone(),
        }
    }

    pub fn clone_ns(&self) -> Self {
        Self {
            ns_id: NEXT_NET_NS_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            stack: NetStack::new_loopback(),
        }
    }
}
